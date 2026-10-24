#!/usr/bin/env python3
"""Native remote-MCP sign-in with the real clients installed on this machine.

Run it against the disposable stack that the Keycloak harness leaves running:

    python3 scripts/keycloak-acceptance.py --stack-only /tmp/wk-stack.json &
    python3 scripts/mcp-client-oauth.py --stack /tmp/wk-stack.json --output /tmp/clients.json

For each client it checks, with isolated profiles only (a throwaway config directory or HOME; the
real ones are never read or written): the client registers itself, the user signs in and consents,
the token carries exactly the server's MCP resource audience and a refresh token, the client can
list the tools (and, for Claude Code, call one through a scripted stand-in model), and the client
renews an expired access token without asking the user again. The realm's access-token lifetime
is shortened to a few seconds for this, so the stack is for testing only.

Clients: Claude Code (`claude`), the Cursor agent CLI (`agent`, which `cursor agent` runs) and the
VS Code desktop app. VS Code runs in a throwaway user-data directory under /tmp (its sockets need a short
path) with its debug ports open: the harness hooks `shell.openExternal` in its main process to capture the
sign-in URL instead of opening a browser, and presses its command palette keys to restart the MCP server.
It needs no GUI interaction, but it does open a VS Code window for a few minutes. The user's own VS Code
settings, MCP config and argv.json are hashed before and after to prove they were not touched. The Cursor
editor is a GUI program and is not driven here.

The Claude Code check bypasses its first-run screens by writing its (undocumented) config file
and uses a fake API key, so it can break with a new Claude Code version; a failure there means
"re-check by hand", not necessarily "the client is broken". Prints a pass/fail line per check.
"""
import argparse, base64, hashlib, html, http.cookiejar, http.server, json, os, pty, re, select, shutil, signal
import socket, ssl, struct, fcntl, subprocess, sys, tempfile, termios, threading, time, urllib.error, urllib.parse, urllib.request
from pathlib import Path

LIFESPAN = 12  # seconds; short enough to watch a refresh, long enough to finish a sign-in
# VS Code refreshes any token with less than 300 s left whenever it asks for one, and each refresh re-triggers the ask:
# at 300 s (Keycloak's default) it fires dozens of refreshes after each sign-in, at 40 s they multiply without end (see the review doc).
VSCODE_LIFESPAN = 330
VSCODE = "/Applications/Visual Studio Code.app/Contents/MacOS/Code"


