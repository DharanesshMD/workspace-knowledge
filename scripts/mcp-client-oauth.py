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

Clients: Claude Code (`claude`) and the Cursor agent CLI (`agent`, which `cursor agent` runs).
The Cursor and VS Code editors are GUI programs and are not driven here.

The Claude Code check bypasses its first-run screens by writing its (undocumented) config file
and uses a fake API key, so it can break with a new Claude Code version; a failure there means
"re-check by hand", not necessarily "the client is broken". Prints a pass/fail line per check.
"""
import argparse, base64, hashlib, html, http.cookiejar, http.server, json, os, pty, re, select, shutil, signal
import ssl, struct, fcntl, subprocess, sys, tempfile, termios, threading, time, urllib.error, urllib.parse, urllib.request
from pathlib import Path

LIFESPAN = 12  # seconds; short enough to watch a refresh, long enough to finish a sign-in


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

    def set_access_lifespan(self, seconds):
        token = self.call("POST", "/realms/master/protocol/openid-connect/token",
                          form={"grant_type": "password", "client_id": "admin-cli", "username": "admin",
                                "password": self.info["admin_password"]})["access_token"]
        realm = self.call("GET", f"/admin/realms/{self.info['realm']}", token=token)
        self.call("PUT", f"/admin/realms/{self.info['realm']}", dict(realm, accessTokenLifespan=seconds), token=token)

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
        ledger.check(f"cursor agent {version}: lists the 10 server tools with the new token", "Tools for wk (10)" in listed.stdout, listed.stdout[-200:] + listed.stderr[-200:])
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
                     "Tools for wk (10)" in again.stdout and after > before, again.stdout[-200:] + again.stderr[-200:])
        mode = auth_file.stat().st_mode & 0o777
        ledger.note(f"cursor agent {version}: its credential file (mcp-auth.json, holding the tokens) has mode {oct(mode)}"
                    + ("" if mode & 0o077 else " (private)"))
    finally:
        if login and login.poll() is None:
            login.kill()
        shutil.rmtree(home, ignore_errors=True)
        shutil.rmtree(work, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--stack", type=Path, required=True, help="info file written by keycloak-acceptance.py --stack-only")
    parser.add_argument("--claude", default=str(Path.home() / ".local/bin/claude"))
    parser.add_argument("--agent", default=str(Path.home() / ".local/bin/agent"))
    parser.add_argument("--only", choices=("claude", "cursor"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    stack, ledger = Stack(args.stack), Ledger()
    stack.set_access_lifespan(LIFESPAN)
    for name, binary, check in (("claude", args.claude, claude_code), ("cursor", args.agent, cursor_agent)):
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
