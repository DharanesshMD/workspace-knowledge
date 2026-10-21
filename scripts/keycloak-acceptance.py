#!/usr/bin/env python3
"""Production-mode OIDC acceptance against a disposable Keycloak over HTTPS.

Everything is created and removed by this script: a throwaway CA, a Keycloak
container serving HTTPS with that CA, a Caddy HTTPS reverse proxy in front of a
locally built wk-server --production, and a PostgreSQL container. Users, secrets
and keys are synthetic and random per run. Output is a pass/fail ledger with no
tokens or credentials.

Usage: python3 scripts/keycloak-acceptance.py [--keycloak-image IMAGE] [--output FILE]
Requires Docker, openssl and built target/debug/{wk-server,wk-agent,wk,wk-mcp}. The native
collector and `wk-mcp login` checks store synthetic credentials in the macOS keychain (removed
afterwards) and are skipped, and reported as skipped, on other platforms.
"""
import argparse, base64, hashlib, html, http.cookiejar, json, os, re, secrets, shutil
import socket, ssl, subprocess, sys, tempfile, time, urllib.error, urllib.parse
import urllib.request, uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SERVER = ROOT / "target/debug/wk-server"
AGENT = ROOT / "target/debug/wk-agent"
WK = ROOT / "target/debug/wk"
MCP = ROOT / "target/debug/wk-mcp"
REALM = "wk"
COLLECTOR_AUDIENCE = "wk-collector"
OPENSSL = os.environ.get("WK_TEST_OPENSSL") or (
    "/opt/homebrew/opt/openssl@3/bin/openssl"
    if Path("/opt/homebrew/opt/openssl@3/bin/openssl").exists()
    else shutil.which("openssl")
)


def run(*args, input=None, check=True, env=None):
    p = subprocess.run(args, input=input, capture_output=True, text=True, env=env)
    if check and p.returncode:
        raise RuntimeError(f"command failed: {args[0]} {args[1] if len(args) > 1 else ''}: {p.stderr[:300]}")
    return p


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def b64url(raw):
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


class Client:
    """Cookie-keeping HTTPS client that trusts only the throwaway CA and never follows redirects."""

    def __init__(self, ca, cookies=True):
        context = ssl.create_default_context(cafile=str(ca))
        self.jar = http.cookiejar.CookieJar()
        handlers = [NoRedirect(), urllib.request.HTTPSHandler(context=context)]
        if cookies:
            handlers.append(urllib.request.HTTPCookieProcessor(self.jar))
        self.opener = urllib.request.build_opener(*handlers)

    def request(self, url, data=None, headers=None, method=None, form=False, raw=None):
        body = raw
        headers = dict(headers or {})
        if raw is not None:
            headers.setdefault("Content-Type", "application/json")
        elif data is not None:
            if form:
                body = urllib.parse.urlencode(data).encode()
                headers.setdefault("Content-Type", "application/x-www-form-urlencoded")
            else:
                body = json.dumps(data).encode()
                headers.setdefault("Content-Type", "application/json")
        try:
            r = self.opener.open(urllib.request.Request(url, data=body, headers=headers, method=method), timeout=20)
        except urllib.error.HTTPError as e:
            r = e
        return r.code, r.headers, r.read(1 << 20)


class Ledger:
    def __init__(self):
        self.rows = []

    def check(self, name, ok, detail=""):
        # Details can embed one-time protocol values, so they are kept only for failures.
        self.rows.append({"check": name, "passed": bool(ok), "detail": "" if ok else detail})
        print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail and not ok else ""), file=sys.stderr)


STARTER_REALM = ROOT / "deploy/keycloak/wk-realm.json"


def starter_realm(values):
    """The shipped starter realm with its ${WK_*} placeholders filled, exactly as Keycloak would."""
    text = STARTER_REALM.read_text()
    for name, value in values.items():
        text = text.replace("${" + name + "}", value)
    assert "${" not in text, "starter realm has an unfilled placeholder"
    return json.loads(text)


def realm_document(app_origin, secrets_, api_audience):
    """The starter realm (portal, collector and MCP clients) plus a test user and test-only clients."""

    def audience_mapper(audience):
        return {
            "name": f"aud-{audience}",
            "protocol": "openid-connect",
            "protocolMapper": "oidc-audience-mapper",
            "config": {"included.custom.audience": audience, "access.token.claim": "true", "id.token.claim": "false"},
        }

    def token_client(client_id, audiences, lifespan=None, **extra):
        attributes = {}
        if lifespan:
            attributes["access.token.lifespan"] = str(lifespan)
        client = {
            "clientId": client_id,
            "enabled": True,
            "publicClient": True,
            "directAccessGrantsEnabled": True,
            "standardFlowEnabled": False,
            "defaultClientScopes": ["basic"],
            "optionalClientScopes": [],
            "attributes": attributes,
            "protocolMappers": [audience_mapper(a) for a in audiences],
        }
        client.update(extra)
        return client

    realm = starter_realm({
        "WK_CONSOLE_ORIGIN": app_origin,
        "WK_BROWSER_CLIENT_SECRET": secrets_["portal_secret"],
        "WK_OIDC_AUDIENCE": api_audience,
        "WK_COLLECTOR_AUDIENCE": COLLECTOR_AUDIENCE,
    })
    realm["defaultRole"] = {"name": "default-roles-wk", "composite": True,
                            "composites": {"realm": ["offline_access", "uma_authorization"],
                                       "client": {"account": ["view-profile", "manage-account"]}}}
    realm["users"] = [
        {
            "id": secrets_["user_id"],
            "username": "alice",
            "enabled": True,
            "email": "alice@example.invalid",
            "emailVerified": True,
            "realmRoles": ["default-roles-wk"],
            "firstName": "Alice",
            "lastName": "Synthetic",
            "credentials": [{"type": "password", "value": secrets_["password"], "temporary": False}],
        }
    ]
    realm["clients"] += [
        token_client("api-ok", [api_audience]),
        token_client("api-short", [api_audience], lifespan=4),
        token_client("api-rotation", [api_audience], lifespan=900),
        token_client("api-wrong-audience", ["some-other-api"]),
        token_client("api-two-audiences", [api_audience, "some-other-api"]),
        token_client("api-no-audience", []),
        # Keycloak's default scopes add "account" to aud; kept to document that pitfall.
        {
            "clientId": "api-default-scopes",
            "enabled": True,
            "publicClient": True,
            "directAccessGrantsEnabled": True,
            "standardFlowEnabled": False,
            "protocolMappers": [audience_mapper(api_audience)],
        },
    ]
    return realm