class Stack:
    def __init__(self, path):
        self.info = json.load(open(path))
        self.ctx = ssl.create_default_context(cafile=self.info["ca_file"])
        self.origin = self.info["app_origin"]

    def call(self, method, path, body=None, form=None, token=None):
        data, headers = None, {}
        if form is not None:
            data, headers["Content-Type"] = urllib.parse.urlencode(form).encode(), "application/x-www-form-urlencoded"
        elif body is not None:
            data, headers["Content-Type"] = json.dumps(body).encode(), "application/json"
        if token:
            headers["Authorization"] = "Bearer " + token
        request = urllib.request.Request(self.info["admin_url"] + path, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request, context=self.ctx, timeout=20) as r:
                raw = r.read()
        except urllib.error.HTTPError as e:
            raw = e.read()
        return json.loads(raw) if raw else None

    def admin_token(self):  # the master realm's tokens last 60 s, so fetch one per use
        return self.call("POST", "/realms/master/protocol/openid-connect/token",
                         form={"grant_type": "password", "client_id": "admin-cli", "username": "admin",
                               "password": self.info["admin_password"]})["access_token"]

    def update_realm(self, **changes):
        token = self.admin_token()
        realm = self.call("GET", f"/admin/realms/{self.info['realm']}", token=token)
        self.call("PUT", f"/admin/realms/{self.info['realm']}", dict(realm, **changes), token=token)

    def set_access_lifespan(self, seconds):
        self.update_realm(accessTokenLifespan=seconds)

    def start_recording_events(self):
        """Keycloak keeps no event history unless asked; the sign-in and refresh events are the proof of what the client did."""
        self.update_realm(eventsEnabled=True, eventsExpiration=3600, enabledEventTypes=[
            "LOGIN", "LOGIN_ERROR", "CODE_TO_TOKEN", "CODE_TO_TOKEN_ERROR", "REFRESH_TOKEN", "REFRESH_TOKEN_ERROR", "CLIENT_REGISTER"])

    def event_counts(self):
        events = self.call("GET", f"/admin/realms/{self.info['realm']}/events?max=5000", token=self.admin_token()) or []
        counts = {}
        for event in events:
            counts[event["type"]] = counts.get(event["type"], 0) + 1
        return counts

    def server_traffic(self):
        """What the HTTPS proxy in front of the server saw: (method, path, status, carried a bearer token)."""
        logs = subprocess.run(["docker", "logs", self.info["proxy_container"]], capture_output=True, text=True)
        rows = []
        for line in (logs.stdout + logs.stderr).splitlines():
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            if entry.get("logger") == "http.log.access" and entry["request"]["uri"] != "/health/ready":
                request = entry["request"]
                rows.append((request["method"], request["uri"], entry["status"], "Authorization" in request["headers"]))
        return rows

    def browser(self, authorization_url):
        """Play the user's browser: sign in as the synthetic user, accept consent, follow to the loopback callback."""

        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *args):
                return None

        opener = urllib.request.build_opener(NoRedirect(), urllib.request.HTTPSHandler(context=self.ctx),
                                             urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

        def fetch(url, form=None):
            try:
                r = opener.open(urllib.request.Request(url, data=urllib.parse.urlencode(form).encode() if form else None), timeout=20)
            except urllib.error.HTTPError as e:
                r = e
            return r.code, r.headers, r.read().decode()

        _, _, page = fetch(authorization_url)
        action = re.search(r'<form[^>]*id="kc-form-login"[^>]*action="([^"]+)"', page)
        if not action:
            raise RuntimeError("Keycloak login form not found")
        _, headers, _ = fetch(html.unescape(action.group(1)), {"username": self.info["username"], "password": self.info["password"], "credentialId": ""})
        location = headers.get("Location", "")
        while "required-action" in location:  # consent to a client that registered itself
            _, _, page = fetch(location)
            form = re.search(r'<form[^>]*action="([^"]+)"', page)
            fields = {k: v for k, v in re.findall(r'<input[^>]*name="([^"]+)"[^>]*value="([^"]*)"', page) if k == "code"}
            _, headers, _ = fetch(urllib.parse.urljoin(self.info["admin_url"], html.unescape(form.group(1))), dict(fields, accept="Yes"))
            location = headers.get("Location", "")
        if not location.startswith(("http://localhost", "http://127.0.0.1")):
            raise RuntimeError("unexpected redirect after sign-in: " + location[:60])
        urllib.request.build_opener(urllib.request.ProxyHandler({})).open(location, timeout=20).read()


class Ledger:
    def __init__(self):
        self.rows = []

    def check(self, name, ok, detail=""):
        self.rows.append({"check": name, "passed": bool(ok), "detail": "" if ok else detail})
        print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else ""), file=sys.stderr)

    def note(self, text):
        """A finding about the client that is not a requirement on this project."""
        self.rows.append({"note": text})
        print("NOTE " + text, file=sys.stderr)


def claims(token):
    payload = token.split(".")[1]
    return json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))