class Stack:
    def __init__(self, keycloak_image):
        self.image = keycloak_image
        self.tag = uuid.uuid4().hex[:10]
        self.work = Path(tempfile.mkdtemp(prefix="wk-kc-"))
        self.work.chmod(0o700)
        self.containers = []
        self.server = None
        self.kc_port, self.app_port, self.server_port, self.db_port = (free_port() for _ in range(4))
        self.issuer = f"https://localhost:{self.kc_port}/realms/{REALM}"
        self.app_origin = f"https://localhost:{self.app_port}"
        # The remote MCP resource is the server origin, and the one API audience must equal it.
        self.api_audience = self.app_origin
        self.secrets = {
            "user_id": str(uuid.uuid4()),
            "password": secrets.token_urlsafe(18),
            "portal_secret": secrets.token_urlsafe(24),
            "admin": secrets.token_urlsafe(18),
            "db_owner": secrets.token_hex(16),
            "db_runtime": secrets.token_hex(16),
        }
        # Request paths (never bodies; Caddy redacts credentials) so a real client's discovery can be read back.
        self.access_log = False
        self.ca = self.work / "ca.pem"
        self.db = f"wk_kc_{self.tag}"
        self.role = "wk_kc_runtime"

    # ---- infrastructure -------------------------------------------------
    def certificates(self):
        w = self.work
        run(OPENSSL, "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
            "-keyout", str(w / "ca.key"), "-out", str(self.ca), "-days", "2", "-subj", "/CN=Throwaway Keycloak Acceptance CA",
            "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign")
        run(OPENSSL, "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
            "-keyout", str(w / "leaf.key"), "-out", str(w / "leaf.csr"), "-subj", "/CN=localhost")
        (w / "ext.cnf").write_text("subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n")
        run(OPENSSL, "x509", "-req", "-in", str(w / "leaf.csr"), "-CA", str(self.ca), "-CAkey", str(w / "ca.key"),
            "-CAcreateserial", "-out", str(w / "leaf.crt"), "-days", "2", "-extfile", str(w / "ext.cnf"))
        (w / "leaf.pem").write_text((w / "leaf.crt").read_text() + self.ca.read_text())
        for name in ("leaf.key", "leaf.pem", "ca.pem"):
            (w / name).chmod(0o644)  # read by the non-root container users; the directory is mode 0700

    def start_postgres(self):
        name = f"wk-kc-pg-{self.tag}"
        self.containers.append(name)
        run("docker", "run", "-d", "--name", name, "-e", "POSTGRES_USER=wk_operator", "-e",
            f"POSTGRES_PASSWORD={self.secrets['db_owner']}", "-e", f"POSTGRES_DB={self.db}",
            "-p", f"127.0.0.1:{self.db_port}:5432", "postgres:17-alpine")
        for _ in range(100):
            if run("docker", "exec", name, "pg_isready", "-h", "127.0.0.1", "-U", "wk_operator", "-d", self.db, check=False).returncode == 0:
                break
            time.sleep(0.3)
        self.owner_url = f"postgresql://wk_operator:{self.secrets['db_owner']}@127.0.0.1:{self.db_port}/{self.db}"
        self.runtime_url = f"postgresql://{self.role}:{self.secrets['db_runtime']}@127.0.0.1:{self.db_port}/{self.db}"
        self.cli("migrate")
        sql = f"CREATE ROLE {self.role} LOGIN PASSWORD '{self.secrets['db_runtime']}';\n"
        sql += (ROOT / "deploy/scripts/application-role.sql").read_text().replace("wk_app", self.role)
        run("docker", "exec", "-i", name, "psql", "-X", "-v", "ON_ERROR_STOP=1", "-U", "wk_operator", "-d", self.db, input=sql)

    def cli(self, mode, value=None, extra_env=None):
        env = dict(os.environ, WK_DATABASE_URL=self.owner_url, **(extra_env or {}))
        p = subprocess.run([str(SERVER), mode], env=env, input=None if value is None else json.dumps(value), capture_output=True, text=True, timeout=60)
        if p.returncode:
            raise RuntimeError(f"wk-server {mode} failed: {p.stderr[:200]}")
        return p.stdout.strip()

    def provision(self):
        """Provision alice to the issuer/subject Keycloak will present; the app never infers membership from claims."""
        self.cli("provision", {"issuer": self.issuer, "subject": self.secrets["user_id"], "org": "kc-org", "user": "alice",
                               "product": "demo", "services": ["backend"], "can_review": True})

    def start_keycloak(self):
        name = f"wk-kc-keycloak-{self.tag}"
        self.containers.append(name)
        realm = self.work / "realm.json"
        realm.write_text(json.dumps(realm_document(self.app_origin, self.secrets, self.api_audience)))
        realm.chmod(0o644)
        run("docker", "run", "-d", "--name", name,
            "-e", "KC_BOOTSTRAP_ADMIN_USERNAME=admin", "-e", f"KC_BOOTSTRAP_ADMIN_PASSWORD={self.secrets['admin']}",
            "-p", f"127.0.0.1:{self.kc_port}:8443",
            "-v", f"{self.work}/leaf.pem:/certs/leaf.pem:ro", "-v", f"{self.work}/leaf.key:/certs/leaf.key:ro",
            "-v", f"{realm}:/opt/keycloak/data/import/realm.json:ro",
            self.image, "start", "--import-realm", f"--hostname=https://localhost:{self.kc_port}",
            "--https-certificate-file=/certs/leaf.pem", "--https-certificate-key-file=/certs/leaf.key", "--http-enabled=false")
        anonymous = Client(self.ca)
        last = ""
        for _ in range(240):
            try:
                status = anonymous.request(self.issuer + "/.well-known/openid-configuration")[0]
                if status == 200:
                    return
                last = f"status {status}"
            except Exception as error:
                last = repr(error)
            time.sleep(1)
        logs = run("docker", "logs", "--tail", "30", name, check=False)
        raise RuntimeError(f"Keycloak did not become ready ({last}): " + (logs.stdout + logs.stderr)[-600:])

    def start_proxy(self):
        name = f"wk-kc-caddy-{self.tag}"
        self.containers.append(name)
        caddyfile = self.work / "Caddyfile"
        caddyfile.write_text(
            "{\n  admin off\n  auto_https off\n}\n"
            f"https://localhost:{self.app_port} {{\n  tls /certs/leaf.pem /certs/leaf.key\n"
            + ("  log\n" if self.access_log else "")
            + f"  reverse_proxy host.docker.internal:{self.server_port}\n}}\n"
        )
        caddyfile.chmod(0o644)
        run("docker", "run", "-d", "--name", name, "-p", f"127.0.0.1:{self.app_port}:{self.app_port}",
            "-v", f"{caddyfile}:/etc/caddy/Caddyfile:ro", "-v", f"{self.work}/leaf.pem:/certs/leaf.pem:ro",
            "-v", f"{self.work}/leaf.key:/certs/leaf.key:ro", "caddy:2-alpine")

    def server_env(self, **overrides):
        env = dict(
            os.environ,
            WK_DATABASE_URL=self.runtime_url,
            WK_OIDC_ISSUER=self.issuer,
            WK_OIDC_AUDIENCE=self.api_audience,
            WK_MCP_RESOURCE=self.app_origin,
            WK_MCP_CLIENT_ID="wk-mcp",
            WK_OIDC_JWKS_URL=self.issuer + "/protocol/openid-connect/certs",
            WK_OIDC_CA_FILE=str(self.ca),
            WK_CONSOLE_ORIGIN=self.app_origin,
            WK_BROWSER_CLIENT_ID="wk-portal",
            WK_BROWSER_CLIENT_SECRET=self.secrets["portal_secret"],
            WK_COLLECTOR_AUDIENCE=COLLECTOR_AUDIENCE,
            WK_COLLECTOR_CLIENT_ID="wk-collector",
            WK_BIND=f"127.0.0.1:{self.server_port}",
        )
        env.update(overrides)
        return {k: v for k, v in env.items() if v is not None}

    def start_server(self):
        self.server = subprocess.Popen([str(SERVER), "--production"], env=self.server_env(),
                                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        anonymous = Client(self.ca)
        for _ in range(100):
            if self.server.poll() is not None:
                raise RuntimeError("wk-server exited: " + self.server.stderr.read()[:300])
            try:
                if anonymous.request(self.app_origin + "/health/ready")[0] == 200:
                    return
            except Exception:
                time.sleep(0.3)
        raise RuntimeError("wk-server was not reachable through the HTTPS proxy")

    def stop_server(self):
        if self.server and self.server.poll() is None:
            self.server.terminate()
            try:
                self.server.wait(10)
            except subprocess.TimeoutExpired:
                self.server.kill()

    def cleanup(self):
        self.stop_server()
        for name in self.containers:
            run("docker", "rm", "-f", "-v", name, check=False)
        shutil.rmtree(self.work, ignore_errors=True)

    # ---- identity helpers -----------------------------------------------
    def token_endpoint(self):
        return self.issuer + "/protocol/openid-connect/token"

    def password_token(self, client_id, ledger_client):
        status, _, raw = ledger_client.request(
            self.token_endpoint(),
            {"grant_type": "password", "client_id": client_id, "username": "alice", "password": self.secrets["password"], "scope": "openid"},
            form=True)
        return status, (json.loads(raw) if raw else {})

    def login_form_action(self, browser, authorization_url):
        status, headers, raw = browser.request(authorization_url)
        if status != 200:
            raise RuntimeError(f"authorization endpoint returned {status}")
        match = re.search(r'<form[^>]*id="kc-form-login"[^>]*action="([^"]+)"', raw.decode())
        if not match:
            raise RuntimeError("login form not found")
        return html.unescape(match.group(1))

    def authenticate(self, browser, authorization_url):
        """Submit the Keycloak login form and return the final redirect Location."""
        action = self.login_form_action(browser, authorization_url)
        status, headers, _ = browser.request(action, {"username": "alice", "password": self.secrets["password"], "credentialId": ""}, form=True)
        if status != 302:
            raise RuntimeError(f"login returned {status}")
        return headers["Location"]


def decode_claims(token):
    payload = token.split(".")[1]
    return json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))


def accept(stack, ledger):
    # Cookie-less on purpose: cookies ignore ports, so localhost provider cookies would reach the app.
    ca_client = Client(stack.ca, cookies=False)
    def api(client, token):
        return client.request(stack.app_origin + "/v1/search", {"product": "demo", "query": "middleware", "limit": 3},
                              headers={"Authorization": "Bearer " + token})
    stack.provision()

    # 1. Browser authorization-code + PKCE login through the real provider.
    browser = Client(stack.ca)
    status, headers, _ = browser.request(stack.app_origin + "/auth/login")
    ledger.check("browser login redirects to the provider", status in (302, 303) and headers.get("Location", "").startswith(stack.issuer))
    login_url = headers["Location"]
    query = urllib.parse.parse_qs(urllib.parse.urlparse(login_url).query)
    ledger.check("authorization request carries state, nonce and an S256 PKCE challenge",
                 all(k in query for k in ("state", "nonce", "code_challenge")) and query.get("code_challenge_method") == ["S256"]
                 and query.get("client_id") == ["wk-portal"] and query.get("redirect_uri") == [stack.app_origin + "/auth/callback"])
    callback = stack.authenticate(browser, login_url)
    ledger.check("provider redirects back to the registered HTTPS callback", callback.startswith(stack.app_origin + "/auth/callback?"), callback[:60])
    replay_client = Client(stack.ca)
    status, headers, _ = browser.request(callback)
    cookies = headers.get_all("Set-Cookie") or []
    ledger.check("callback creates a session", status in (302, 303) and any("__Host-wk_session" in c for c in cookies), str(status))
    flags = " ".join(c for c in cookies if "__Host-wk_session" in c)
    ledger.check("session cookie is Secure, HttpOnly, SameSite and path-scoped",
                 all(flag in flags for flag in ("Secure", "HttpOnly", "SameSite=Lax", "Path=/")) and "Domain" not in flags)
    status, _, raw = browser.request(stack.app_origin + "/v1/me")
    me = json.loads(raw) if status == 200 else {}
    ledger.check("session resolves the explicitly provisioned identity", status == 200 and me.get("user") == "alice", str(status))
    status, _, _ = browser.request(callback)
    ledger.check("authorization code replay is rejected", status >= 400, str(status))
    bad = Client(stack.ca)
    _, headers, _ = bad.request(stack.app_origin + "/auth/login")
    bad_callback = stack.authenticate(bad, headers["Location"])
    tampered = re.sub(r"state=[^&]+", "state=" + secrets.token_hex(16), bad_callback)
    status, _, _ = bad.request(tampered)
    ledger.check("callback with a forged state is rejected", status >= 400, str(status))
    status, _, _ = bad.request(stack.app_origin + "/v1/me")
    ledger.check("forged-state callback leaves no session", status == 401, str(status))
    status, _, _ = browser.request(stack.app_origin + "/auth/logout", method="POST", headers={"Origin": stack.app_origin})
    ledger.check("logout without the CSRF token is refused", status == 403, str(status))
    status, _, _ = browser.request(stack.app_origin + "/auth/logout", method="POST",
                                   headers={"Origin": stack.app_origin, "X-CSRF-Token": me.get("csrf_token", "")})
    status_after, _, _ = browser.request(stack.app_origin + "/v1/me")
    ledger.check("logout ends the application session", status in (200, 204, 303) and status_after == 401, f"{status}/{status_after}")

    # 2. Bearer access tokens: audience separation, expiry, signature.
    status, token = stack.password_token("api-ok", ca_client)
    good = token.get("access_token", "")
    claims = decode_claims(good) if good else {}
    ledger.check("provider issues a single-audience API token", status == 200 and claims.get("aud") == stack.api_audience and "iat" in claims, str(claims.get("aud")))
    status, _, _ = api(ca_client, good)
    ledger.check("API token is accepted", status == 200, str(status))
    for client_id, why in (("api-wrong-audience", "wrong audience"), ("api-two-audiences", "extra audience"),
                           ("api-no-audience", "missing audience"), ("api-default-scopes", "Keycloak default scopes add the account audience")):
        _, token = stack.password_token(client_id, ca_client)
        value = token.get("access_token", "")
        status, _, _ = api(ca_client, value)
        ledger.check(f"API token rejected: {why}", bool(value) and status == 401, f"aud={decode_claims(value).get('aud') if value else None} status={status}")
    status, _, _ = api(ca_client, good[:-4] + ("AAAA" if not good.endswith("AAAA") else "BBBB"))
    ledger.check("API token with a corrupted signature is rejected", status == 401, str(status))
    header, payload, _ = good.split(".")
    status, _, _ = api(ca_client, b64url(json.dumps({"alg": "none", "typ": "JWT"}).encode()) + "." + payload + ".")
    ledger.check("unsigned (alg=none) token is rejected", status == 401, str(status))
    _, token = stack.password_token("api-short", ca_client)
    short = token.get("access_token", "")
    ledger.check("short-lived token is accepted while fresh", api(ca_client, short)[0] == 200)
    time.sleep(7)
    ledger.check("expired token is rejected", api(ca_client, short)[0] == 401)

    # 3. Collector flow: authorization code + PKCE for the native public client.
    verifier = secrets.token_urlsafe(48)
    challenge = b64url(hashlib.sha256(verifier.encode()).digest())
    redirect = f"http://127.0.0.1:{free_port()}/callback"
    native = Client(stack.ca)
    url = stack.issuer + "/protocol/openid-connect/auth?" + urllib.parse.urlencode({
        "client_id": "wk-collector", "response_type": "code", "scope": "openid", "redirect_uri": redirect,
        "state": secrets.token_hex(8), "nonce": secrets.token_hex(8), "code_challenge": challenge, "code_challenge_method": "S256"})
    location = stack.authenticate(native, url)
    ledger.check("provider accepts the loopback redirect with an ephemeral port", location.startswith(redirect + "?"), location[:80])
    code = urllib.parse.parse_qs(urllib.parse.urlparse(location).query)["code"][0]
    status, _, raw = native.request(stack.token_endpoint(), {"grant_type": "authorization_code", "client_id": "wk-collector", "code": code,
                                                              "redirect_uri": redirect, "code_verifier": verifier}, form=True)
    collector_token = json.loads(raw).get("access_token", "") if status == 200 else ""
    claims = decode_claims(collector_token) if collector_token else {}
    ledger.check("collector token has exactly the collector audience", claims.get("aud") == COLLECTOR_AUDIENCE and claims.get("sub") == stack.secrets["user_id"], str(claims.get("aud")))
    status, _, _ = native.request(stack.token_endpoint(), {"grant_type": "authorization_code", "client_id": "wk-collector", "code": code,
                                                            "redirect_uri": redirect, "code_verifier": verifier}, form=True)
    ledger.check("provider rejects authorization-code reuse", status == 400, str(status))
    ledger.check("collector token is not accepted as an API token", api(ca_client, collector_token)[0] == 401)

    # 4. Signed collector requests: the collector token is accepted only on collector routes.
    device_key = stack.work / "device.pem"
    run(OPENSSL, "genpkey", "-algorithm", "ED25519", "-out", str(device_key))
    public_key = subprocess.check_output([OPENSSL, "pkey", "-in", str(device_key), "-pubout", "-outform", "DER"], stderr=subprocess.DEVNULL)[-32:].hex()
    device, approval = str(uuid.uuid4()), secrets.token_hex(32)

    def signed(operation, value, token):
        body = json.dumps(value, separators=(",", ":")).encode()
        path, now, nonce = "/collector/" + operation, int(time.time()), str(uuid.uuid4())
        message = json.dumps(["wk-device-v1", stack.app_origin, "POST", path, now, nonce, device,
                              hashlib.sha256(token.encode()).hexdigest(), hashlib.sha256(body).hexdigest()], separators=(",", ":")).encode()
        (stack.work / "message").write_bytes(message)
        signature = subprocess.check_output([OPENSSL, "pkeyutl", "-sign", "-rawin", "-inkey", str(device_key), "-in", str(stack.work / "message")],
                                            stderr=subprocess.DEVNULL).hex()
        return ca_client.request(stack.app_origin + path, raw=body, headers={
            "Authorization": "Bearer " + token, "X-WK-Device": device, "X-WK-Time": str(now), "X-WK-Nonce": nonce, "X-WK-Signature": signature})

    registration = {"id": device, "product": "demo", "services": ["backend"], "public_key": public_key, "approval_hash": approval}
    status, _, _ = signed("register", registration, good)
    ledger.check("device registration with an API-audience token is rejected", status == 401, str(status))
    status, _, raw = signed("register", registration, collector_token)
    ledger.check("device registration with the Keycloak collector token succeeds", status in (200, 201), f"{status} {raw[:200]!r}")
    status, _, _ = signed("policy", {"product": "demo", "approval_hash": approval, "request": {}}, collector_token)
    ledger.check("signed collector request authenticates end to end", status == 200, str(status))
    status, _, _ = signed("policy", {"product": "demo", "approval_hash": approval, "request": {}}, good)
    ledger.check("signed collector request with an API-audience token is rejected", status == 401, str(status))

    # 4b. The native collector reaches the same private-CA stack with its own --ca-file.
    collector = native_collector(stack, ledger)

    # 4c. `wk-mcp login` and the stdio bridge, then the hosted /mcp endpoint itself.
    mcp = remote_mcp(stack, ledger, ca_client, good)

    # 4d. Native remote-MCP sign-in: dynamic client registration is off until the documented setup step.
    signin = remote_signin(stack, ledger, ca_client)

    # 5. Misconfiguration fails closed with an operator-readable reason.
    other_ca = ROOT / "crates/server/tests/fixtures/synthetic-ca.pem"
    garbage = stack.work / "garbage.pem"
    garbage.write_text("not a certificate")
    for label, overrides, expected in (
        ("untrusted CA bundle", {"WK_OIDC_CA_FILE": str(other_ca)}, "JWKS unavailable"),
        ("malformed CA bundle", {"WK_OIDC_CA_FILE": str(garbage)}, "OIDC CA bundle"),
        ("plain-HTTP JWKS URL", {"WK_OIDC_JWKS_URL": stack.issuer.replace("https", "http") + "/protocol/openid-connect/certs"}, "JWKS requires HTTPS"),
        ("issuer that differs from the provider's advertised issuer", {"WK_OIDC_ISSUER": stack.issuer + "/"}, "provider_issuer_mismatch"),
        ("unreachable provider", {"WK_OIDC_JWKS_URL": f"https://localhost:{free_port()}/certs"}, "JWKS unavailable"),
    ):
        p = subprocess.run([str(SERVER), "--production"], env=stack.server_env(WK_BIND="127.0.0.1:0", **overrides),
                           capture_output=True, text=True, timeout=60)
        ledger.check(f"startup fails closed: {label}", p.returncode != 0 and expected in p.stderr, p.stderr.strip()[:120])

    # 6. Key rotation: a newly promoted signing key is honoured after the next key refresh.
    admin = Client(stack.ca)
    status, _, raw = admin.request(f"https://localhost:{stack.kc_port}/realms/master/protocol/openid-connect/token",
                                   {"grant_type": "password", "client_id": "admin-cli", "username": "admin", "password": stack.secrets["admin"]}, form=True)
    bearer = {"Authorization": "Bearer " + json.loads(raw)["access_token"]}
    admin_api = f"https://localhost:{stack.kc_port}/admin/realms/{REALM}/components"
    kid = lambda t: json.loads(base64.urlsafe_b64decode(t.split(".")[0] + "==")).get("kid")
    _, token = stack.password_token("api-rotation", ca_client)
    before = token["access_token"]
    ledger.check("pre-rotation token is accepted", api(ca_client, before)[0] == 200)
    _, _, raw = admin.request(admin_api + "?type=org.keycloak.keys.KeyProvider", headers=bearer)
    old_provider = next(c for c in json.loads(raw) if c["providerId"] == "rsa-generated")
    created = admin.request(admin_api, {"name": "rsa-rotated", "providerId": "rsa-generated", "providerType": "org.keycloak.keys.KeyProvider",
                                        "config": {"priority": ["200"], "keySize": ["2048"], "algorithm": ["RS256"]}}, headers=bearer)
    ledger.check("provider accepted a new higher-priority signing key", created[0] == 201, str(created[0]))
    _, token = stack.password_token("api-rotation", ca_client)
    after = token["access_token"]
    ledger.check("new tokens are signed by a different key", kid(after) != kid(before))
    started, accepted_after = time.monotonic(), None
    while time.monotonic() - started < 130:
        if api(ca_client, after)[0] == 200:
            accepted_after = round(time.monotonic() - started)
            break
        time.sleep(3)
    ledger.check("new key is honoured within two refresh intervals", accepted_after is not None, "never accepted")
    ledger.check("old-key token stays valid while the provider still publishes the old key", api(ca_client, before)[0] == 200)
    admin.request(f"{admin_api}/{old_provider['id']}", headers=bearer, method="DELETE")
    started, rejected_after = time.monotonic(), None
    while time.monotonic() - started < 130:
        if api(ca_client, before)[0] == 401:
            rejected_after = round(time.monotonic() - started)
            break
        time.sleep(3)
    ledger.check("token signed by a withdrawn key is rejected after the next refresh", rejected_after is not None, "still accepted")
    ledger.check("tokens from the promoted key keep working", api(ca_client, after)[0] == 200)
    return {"new_key_accepted_after_seconds": accepted_after, "withdrawn_key_rejected_after_seconds": rejected_after, "native_collector": collector, "wk_mcp": mcp, "remote_mcp_signin": signin}