class FakeModel(http.server.ThreadingHTTPServer):
    """Stands in for the Anthropic API: asks for one knowledge.search call, then finishes."""

    def __init__(self):
        outer = self
        self.requests = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
                outer.requests.append((self.path, body))
                if not self.path.startswith("/v1/messages") or "count_tokens" in self.path:
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.end_headers()
                    self.wfile.write(b'{"input_tokens":10}')
                    return
                last = body.get("messages", [{}])[-1].get("content")
                answered = isinstance(last, list) and any(c.get("type") == "tool_result" for c in last)
                tool = next((t["name"] for t in body.get("tools", []) if t["name"].startswith("mcp__wk__") and t["name"].endswith("knowledge_search")), None)
                if answered or not tool:
                    block, stop = {"type": "text", "text": "search finished"}, "end_turn"
                else:
                    block, stop = {"type": "tool_use", "id": "toolu_01fake", "name": tool, "input": {"query": "middleware", "limit": 3}}, "tool_use"
                message = {"id": "msg_fake", "type": "message", "role": "assistant", "model": body.get("model", "fake"), "content": [],
                           "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 5}}
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()

                def event(name, data):
                    self.wfile.write(f"event: {name}\ndata: {json.dumps(data)}\n\n".encode())

                event("message_start", {"type": "message_start", "message": message})
                if block["type"] == "text":
                    event("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})
                    event("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": block["text"]}})
                else:
                    event("content_block_start", {"type": "content_block_start", "index": 0, "content_block": dict(block, input={})})
                    event("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": json.dumps(block["input"])}})
                event("content_block_stop", {"type": "content_block_stop", "index": 0})
                event("message_delta", {"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": None}, "usage": {"output_tokens": 5}})
                event("message_stop", {"type": "message_stop"})

        super().__init__(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.serve_forever, daemon=True).start()

    def tool_results(self):
        found = []
        for _, body in self.requests:
            content = (body.get("messages") or [{}])[-1].get("content")
            if isinstance(content, list):
                found += [json.dumps(c.get("content")) for c in content if c.get("type") == "tool_result"]
        return found


class Pty:
    def __init__(self, argv, env, cwd):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(cwd)
            os.execve(argv[0], argv, env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 200, 0, 0))
        self.raw = b""

    def read(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            if select.select([self.fd], [], [], 0.3)[0]:
                try:
                    chunk = os.read(self.fd, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                self.raw += chunk

    def send(self, text):
        os.write(self.fd, text.encode())

    def screen(self):
        text = self.raw.decode("utf8", "replace")
        text = re.sub(r"\x1b\[[0-9;?]*[ -/]*[@-~]", "", text)
        return re.sub(r"\x1b\][^\x07]*\x07|\x1b[()][A-Za-z0-9]", "", text)

    def kill(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        except OSError:
            pass


def claude_code(stack, binary, ledger):
    version = subprocess.run([binary, "--version"], capture_output=True, text=True).stdout.strip()
    config, work = tempfile.mkdtemp(prefix="wk-cc-"), tempfile.mkdtemp(prefix="wk-cc-work-")
    key = "sk-ant-api03-" + "x" * 80
    model = FakeModel()
    env = {"HOME": os.environ["HOME"], "PATH": os.environ["PATH"], "TERM": "xterm-256color", "CLAUDE_CONFIG_DIR": config,
           "NODE_EXTRA_CA_CERTS": stack.info["ca_file"], "ANTHROPIC_API_KEY": key,
           "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{model.server_port}", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"}
    run = lambda *args, timeout=90: subprocess.run([binary, *args], env=env, cwd=work, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)
    terminal = None
    try:
        add = run("mcp", "add", "--transport", "http", "wk", stack.origin + "/mcp/demo")
        config_file = Path(config) / ".claude.json"
        document = json.loads(config_file.read_text())
        document.update({"hasCompletedOnboarding": True, "theme": "dark", "numStartups": 5, "customApiKeyResponses": {"approved": [key[-20:]], "rejected": []}})
        for project in document["projects"].values():
            project.update(hasTrustDialogAccepted=True, hasCompletedProjectOnboarding=True)
        config_file.write_text(json.dumps(document))
        listed = run("mcp", "list")
        ledger.check(f"claude {version}: registers itself and reports that it needs authentication", add.returncode == 0 and "Needs authentication" in listed.stdout, listed.stdout[-200:] + listed.stderr[-200:])

        terminal = Pty([binary], env, work)
        terminal.read(8)
        for keys in ("/mcp\r", "\r", "\r"):
            terminal.send(keys)
            terminal.read(3)
        terminal.raw = b""
        terminal.send("\r")
        terminal.read(6)
        match = re.search(r"(https://localhost:\d+/realms/[^/]+/protocol/openid-connect/auth\?\S*(?:\s+\S+)*?)\s*Return here", terminal.screen())
        url = re.sub(r"\s+", "", match.group(1)) if match else ""
        query = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)
        ledger.check(f"claude {version}: authorization request uses PKCE S256, a loopback redirect and the MCP resource",
                     query.get("code_challenge_method") == ["S256"] and query.get("redirect_uri", [""])[0].startswith("http://localhost:")
                     and bool(query.get("resource")), str(query)[:200])
        if not url:
            return
        stack.browser(url)
        terminal.raw = b""
        terminal.read(8)
        ledger.check(f"claude {version}: sign-in completes and the server connects", re.search(r"Authentication\s*successful", terminal.screen().replace(" ", "")) is not None
                     or "Connected" in terminal.screen(), terminal.screen()[-200:])
        terminal.kill()
        terminal = None

        credentials = json.loads((Path(config) / ".credentials.json").read_text())["mcpOAuth"]
        entry = next(iter(credentials.values()))
        token_claims = claims(entry["accessToken"])
        ledger.check(f"claude {version}: token has exactly the MCP resource audience, and a refresh token is kept",
                     token_claims.get("aud") == stack.origin and bool(entry.get("refreshToken")), str(token_claims.get("aud")))
        before = entry["expiresAt"]
        reply = run("-p", "search the knowledge base for middleware", "--allowedTools", "mcp__wk__knowledge_search", "--output-format", "json")
        results = model.tool_results()
        ledger.check(f"claude {version}: calls knowledge.search through the signed-in connection (scripted model)",
                     reply.returncode == 0 and any('\\"cards\\"' in r for r in results), (reply.stdout + reply.stderr)[-300:] + str(results)[:200])

        time.sleep(LIFESPAN + 3)
        model.requests.clear()
        again = run("-p", "search the knowledge base for middleware", "--allowedTools", "mcp__wk__knowledge_search", "--output-format", "json")
        renewed = next(iter(json.loads((Path(config) / ".credentials.json").read_text())["mcpOAuth"].values()))["expiresAt"]
        ledger.check(f"claude {version}: renews an expired access token without asking the user",
                     again.returncode == 0 and renewed > before and any('\\"cards\\"' in r for r in model.tool_results()), (again.stdout + again.stderr)[-300:])
    finally:
        if terminal:
            terminal.kill()
        model.shutdown()
        subprocess.run(["/usr/bin/security", "delete-generic-password", "-s", "Claude Code-credentials-" + hashlib.sha256(config.encode()).hexdigest()[:8]],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        shutil.rmtree(config, ignore_errors=True)
        shutil.rmtree(work, ignore_errors=True)


def cursor_agent(stack, binary, ledger):
    version = subprocess.run([binary, "--version"], capture_output=True, text=True).stdout.strip()
    home, work = tempfile.mkdtemp(prefix="wk-cursor-home-"), os.path.realpath(tempfile.mkdtemp(prefix="wk-cursor-work-"))
    env = {"HOME": home, "PATH": os.environ["PATH"], "NODE_EXTRA_CA_CERTS": stack.info["ca_file"], "NO_OPEN_BROWSER": "1", "TERM": "xterm"}
    run = lambda *args, timeout=90: subprocess.run([binary, *args], env=env, cwd=work, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)
    login = None
    try:
        (Path(work) / ".cursor").mkdir()
        (Path(work) / ".cursor/mcp.json").write_text(json.dumps({"mcpServers": {"wk": {"url": stack.origin + "/mcp/demo"}}}))
        run("mcp", "enable", "wk")
        login = subprocess.Popen([binary, "mcp", "login", "wk"], env=env, cwd=work, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        lines, url = [], ""
        for line in login.stdout:
            lines.append(line)
            found = re.search(r"https://localhost:\d+/realms/\S+/protocol/openid-connect/auth\S*", line)
            if found:
                url = found.group(0)
                break
            if "failed" in line.lower():
                break
        query = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)
        ledger.check(f"cursor agent {version}: registers itself and starts PKCE S256 sign-in with a loopback redirect and the MCP resource",
                     query.get("code_challenge_method") == ["S256"] and query.get("redirect_uri", [""])[0].startswith("http://localhost:")
                     and bool(query.get("resource")), "".join(lines)[-300:])
        if not url:
            return
        stack.browser(url)
        rest = login.communicate(timeout=60)[0]
        ledger.check(f"cursor agent {version}: sign-in completes", "login successful" in rest, rest[-200:])
        listed = run("mcp", "list-tools", "wk")
        ledger.check(f"cursor agent {version}: lists the 11 server tools with the new token", "Tools for wk (11)" in listed.stdout, listed.stdout[-200:] + listed.stderr[-200:])
        auth_file = next(Path(home).glob(".cursor/projects/*/mcp-auth.json"))
        entry = json.loads(auth_file.read_text())["wk"]
        token_claims = claims(entry["tokens"]["access_token"])
        ledger.check(f"cursor agent {version}: token has exactly the MCP resource audience, and a refresh token is kept",
                     token_claims.get("aud") == stack.origin and bool(entry["tokens"].get("refresh_token")), str(token_claims.get("aud")))
        before = token_claims["iat"]
        time.sleep(LIFESPAN + 3)
        again = run("mcp", "list-tools", "wk")
        after = claims(json.loads(auth_file.read_text())["wk"]["tokens"]["access_token"])["iat"]
        ledger.check(f"cursor agent {version}: renews an expired access token without asking the user",
                     "Tools for wk (11)" in again.stdout and after > before, again.stdout[-200:] + again.stderr[-200:])
        mode = auth_file.stat().st_mode & 0o777
        ledger.note(f"cursor agent {version}: its credential file (mcp-auth.json, holding the tokens) has mode {oct(mode)}"
                    + ("" if mode & 0o077 else " (private)"))
    finally:
        if login and login.poll() is None:
            login.kill()
        shutil.rmtree(home, ignore_errors=True)
        shutil.rmtree(work, ignore_errors=True)


class Cdp:
    """Just enough of the Chrome DevTools Protocol, over a bare WebSocket, to drive a debuggable Electron app."""

    def __init__(self, ws_url):
        parts = urllib.parse.urlsplit(ws_url)
        self.sock = socket.create_connection((parts.hostname, parts.port), timeout=60)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f"GET {parts.path} HTTP/1.1\r\nHost: {parts.hostname}:{parts.port}\r\nUpgrade: websocket\r\n"
                           f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
        head = b""
        while not head.endswith(b"\r\n\r\n"):  # one byte at a time, so no frame bytes are consumed with the headers
            head += self.sock.recv(1)
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise RuntimeError("WebSocket upgrade refused: " + head[:80].decode("latin1"))
        self.last_id = 0

    def _read(self, count):
        data = b""
        while len(data) < count:
            chunk = self.sock.recv(count - len(data))
            if not chunk:
                raise EOFError("debug connection closed")
            data += chunk
        return data

    def _message(self):
        message = b""
        while True:
            first, second = self._read(2)
            length = second & 127
            if length == 126:
                length = struct.unpack(">H", self._read(2))[0]
            elif length == 127:
                length = struct.unpack(">Q", self._read(8))[0]
            payload = self._read(length)  # frames from the server are never masked
            if first & 15 == 8:
                raise EOFError("debug connection closed")
            if first & 15 not in (9, 10):  # ping and pong carry nothing for us
                message += payload
                if first & 128:
                    return message

    def call(self, method, **params):
        self.last_id += 1
        data = json.dumps({"id": self.last_id, "method": method, "params": params}).encode()
        mask = os.urandom(4)
        size = len(data)
        header = bytes([0x81]) + (bytes([0x80 | size]) if size < 126 else bytes([0x80 | 126]) + struct.pack(">H", size) if size < 65536
                                  else bytes([0x80 | 127]) + struct.pack(">Q", size))
        self.sock.sendall(header + mask + bytes(byte ^ mask[i % 4] for i, byte in enumerate(data)))
        while True:
            reply = json.loads(self._message())
            if reply.get("id") == self.last_id:
                if "error" in reply:
                    raise RuntimeError(f"{method}: {reply['error']}")
                return reply["result"]

    def press(self, name, modifiers=0):
        codes = {"Enter": 13, "ArrowDown": 40}
        key = dict(key=name, code=name if len(name) > 1 else "Key" + name.upper(), windowsVirtualKeyCode=codes.get(name) or ord(name.upper()), modifiers=modifiers)
        self.call("Input.dispatchKeyEvent", type="rawKeyDown", **key)
        self.call("Input.dispatchKeyEvent", type="keyUp", **key)

    def close(self):
        self.sock.close()


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def debug_target(port, kind, timeout=60):
    end = time.time() + timeout
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    while time.time() < end:
        try:
            targets = json.load(opener.open(f"http://127.0.0.1:{port}/json", timeout=3))
            found = [t["webSocketDebuggerUrl"] for t in targets if kind is None or t.get("type") == kind]
            if found:
                return found[0]
        except (OSError, ValueError):
            pass
        time.sleep(0.5)
    raise RuntimeError(f"no debug target on port {port}")


def vscode(stack, binary, ledger, lifespan):
    """The VS Code desktop app, in a throwaway profile, signing in to the hosted MCP through its own remote OAuth."""
    version = json.loads((Path(binary).parents[1] / "Resources/app/package.json").read_text())["version"]
    name = f"vscode {version}"
    own = [Path.home() / "Library/Application Support/Code/User/settings.json", Path.home() / "Library/Application Support/Code/User/mcp.json",
           Path.home() / ".vscode/argv.json"]
    digest = lambda: [hashlib.sha256(f.read_bytes()).hexdigest() if f.exists() else None for f in own]
    before = digest()
    root = Path(tempfile.mkdtemp(prefix="wkvsc-", dir="/tmp"))  # a short path: its IPC socket path must stay under ~100 characters
    (root / "data/User").mkdir(parents=True)
    (root / "work/.vscode").mkdir(parents=True)
    # The profile's CA setting: VS Code sets NODE_USE_SYSTEM_CA for its extension host, which makes NODE_EXTRA_CA_CERTS inert unless the
    # system-certificate options are off. A real team puts the CA in the OS trust store instead.
    (root / "data/User/settings.json").write_text(json.dumps({
        "http.systemCertificatesNode": False, "http.systemCertificates": False, "window.dialogStyle": "custom",
        # no telemetry setting here: VS Code copies it into the shared ~/.vscode/argv.json (crash reporter), which is the user's
        "update.mode": "none", "workbench.startupEditor": "none", "extensions.autoUpdate": False}))
    opened = root / "opened-urls.txt"
    ports = free_port(), free_port()
    env = {k: v for k, v in os.environ.items() if not k.startswith("VSCODE_") and k != "ELECTRON_RUN_AS_NODE"}
    env["NODE_EXTRA_CA_CERTS"] = stack.info["ca_file"]
    app = subprocess.Popen([binary, "--user-data-dir", str(root / "data"), "--extensions-dir", str(root / "ext"), f"--remote-debugging-port={ports[0]}",
                            f"--inspect={ports[1]}", "--disable-extensions", "--disable-updates", "--skip-welcome", "--skip-release-notes",
                            "--disable-workspace-trust", "-n", str(root / "work")], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           start_new_session=True)
    main = page = None
    try:
        main = Cdp(debug_target(ports[1], None))
        # The sign-in URL would open in the user's default browser; capture it instead, before any MCP server is configured.
        hook = """(() => { const load = process.mainModule ? process.mainModule.require : require; const {shell} = load('electron'); const fs = load('fs');
                  shell.openExternal = async (url) => { fs.appendFileSync(%s, url + '\\n'); }; return 'hooked'; })()""" % json.dumps(str(opened))
        hooked = main.call("Runtime.evaluate", expression=hook, returnByValue=True)["result"].get("value")
        page = Cdp(debug_target(ports[0], "page"))
        time.sleep(5)  # let the workbench settle before the server definition appears
        stack.update_realm(accessTokenLifespan=lifespan)
        stack.start_recording_events()
        seen = len(stack.server_traffic())
        base = stack.event_counts()  # Keycloak keeps events from earlier runs against the same stack

        def new_events():
            now = stack.event_counts()
            return {k: now[k] - base.get(k, 0) for k in now if now[k] != base.get(k, 0)}
        (root / "work/.vscode/mcp.json").write_text(json.dumps({"servers": {"wk": {"type": "http", "url": stack.origin + "/mcp/demo"}}}))
        time.sleep(3)

        focused = "(() => { const r = document.querySelector('.quick-input-list .monaco-list-row.focused'); return r ? r.textContent : ''; })()"

        def palette(*labels):  # MCP: List Servers, then pick each label in turn (the server, then its action) by what the focused row says
            page.press("P", 12)  # Cmd+Shift+P
            time.sleep(1)
            page.call("Input.insertText", text="MCP: List Servers")
            time.sleep(1)
            page.press("Enter")
            time.sleep(1.5)
            for label in labels:
                for _ in range(10):
                    if label in page.call("Runtime.evaluate", expression=focused, returnByValue=True)["result"].get("value", ""):
                        break
                    page.press("ArrowDown")
                    time.sleep(0.3)
                else:
                    raise RuntimeError(f"command palette entry {label!r} not found")
                page.press("Enter")
                time.sleep(1.5)

        palette("wk", "Start Server")
        allow = """(() => { const b = [...document.querySelectorAll('a,button,.monaco-button')].find(x => x.textContent.trim() === 'Allow' && x.offsetParent);
                   if (!b) return false; b.click(); return true; })()"""
        end = time.time() + 60
        allowed = False
        while time.time() < end and not (opened.exists() and opened.read_text().strip()):
            if not allowed:  # "The MCP Server Definition 'wk' wants to authenticate to <provider>": a custom dialog, since window.dialogStyle is custom
                allowed = bool(page.call("Runtime.evaluate", expression=allow, returnByValue=True)["result"].get("value"))
            time.sleep(1)
        url = opened.read_text().split()[0] if opened.exists() and opened.read_text().strip() else ""
        query = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)
        scopes = query.get("scope", [""])[0].split()
        ledger.check(f"{name}: registers itself and starts PKCE S256 sign-in with a loopback redirect, the MCP resource and the audience scope",
                     hooked == "hooked" and query.get("code_challenge_method") == ["S256"] and query.get("redirect_uri", [""])[0].startswith("http://127.0.0.1:")
                     and bool(query.get("resource")) and {"openid", "offline_access", "wk-mcp-resource"} <= set(scopes), str(query)[:300])
        if not url:
            return
        ledger.note(f"{name}: its 'wants to authenticate to <provider>' dialog " + ("appeared and was allowed" if allowed else "did not appear in this run"))
        ledger.note(f"{name}: sent resource={query['resource'][0]} and scopes {' '.join(scopes)}")
        stack.browser(url)

        def tools_logged():
            return any("Discovered 11 tools" in f.read_text(errors="replace") for f in (root / "data/logs").rglob("mcpServer*.log"))

        end = time.time() + 60
        while time.time() < end and not tools_logged():
            time.sleep(1)
        signed_in = time.time()
        traffic = [row for row in stack.server_traffic()[seen:] if row[3]]
        ledger.check(f"{name}: sign-in completes and VS Code lists the 11 server tools", tools_logged(), "no 'Discovered 11 tools' in its MCP log")
        ledger.check(f"{name}: the server answered initialize, the initialized notification and tools/list with the new token (200, 202, 200)",
                     [row[2] for row in traffic if row[0] == "POST"][:3] == [200, 202, 200], str(traffic))
        events = new_events()
        ledger.note(f"{name}: Keycloak events after sign-in: {events}")
        ledger.check(f"{name}: Keycloak recorded one login and one code-for-token exchange", events.get("LOGIN") == 1 and events.get("CODE_TO_TOKEN") == 1, str(events))

        time.sleep(max(0, lifespan + 10 - (time.time() - signed_in)))  # the access token has now expired
        mark = len(stack.server_traffic())
        palette("wk", "Restart Server")
        end = time.time() + 90
        after = []
        while time.time() < end:
            after = [row for row in stack.server_traffic()[mark:] if row[3] and row[0] == "POST" and row[2] == 200]
            if len(after) >= 2:
                break
            time.sleep(2)
        events = new_events()
        urls = [u for u in opened.read_text().split() if u.startswith(stack.info["issuer"])]  # VS Code also opens its own local sign-in page for GitHub
        ledger.check(f"{name}: renews an expired access token without asking the user (refresh event, no second login, no second browser URL, server answers 200)",
                     len(after) >= 2 and events.get("REFRESH_TOKEN", 0) >= 1 and events.get("LOGIN") == 1 and len(urls) == 1, f"{events} {urls[1:]} {after}")
        ledger.check(f"{name}: refreshes are bounded (no refresh burst; needs an access-token lifespan above 300 s)", events.get("REFRESH_TOKEN", 0) <= 10,
                     f"{events.get('REFRESH_TOKEN')} refresh events at an access-token lifespan of {lifespan} s")
    finally:
        for client in (page, main):
            if client:
                client.close()
        try:
            os.killpg(app.pid, signal.SIGKILL)
        except OSError:
            pass
        time.sleep(1)
        shutil.rmtree(root, ignore_errors=True)
        ledger.check(f"{name}: the user's own settings, MCP config and argv.json are unchanged, and the throwaway profile is removed",
                     digest() == before and not root.exists())


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--stack", type=Path, required=True, help="info file written by keycloak-acceptance.py --stack-only")
    parser.add_argument("--claude", default=str(Path.home() / ".local/bin/claude"))
    parser.add_argument("--agent", default=str(Path.home() / ".local/bin/agent"))
    parser.add_argument("--vscode", default=VSCODE)
    parser.add_argument("--vscode-lifespan", type=int, default=VSCODE_LIFESPAN, help="access-token lifespan in seconds for the VS Code run (default %(default)s)")
    parser.add_argument("--only", choices=("claude", "cursor", "vscode"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    stack, ledger = Stack(args.stack), Ledger()
    stack.set_access_lifespan(LIFESPAN)
    for name, binary, check in (("claude", args.claude, claude_code), ("cursor", args.agent, cursor_agent),
                                ("vscode", args.vscode, lambda s, b, l: vscode(s, b, l, args.vscode_lifespan))):
        if args.only in (None, name):
            if not Path(binary).exists():
                ledger.check(f"{name}: installed at {binary}", False, "not found")
                continue
            try:
                check(stack, binary, ledger)
            except Exception as error:
                ledger.check(f"{name}: ran to completion", False, repr(error)[:300])
    summary = {"passed": sum(r.get("passed", False) for r in ledger.rows), "failed": sum(r.get("passed", True) is False for r in ledger.rows), "checks": ledger.rows}
    if args.output:
        args.output.write_text(json.dumps(summary, indent=2))
    print(json.dumps({k: summary[k] for k in ("passed", "failed")}))
    return 0 if summary["failed"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