def native_collector(stack, ledger):
    """`wk-agent device connect` against the private-CA stack: server and provider trust only --ca-file."""
    if sys.platform != "darwin":
        return "skipped: the harness cleans up synthetic macOS keychain entries only"
    scratch = Path(tempfile.mkdtemp(prefix="wk-kc-agent-", dir=stack.work))
    workspace, consent = scratch / "workspace", scratch / "approval"
    (workspace / "backend").mkdir(parents=True)
    run(str(WK), "--state-dir", str(consent), "workspace", "approve", str(workspace), "--product", "demo", "--service", "backend", "--yes")
    binding = consent / (hashlib.sha256(str(workspace.resolve()).encode()).hexdigest() + ".json")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    states = []

    def connect(state, *extra):
        states.append(state)
        return subprocess.Popen([str(AGENT), "--state-dir", str(state), "device", "connect", "--binding", str(binding), "--service", "backend",
                                 "--server", stack.app_origin, "--summarizer", "rules", "--yes", "--no-browser", *extra],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def agent(state, *args):
        return subprocess.run([str(AGENT), "--state-dir", str(state), *args], capture_output=True, text=True, timeout=120)

    def sign_in(proc, expected_issuer):
        """Play the system browser: complete the Keycloak login and return the loopback callback to the collector."""
        attempt = json.loads(proc.stderr.readline())["authorization_url"]
        assert attempt.startswith(expected_issuer), "authorization URL points elsewhere"
        location = stack.authenticate(Client(stack.ca), attempt)
        assert opener.open(location, timeout=30).status == 200

    try:
        # Without a bundle the public roots cannot verify the private CA, and nothing falls back.
        other = ROOT / "crates/server/tests/fixtures/synthetic-ca.pem"
        garbage = scratch / "garbage.pem"
        garbage.write_text("not a certificate")
        for label, extra in (("no --ca-file", ()), ("an unrelated --ca-file", ("--ca-file", str(other))), ("a malformed --ca-file", ("--ca-file", str(garbage)))):
            state = scratch / ("refused-" + label.replace(" ", "-"))
            p = connect(state, *extra)
            _, error = p.communicate(timeout=120)
            ledger.check(f"collector connect fails closed with {label}", p.returncode != 0 and not (state / "agent.json").exists()
                         and not (state / "server-ca.pem").exists(), error.strip()[:100])
        native = scratch / "native"
        proc = connect(native, "--ca-file", str(stack.ca))
        sign_in(proc, stack.issuer)
        stdout, error = proc.communicate(timeout=300)
        connected = json.loads(stdout) if proc.returncode == 0 else {}
        ledger.check("collector connects through the private-CA Keycloak and Caddy with --ca-file", bool(connected.get("connected")), error.strip()[:160])
        stored = native / "server-ca.pem"
        ledger.check("the approved bundle is copied into private state, owner-only", stored.exists() and stored.read_bytes() == stack.ca.read_bytes()
                     and (stored.stat().st_mode & 0o077) == 0)
        status = agent(native, "device", "status")
        ledger.check("collector status reports the private CA", status.returncode == 0 and json.loads(status.stdout).get("server_trust") == "private_ca", status.stderr[:100])
        poll = agent(native, "device", "poll")
        ledger.check("signed collector poll reaches the server over the private CA", poll.returncode == 0, poll.stderr[:160])
        login = subprocess.Popen([str(AGENT), "--state-dir", str(native), "device", "login", "--no-browser"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        sign_in(login, stack.issuer)
        out, error = login.communicate(timeout=180)
        ledger.check("collector re-login uses the stored bundle for discovery and token exchange", login.returncode == 0 and json.loads(out).get("authenticated") is True, error.strip()[:160])
        stored.write_bytes(other.read_bytes())
        tampered = agent(native, "device", "poll")
        ledger.check("a replaced bundle that no longer chains to the server stops the collector", tampered.returncode != 0, tampered.stderr[:100])
        stored.write_text("garbage")
        broken = agent(native, "device", "poll")
        ledger.check("an unusable stored bundle fails instead of using public roots", broken.returncode != 0 and "stored CA bundle invalid" in broken.stderr, broken.stderr[:100])
    finally:
        for state in states:
            account = hashlib.sha256(str(state.resolve() if state.exists() else state).encode()).hexdigest()
            for purpose in ("capture-key", "ipc-key", "device-key", "server-token"):
                subprocess.run(["/usr/bin/security", "delete-generic-password", "-s", "workspace-knowledge", "-a", f"{account}:{purpose}"],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return "ran"


def play_browser(stack, proc, expected_issuer):
    """Play the system browser for a process that printed its authorization URL on stderr."""
    attempt = json.loads(proc.stderr.readline())["authorization_url"]
    assert attempt.startswith(expected_issuer), "authorization URL points elsewhere"
    location = stack.authenticate(Client(stack.ca), attempt)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    assert opener.open(location, timeout=30).status == 200
    return urllib.parse.parse_qs(urllib.parse.urlparse(attempt).query)


def mcp_exchange(env, calls, timeout=60):
    """Speak MCP over stdio to a `wk-mcp` bridge; returns the JSON-RPC response for each call."""
    import select
    proc = subprocess.Popen([str(MCP)], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def send(message):
        proc.stdin.write(json.dumps(message) + "\n")
        proc.stdin.flush()

    def receive(identifier):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if proc.poll() is not None:
                return {"error": {"message": "bridge exited: " + proc.stderr.read()[:200]}}
            if select.select([proc.stdout], [], [], 1)[0]:
                line = proc.stdout.readline()
                if line and json.loads(line).get("id") == identifier:
                    return json.loads(line)
        return {"error": {"message": "timed out"}}

    results = []
    try:
        send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "keycloak-acceptance", "version": "0"}}})
        first = receive(1)
        if "result" not in first:
            return [first]
        send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        for number, (method, params) in enumerate(calls, start=2):
            send({"jsonrpc": "2.0", "id": number, "method": method, "params": params})
            results.append(receive(number))
    finally:
        proc.stdin.close()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            proc.kill()
    return results


def remote_mcp(stack, ledger, ca_client, api_token):
    """The hosted MCP endpoint, then `wk-mcp login` and the bridge against the private-CA stack."""
    metadata = ca_client.request(stack.app_origin + "/.well-known/oauth-protected-resource")
    document = json.loads(metadata[2]) if metadata[0] == 200 else {}
    ledger.check("protected-resource metadata names the resource, the single issuer and the sign-in client",
                 document.get("resource") == stack.app_origin and document.get("authorization_servers") == [stack.issuer]
                 and document.get("wk_mcp_client_id") == "wk-mcp", str(document))
    initialize = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "acceptance", "version": "0"}}}
    headers = {"Accept": "application/json, text/event-stream"}
    status, response, _ = ca_client.request(stack.app_origin + "/mcp/demo", initialize, headers=headers)
    challenge = response.get("WWW-Authenticate", "")
    ledger.check("/mcp/demo without a token is 401 and points at the resource metadata",
                 status == 401 and "resource_metadata" in challenge and stack.app_origin in challenge, f"{status} {challenge}")
    status, _, raw = ca_client.request(stack.app_origin + "/mcp/demo", initialize, headers=dict(headers, Authorization="Bearer " + api_token))
    ledger.check("/mcp/demo accepts a single-audience token", status == 200 and b"serverInfo" in raw, f"{status} {raw[:100]!r}")
    _, token = stack.password_token("api-two-audiences", ca_client)
    status, _, _ = ca_client.request(stack.app_origin + "/mcp/demo", initialize, headers=dict(headers, Authorization="Bearer " + token.get("access_token", "")))
    ledger.check("/mcp/demo rejects a token with an extra audience", status == 401, str(status))
    if sys.platform != "darwin":
        return "skipped: the harness cleans up synthetic macOS keychain entries only"

    account = "mcp:" + hashlib.sha256(stack.app_origin.encode()).hexdigest()
    keychain = lambda: subprocess.run(["/usr/bin/security", "find-generic-password", "-s", "workspace-knowledge", "-a", account],
                                      capture_output=True).returncode == 0
    bridge_env = {k: v for k, v in os.environ.items() if k not in ("WK_ACCESS_TOKEN", "WK_DEV_TOKEN", "WK_SERVER_CA_FILE", "WK_SERVER_URL", "WK_MCP_CLIENT_ID")}
    bridge_env.update(WK_SERVER_URL=stack.app_origin, WK_PRODUCT="demo")
    search = ("tools/call", {"name": "knowledge.search", "arguments": {"query": "middleware", "limit": 3}})

    def searched(env):
        responses = mcp_exchange(env, [search])
        reply = responses[-1]
        return "result" in reply and not reply["result"].get("isError"), json.dumps(reply)[:200]

    def login(*extra, ca=True):
        argv = [str(MCP), "login", "--server", stack.app_origin, "--no-browser", *extra]
        if ca:
            argv += ["--ca-file", str(stack.ca)]
        return subprocess.Popen(argv, env=bridge_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    try:
        # Without a bundle the public roots cannot verify the private CA, and nothing falls back.
        p = login(ca=False)
        _, error = p.communicate(timeout=60)
        ledger.check("wk-mcp login fails closed without the private CA", p.returncode != 0 and not keychain(), error.strip()[:120])
        p = login("--ca-file", str(ROOT / "crates/server/tests/fixtures/synthetic-ca.pem"), ca=False)
        _, error = p.communicate(timeout=60)
        ledger.check("wk-mcp login fails closed with an unrelated CA bundle", p.returncode != 0 and not keychain(), error.strip()[:120])

        p = login()
        query = play_browser(stack, p, stack.issuer)
        ledger.check("wk-mcp login asks for the MCP resource with PKCE S256 and a loopback redirect",
                     query.get("resource") == [stack.app_origin] and query.get("code_challenge_method") == ["S256"]
                     and query.get("client_id") == ["wk-mcp"] and query["redirect_uri"][0].startswith("http://127.0.0.1:"), str(query))
        out, error = p.communicate(timeout=120)
        signed = json.loads(out) if p.returncode == 0 else {}
        ledger.check("wk-mcp login signs in through the private-CA Keycloak", signed.get("authenticated") is True, error.strip()[:160])
        ledger.check("login output carries no token material", "eyJ" not in out + error)
        ledger.check("the refresh token is in the OS credential store", keychain())
        status = subprocess.run([str(MCP), "status", "--server", stack.app_origin], env=bridge_env, capture_output=True, text=True)
        ledger.check("wk-mcp status reports the stored sign-in without contacting the server",
                     status.returncode == 0 and json.loads(status.stdout).get("signed_in") is True and "eyJ" not in status.stdout)

        trusted = dict(bridge_env, WK_SERVER_CA_FILE=str(stack.ca))
        ok, detail = searched(trusted)
        ledger.check("the stdio bridge searches with the stored sign-in and no copied token", ok, detail)
        ok, detail = searched(trusted)
        ledger.check("a second bridge start renews silently from the same stored sign-in", ok, detail)
        ok, detail = searched(bridge_env)
        ledger.check("the bridge cannot reach the provider without the CA and does not use public roots", not ok, detail)
        ok, detail = searched(dict(bridge_env, WK_SERVER_CA_FILE=str(ROOT / "crates/server/tests/fixtures/synthetic-ca.pem")))
        ledger.check("the bridge fails with an unrelated CA bundle", not ok, detail)

        p = subprocess.run([str(MCP), "logout", "--server", stack.app_origin], env=bridge_env, capture_output=True, text=True)
        ledger.check("wk-mcp logout removes the credential-store entry", p.returncode == 0 and not keychain(), p.stderr[:100])
        ok, detail = searched(trusted)
        ledger.check("after logout the bridge asks for a login", not ok and "wk-mcp login" in detail, detail)
        ok, detail = searched(dict(trusted, WK_ACCESS_TOKEN=api_token))
        ledger.check("WK_ACCESS_TOKEN still works when nothing is stored", ok, detail)

        p = login("--offline-access")
        play_browser(stack, p, stack.issuer)
        out, error = p.communicate(timeout=120)
        ledger.check("wk-mcp login --offline-access signs in", p.returncode == 0 and keychain(), error.strip()[:160])
        ok, detail = searched(trusted)
        ledger.check("the bridge works with an offline-access sign-in", ok, detail)
    finally:
        subprocess.run(["/usr/bin/security", "delete-generic-password", "-s", "workspace-knowledge", "-a", account],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return "ran"


def remote_signin(stack, ledger, ca_client):
    """What Claude Code, VS Code and Cursor do: register themselves, sign in with consent, then refresh."""
    registration = stack.issuer + "/clients-registrations/openid-connect"
    port = free_port()
    redirect = f"http://localhost:{port}/callback"
    document = {"client_name": "acceptance-native-client", "redirect_uris": [redirect], "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"], "token_endpoint_auth_method": "none"}
    status, _, raw = ca_client.request(registration, document)
    ledger.check("anonymous client registration is refused until the setup step is run", status in (401, 403) and b"not trusted" in raw, f"{status} {raw[:120]!r}")

    script = ROOT / "deploy/keycloak/enable-remote-mcp-signin.py"
    env = dict(os.environ, KC_ADMIN_PASSWORD=stack.secrets["admin"])
    args = [sys.executable, str(script), "--keycloak", f"https://localhost:{stack.kc_port}", "--realm", REALM,
            "--audience", stack.app_origin, "--ca-file", str(stack.ca)]
    first = subprocess.run(args, env=env, capture_output=True, text=True, timeout=120)
    ledger.check("enable-remote-mcp-signin.py configures the realm", first.returncode == 0 and "Trusted Hosts" in first.stdout, first.stderr[:200] + first.stdout[:200])
    again = subprocess.run(args, env=env, capture_output=True, text=True, timeout=120)
    ledger.check("the setup step is idempotent", again.returncode == 0 and "nothing to change" in again.stdout, again.stderr[:200] + again.stdout[:200])
    ledger.check("the setup step takes the admin password from the environment only", stack.secrets["admin"] not in " ".join(args))

    status, _, raw = ca_client.request(registration, document)
    registered = json.loads(raw) if status in (200, 201) else {}
    ledger.check("a native client can now register a loopback redirect", bool(registered.get("client_id")), f"{status} {raw[:120]!r}")
    status, _, raw = ca_client.request(registration, dict(document, redirect_uris=["https://attacker.example/callback"]))
    ledger.check("a client with a non-loopback redirect cannot register", status in (400, 401, 403), f"{status} {raw[:120]!r}")
    if not registered.get("client_id"):
        return "registration failed"

    client_id = registered["client_id"]
    verifier = secrets.token_urlsafe(48)
    browser = Client(stack.ca)
    url = stack.issuer + "/protocol/openid-connect/auth?" + urllib.parse.urlencode({
        "response_type": "code", "client_id": client_id, "redirect_uri": redirect, "state": secrets.token_hex(8),
        "code_challenge": b64url(hashlib.sha256(verifier.encode()).digest()), "code_challenge_method": "S256",
        "resource": stack.app_origin + "/"})
    location = stack.authenticate(browser, url)
    consent = location.startswith(stack.issuer) and "required-action" in location
    ledger.check("Keycloak asks the user to consent to a self-registered client", consent, location[:100])
    if consent:
        status, headers, raw = browser.request(location)
        form = re.search(r'<form[^>]*action="([^"]+)"', raw.decode())
        fields = dict(re.findall(r'<input[^>]*name="(code)"[^>]*value="([^"]*)"', raw.decode()))
        status, headers, _ = browser.request(urllib.parse.urljoin(stack.issuer, html.unescape(form.group(1))), dict(fields, accept="Yes"), form=True)
        location = headers.get("Location", "")
    code = urllib.parse.parse_qs(urllib.parse.urlparse(location).query).get("code", [""])[0]
    ledger.check("consent returns the authorization code to the registered loopback redirect", location.startswith(redirect + "?") and bool(code), location[:100])
    status, _, raw = ca_client.request(stack.token_endpoint(), {"grant_type": "authorization_code", "client_id": client_id, "code": code,
                                                                 "redirect_uri": redirect, "code_verifier": verifier, "resource": stack.app_origin + "/"}, form=True)
    tokens = json.loads(raw) if status == 200 else {}
    claims = decode_claims(tokens["access_token"]) if tokens.get("access_token") else {}
    ledger.check("a self-registered client's token carries exactly the MCP resource audience", claims.get("aud") == stack.app_origin, str(claims.get("aud")))
    ledger.check("it also receives a refresh token", bool(tokens.get("refresh_token")))
    initialize = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                                                                              "clientInfo": {"name": "native", "version": "0"}}}
    headers = {"Accept": "application/json, text/event-stream"}

    def mcp_status(token):
        return ca_client.request(stack.app_origin + "/mcp/demo", initialize, headers=dict(headers, Authorization="Bearer " + token))[0]

    ledger.check("the hosted MCP endpoint accepts that token", mcp_status(tokens.get("access_token", "")) == 200)
    status, _, raw = ca_client.request(stack.token_endpoint(), {"grant_type": "refresh_token", "client_id": client_id,
                                                                 "refresh_token": tokens.get("refresh_token", "")}, form=True)
    refreshed = json.loads(raw) if status == 200 else {}
    claims = decode_claims(refreshed["access_token"]) if refreshed.get("access_token") else {}
    ledger.check("refreshing keeps the exact audience and the endpoint accepts the new token",
                 claims.get("aud") == stack.app_origin and mcp_status(refreshed.get("access_token", "")) == 200, f"{status} {claims.get('aud')}")
    return "ran"


def starter_realm_import(ledger):
    """The shipped realm file, unmodified, imports with its ${WK_*} placeholders filled from the environment."""
    name = f"wk-kc-starter-{uuid.uuid4().hex[:8]}"
    port = free_port()
    values = {"WK_CONSOLE_ORIGIN": "https://knowledge.example", "WK_BROWSER_CLIENT_SECRET": secrets.token_urlsafe(18),
              "WK_OIDC_AUDIENCE": "https://knowledge.example", "WK_COLLECTOR_AUDIENCE": "wk-collector-audience"}
    admin_password = secrets.token_urlsafe(18)
    work = Path(tempfile.mkdtemp(prefix="wk-kc-starter-"))
    try:
        shutil.copy(STARTER_REALM, work / "wk-realm.json")
        (work / "wk-realm.json").chmod(0o644)
        envs = [item for k, v in values.items() for item in ("-e", f"{k}={v}")]
        run("docker", "run", "-d", "--name", name, "-p", f"127.0.0.1:{port}:8080", "-e", "KC_BOOTSTRAP_ADMIN_USERNAME=admin",
            "-e", f"KC_BOOTSTRAP_ADMIN_PASSWORD={admin_password}", *envs, "-v", f"{work}/wk-realm.json:/opt/keycloak/data/import/wk-realm.json:ro",
            "quay.io/keycloak/keycloak:26.4", "start-dev", "--import-realm")
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        base = f"http://127.0.0.1:{port}"

        def fetch(path, form=None, token=None):
            request = urllib.request.Request(base + path, data=urllib.parse.urlencode(form).encode() if form else None,
                                             headers={"Authorization": "Bearer " + token} if token else {})
            return json.loads(opener.open(request, timeout=20).read())

        for _ in range(120):
            try:
                fetch("/realms/wk/.well-known/openid-configuration")
                break
            except Exception:
                time.sleep(1)
        token = fetch("/realms/master/protocol/openid-connect/token", {"grant_type": "password", "client_id": "admin-cli",
                                                                        "username": "admin", "password": admin_password})["access_token"]
        clients = {c["clientId"]: c for c in fetch("/admin/realms/wk/clients", token=token)}
        audience = lambda c: [m["config"]["included.custom.audience"] for m in c.get("protocolMappers", [])]
        secret = fetch(f"/admin/realms/wk/clients/{clients['wk-portal']['id']}/client-secret", token=token)["value"]
        ok = (clients["wk-portal"]["redirectUris"] == ["https://knowledge.example/auth/callback"] and secret == values["WK_BROWSER_CLIENT_SECRET"]
              and audience(clients["wk-mcp"]) == ["https://knowledge.example"] and audience(clients["wk-collector"]) == ["wk-collector-audience"]
              and all(c["defaultClientScopes"] == ["basic"] and c["attributes"].get("pkce.code.challenge.method") == "S256"
                      for c in (clients["wk-portal"], clients["wk-collector"], clients["wk-mcp"])))
        ledger.check("the unmodified starter realm imports with its environment placeholders filled in", ok, json.dumps({k: clients[k].get("redirectUris") for k in clients if k.startswith("wk-")}))
    finally:
        run("docker", "rm", "-f", "-v", name, check=False)
        shutil.rmtree(work, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--keycloak-image", default="quay.io/keycloak/keycloak:26.4")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--stack-only", type=Path, metavar="INFO_FILE",
                        help="start the stack, write its connection details (including synthetic credentials) "
                             "to INFO_FILE, then wait for SIGTERM or Ctrl-C instead of running the checks; for trying real clients")
    args = parser.parse_args()
    assert SERVER.exists() and AGENT.exists() and WK.exists() and MCP.exists(), "build target/debug/{wk-server,wk-agent,wk,wk-mcp} first"
    ledger = Ledger()
    stack = Stack(args.keycloak_image)
    stack.access_log = bool(args.stack_only)
    try:
        stack.certificates()
        stack.start_postgres()
        stack.start_keycloak()
        stack.start_proxy()
        ledger.check("server cannot start without the private CA bundle (fails closed)", _fails_without_ca(stack))
        stack.start_server()
        if args.stack_only:
            return keep_alive(stack, args.stack_only)
        timings = accept(stack, ledger)
        starter_realm_import(ledger)
    finally:
        stack.cleanup()
    summary = {"keycloak_image": args.keycloak_image, "passed": sum(r["passed"] for r in ledger.rows),
               "failed": sum(not r["passed"] for r in ledger.rows), "rotation": timings, "checks": ledger.rows}
    text = json.dumps(summary, indent=2)
    if args.output:
        args.output.write_text(text)
    print(json.dumps({k: summary[k] for k in ("keycloak_image", "passed", "failed")}))
    return 0 if summary["failed"] == 0 else 1


def keep_alive(stack, info_file):
    import signal
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    stack.provision()
    # The documented step that lets native MCP clients register themselves and get the right audience.
    run(sys.executable, str(ROOT / "deploy/keycloak/enable-remote-mcp-signin.py"), "--keycloak", f"https://localhost:{stack.kc_port}",
        "--realm", REALM, "--audience", stack.app_origin, "--ca-file", str(stack.ca),
        # Claude Code and the Cursor agent CLI register loopback redirects; Cursor and VS Code also register their own hosts.
        *[a for host in ("localhost", "127.0.0.1", "www.cursor.com", "anysphere.cursor-mcp", "vscode.dev", "insiders.vscode.dev", "code.visualstudio.com") for a in ("--trusted-host", host)],
        env=dict(os.environ, KC_ADMIN_PASSWORD=stack.secrets["admin"]))
    info = {"app_origin": stack.app_origin, "issuer": stack.issuer, "ca_file": str(stack.ca), "realm": REALM, "product": "demo",
            "username": "alice", "password": stack.secrets["password"], "admin_url": f"https://localhost:{stack.kc_port}",
            "admin_password": stack.secrets["admin"], "work_dir": str(stack.work), "wk_server_port": stack.server_port, "proxy_container": f"wk-kc-caddy-{stack.tag}"}
    info_file.write_text(json.dumps(info, indent=2))
    info_file.chmod(0o600)
    print(f"stack ready; details in {info_file}; send SIGTERM or press Ctrl-C to remove it", file=sys.stderr)
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        return 0


def _fails_without_ca(stack):
    p = subprocess.run([str(SERVER), "--production"], env=stack.server_env(WK_OIDC_CA_FILE=None, WK_BIND="127.0.0.1:0"),
                       capture_output=True, text=True, timeout=60)
    return p.returncode != 0 and "OIDC initialization failed" in p.stderr


if __name__ == "__main__":
    sys.exit(main())
