#!/usr/bin/env python3
"""Bring up a complete single-host Workspace Knowledge stack with Docker Compose, idempotently.

PostgreSQL, Keycloak (starter realm), the server, and a Caddy HTTPS proxy, plus the operator steps
the install guide lists: migrations, the restricted `wk_app` role, Keycloak users, explicit
memberships, and native remote-MCP sign-in for Claude Code and Cursor. Run it again after a
failure or to add users; secrets and certificates in the state directory are reused.

  python3 deploy/stack/stack.py up                       # local: https://sso.localhost:8443, https://wk.localhost:8444
  python3 deploy/stack/stack.py up --domain wk.example.com --bind 0.0.0.0 --user alice:backend:review
  python3 deploy/stack/stack.py status | down [--volumes]

Without --tls-cert/--tls-key it creates a private CA (state-dir/certs/ca.pem) that clients must trust.
Passwords are written only to state-dir/credentials.txt (mode 0600), never printed.
See docs/operations/agent-install.md.
"""
import argparse, json, os, re, secrets, shutil, socket, ssl, subprocess, sys, time, urllib.error, urllib.parse, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REALM, DB = "wk", "workspace_knowledge"
DEFAULT_USERS = ["anika:backend,frontend:review", "ravi:backend,frontend"]
TRUSTED_HOSTS = ["localhost", "127.0.0.1", "www.cursor.com", "anysphere.cursor-mcp", "vscode.dev", "insiders.vscode.dev", "code.visualstudio.com"]


def run(*args, input=None, check=True, env=None, quiet=False):
    p = subprocess.run(args, input=input, text=True, capture_output=True, env=env)
    if check and p.returncode:
        sys.exit(f"command failed: {' '.join(args[:6])}\n{(p.stdout + p.stderr)[-1500:]}")
    if not quiet and p.stdout.strip():
        print(p.stdout.strip())
    return p


def say(message):
    print(f"==> {message}", flush=True)


# *.localhost resolves to loopback in browsers and on macOS; make it true for this script everywhere.
_getaddrinfo = socket.getaddrinfo
socket.getaddrinfo = lambda host, *a, **k: _getaddrinfo("127.0.0.1" if str(host).endswith(".localhost") else host, *a, **k)


class Stack:
    def __init__(self, args):
        self.a = args
        self.state = Path(args.state_dir).expanduser().resolve()
        self.kc_origin = f"https://sso.{args.domain}:{args.kc_port}"
        self.app_origin = f"https://wk.{args.domain}:{args.app_port}"
        self.issuer = f"{self.kc_origin}/realms/{REALM}"
        self.users = [self.parse_user(u) for u in (args.user or DEFAULT_USERS)]

    @staticmethod
    def parse_user(spec):
        name, _, rest = spec.partition(":")
        services, _, flag = rest.partition(":")
        if not re.fullmatch(r"[a-z][a-z0-9._-]{0,31}", name):
            sys.exit(f"bad user name {name!r}: lowercase letters, digits, . _ -")
        return {"name": name, "services": [s for s in (services or "backend").split(",") if s], "review": flag == "review"}

    # ---- state: certificates, secrets, env file ------------------------------------------
    def prepare(self):
        self.state.mkdir(parents=True, exist_ok=True)
        self.state.chmod(0o700)
        certs = self.state / "certs"
        certs.mkdir(exist_ok=True)
        a = self.a
        if a.tls_cert:
            shutil.copyfile(a.tls_cert, certs / "leaf.pem")
            shutil.copyfile(a.tls_key, certs / "leaf.key")
            shutil.copyfile(a.ca_file or a.tls_cert, certs / "ca.pem")
        elif not (certs / "leaf.pem").exists():
            say("creating a private CA and a server certificate")
            names = [f"DNS:sso.{a.domain}", f"DNS:wk.{a.domain}", "DNS:localhost", "IP:127.0.0.1"]
            run("openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", str(certs / "ca.key"),
                "-out", str(certs / "ca.pem"), "-days", "825", "-subj", "/CN=Workspace Knowledge local CA",
                "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign",
                "-addext", "subjectKeyIdentifier=hash", quiet=True)
            run("openssl", "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", str(certs / "leaf.key"),
                "-out", str(certs / "leaf.csr"), "-subj", f"/CN=wk.{a.domain}", quiet=True)
            (certs / "ext.cnf").write_text(f"subjectAltName={','.join(names)}\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n"
                                         "authorityKeyIdentifier=keyid\nsubjectKeyIdentifier=hash\n")
            run("openssl", "x509", "-req", "-in", str(certs / "leaf.csr"), "-CA", str(certs / "ca.pem"), "-CAkey", str(certs / "ca.key"),
                "-CAcreateserial", "-out", str(certs / "leaf.crt"), "-days", "825", "-extfile", str(certs / "ext.cnf"), quiet=True)
            (certs / "leaf.pem").write_text((certs / "leaf.crt").read_text() + (certs / "ca.pem").read_text())
        for name in ("leaf.pem", "leaf.key", "ca.pem"):
            (certs / name).chmod(0o644)  # read by non-root container users; the directory is 0700
        path = self.state / "secrets.json"
        s = json.loads(path.read_text()) if path.exists() else {}
        for key in ("db_owner", "db_app", "kc_db"):  # hex: safe inside a URL and a SQL literal
            s.setdefault(key, secrets.token_hex(16))
        s.setdefault("kc_admin", secrets.token_urlsafe(18))
        s.setdefault("portal", secrets.token_urlsafe(24))
        s.setdefault("users", {})
        for u in self.users:
            s["users"].setdefault(u["name"], secrets.token_urlsafe(15))
        path.write_text(json.dumps(s, indent=1))
        path.chmod(0o600)
        self.secrets = s
        (self.state / "Caddyfile").write_text(
            "{\n  admin off\n  auto_https off\n}\n"
            f"https://wk.{a.domain}:{a.app_port} {{\n  tls /certs/leaf.pem /certs/leaf.key\n  reverse_proxy app:8080\n}}\n")
        (self.state / "Caddyfile").chmod(0o644)
        cred = [f"Keycloak admin: {self.kc_origin}  user admin  password {s['kc_admin']}", f"Portal / MCP server: {self.app_origin}", ""]
        cred += [f"{u['name']}  password {s['users'][u['name']]}" for u in self.users]
        (self.state / "credentials.txt").write_text("\n".join(cred) + "\n")
        (self.state / "credentials.txt").chmod(0o600)
        env = {
            "WK_STACK_DIR": str(self.state), "WK_DOMAIN": a.domain, "WK_APP_PORT": a.app_port, "WK_IMAGE": a.image, "WK_PROXY_BIND": a.bind,
            "WK_POSTGRES_PASSWORD": s["db_owner"], "WK_DATABASE_URL": f"postgres://wk_app:{s['db_app']}@postgres:5432/{DB}",
            "WK_OIDC_ISSUER": self.issuer, "WK_OIDC_JWKS_URL": f"{self.issuer}/protocol/openid-connect/certs", "WK_OIDC_CA_FILE": "/etc/wk/ca.pem",
            "WK_OIDC_AUDIENCE": self.app_origin, "WK_MCP_RESOURCE": self.app_origin, "WK_CONSOLE_ORIGIN": self.app_origin,
            "WK_COLLECTOR_AUDIENCE": "wk-collector", "WK_COLLECTOR_CLIENT_ID": "wk-collector", "WK_MCP_CLIENT_ID": "wk-mcp",
            "WK_BROWSER_CLIENT_ID": "wk-portal", "WK_BROWSER_CLIENT_SECRET": s["portal"],
            "WK_KEYCLOAK_ORIGIN": self.kc_origin, "WK_KEYCLOAK_CERT": str(certs / "leaf.pem"), "WK_KEYCLOAK_KEY": str(certs / "leaf.key"),
            "WK_KEYCLOAK_BIND": f"{a.bind}:{a.kc_port}", "WK_KEYCLOAK_ADMIN_PASSWORD": s["kc_admin"], "WK_KEYCLOAK_DB_PASSWORD": s["kc_db"],
        }
        (self.state / ".env").write_text("".join(f"{k}={v}\n" for k, v in env.items()))
        (self.state / ".env").chmod(0o600)

    # ---- docker compose ---------------------------------------------------------------------
    def compose(self, *args, **kw):
        base = ["docker", "compose", "-p", "wk-stack", "--env-file", str(self.state / ".env"),
                "-f", str(ROOT / "deploy/compose/compose.yaml"), "-f", str(ROOT / "deploy/compose/keycloak.yaml"),
                "-f", str(ROOT / "deploy/stack/compose.override.yaml")]
        return run(*base, *args, **kw)

    def operator(self, mode, payload=None):
        """Run `wk-server MODE` in a one-off container with the database owner connection (never the long-running app's)."""
        owner = f"postgres://wk_owner:{self.secrets['db_owner']}@postgres:5432/{DB}"
        return self.compose("run", "--rm", "--no-deps", "-T", "-e", f"WK_DATABASE_URL={owner}", "app", mode,
                            input=None if payload is None else json.dumps(payload), quiet=True)

    def ensure_image(self):
        if run("docker", "image", "inspect", self.a.image, check=False, quiet=True).returncode:
            say(f"building {self.a.image} (first time only; takes several minutes)")
            self.compose("build", "app", quiet=True)

    def sql(self, text):
        self.compose("exec", "-T", "postgres", "psql", "-X", "-v", "ON_ERROR_STOP=1", "-U", "wk_owner", "-d", DB, input=text, quiet=True)

    # ---- Keycloak admin API -----------------------------------------------------------------
    def https(self, method, url, body=None, form=None, token=None):
        ctx = ssl.create_default_context(cafile=str(self.state / "certs/ca.pem"))
        data, headers = None, {}
        if form is not None:
            data, headers["Content-Type"] = urllib.parse.urlencode(form).encode(), "application/x-www-form-urlencoded"
        elif body is not None:
            data, headers["Content-Type"] = json.dumps(body).encode(), "application/json"
        if token:
            headers["Authorization"] = "Bearer " + token
        opener = urllib.request.build_opener(urllib.request.HTTPSHandler(context=ctx), urllib.request.ProxyHandler({}))
        try:
            with opener.open(urllib.request.Request(url, data=data, headers=headers, method=method), timeout=20) as r:
                raw = r.read()
                return r.status, (json.loads(raw) if raw[:1] in (b"{", b"[") else raw), r.headers
        except urllib.error.HTTPError as e:
            return e.code, e.read()[:300], e.headers

    def wait(self, url, what, seconds=240):
        last = ""
        for _ in range(seconds):
            try:
                status = self.https("GET", url)[0]
                if status == 200:
                    return
                last = f"HTTP {status}"
            except Exception as e:
                last = repr(e)[:120]
            time.sleep(1)
        sys.exit(f"{what} did not become ready ({last}); see: docker compose -p wk-stack logs")

    def keycloak_users(self):
        _, tok, _ = self.https("POST", f"{self.kc_origin}/realms/master/protocol/openid-connect/token",
                               form={"grant_type": "password", "client_id": "admin-cli", "username": "admin", "password": self.secrets["kc_admin"]})
        token = tok["access_token"]
        ids = {}
        for u in self.users:
            admin = f"{self.kc_origin}/admin/realms/{REALM}/users"
            found = self.https("GET", f"{admin}?username={u['name']}&exact=true", token=token)[1]
            if not found:
                self.https("POST", admin, body={"username": u["name"], "enabled": True, "emailVerified": True, "firstName": u["name"].capitalize(),
                                                 "lastName": "Dev", "email": f"{u['name']}@wk.invalid"}, token=token)
                found = self.https("GET", f"{admin}?username={u['name']}&exact=true", token=token)[1]
            uid = found[0]["id"]
            status = self.https("PUT", f"{admin}/{uid}/reset-password",
                                body={"type": "password", "value": self.secrets["users"][u["name"]], "temporary": False}, token=token)[0]
            if status >= 300:
                sys.exit(f"could not set the password for {u['name']} (HTTP {status})")
            ids[u["name"]] = uid
        return ids

    # ---- the steps --------------------------------------------------------------------------
    def up(self):
        self.prepare()
        self.ensure_image()
        say("starting PostgreSQL and Keycloak")
        self.compose("up", "-d", "--wait", "postgres", "keycloak-postgres", quiet=True)
        self.compose("up", "-d", "keycloak", quiet=True)
        self.wait(f"{self.issuer}/.well-known/openid-configuration", "Keycloak")
        say("migrating the database and creating the restricted application role")
        self.operator("migrate")
        self.sql(f"DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='wk_app') THEN CREATE ROLE wk_app LOGIN; END IF; END $$;\n"
                 f"ALTER ROLE wk_app PASSWORD '{self.secrets['db_app']}';\n" + (ROOT / "deploy/scripts/application-role.sql").read_text())
        say("creating Keycloak users and provisioning their memberships")
        ids = self.keycloak_users()
        for u in self.users:
            self.operator("provision", {"issuer": self.issuer, "subject": ids[u["name"]], "org": self.a.org, "user": u["name"],
                                        "product": self.a.product, "services": u["services"], "can_review": u["review"]})
        say("enabling native MCP sign-in for Claude Code, Cursor and VS Code")
        hosts = [x for h in TRUSTED_HOSTS for x in ("--trusted-host", h)]
        run(sys.executable, str(ROOT / "deploy/keycloak/enable-remote-mcp-signin.py"), "--keycloak", self.kc_origin, "--audience", self.app_origin,
            "--ca-file", str(self.state / "certs/ca.pem"), *hosts, env=dict(os.environ, KC_ADMIN_PASSWORD=self.secrets["kc_admin"]), quiet=True)
        say("starting the server and the HTTPS proxy")
        self.compose("up", "-d", "--wait", "app", quiet=True)
        self.compose("up", "-d", "caddy", quiet=True)
        self.wait(f"{self.app_origin}/health/ready", "the server", 90)
        self.summary()

    def summary(self):
        ca, mcp = self.state / "certs/ca.pem", f"{self.app_origin}/mcp/{self.a.product}"
        print(f"""
Stack is up.
  Portal:        {self.app_origin}
  MCP address:   {mcp}
  Keycloak:      {self.kc_origin}
  Private CA:    {ca}  (trust it in your OS/browser, and give Claude Code NODE_EXTRA_CA_CERTS={ca})
  Credentials:   {self.state / 'credentials.txt'}
  Users:         {', '.join(u['name'] + ' (' + ','.join(u['services']) + (', reviewer' if u['review'] else '') + ')' for u in self.users)}
Product '{self.a.product}'. Add a user later by running `up` again with another --user.""")

    def status(self):
        self.prepare_env_only()
        self.compose("ps")
        try:
            print("server /health/ready:", self.https("GET", f"{self.app_origin}/health/ready")[0])
        except Exception as e:
            print("server unreachable:", repr(e)[:100])

    def prepare_env_only(self):
        if not (self.state / ".env").exists():
            sys.exit(f"no stack state in {self.state}; run `up` first")

    def down(self):
        self.prepare_env_only()
        self.compose("down", *(["-v"] if self.a.volumes else []), quiet=True)
        say("stopped" + (" and database/Keycloak data removed" if self.a.volumes else "; data kept (use --volumes to delete it)"))


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("command", choices=["up", "down", "status"])
    p.add_argument("--state-dir", default="~/.local/share/workspace-knowledge/stack", help="certificates, secrets, credentials.txt (outside the repo)")
    p.add_argument("--domain", default="localhost", help="sso.DOMAIN and wk.DOMAIN; the default works on a Mac without DNS or hosts-file edits")
    p.add_argument("--kc-port", default="8443")
    p.add_argument("--app-port", default="8444")
    p.add_argument("--bind", default="127.0.0.1", help="host address for the published ports; 0.0.0.0 for a server other people reach")
    p.add_argument("--product", default="demo")
    p.add_argument("--org", default="local")
    p.add_argument("--user", action="append", metavar="NAME[:svc,svc[:review]]", help=f"repeatable; default {' '.join(DEFAULT_USERS)}")
    p.add_argument("--image", default="wk-server:local", help="built from deploy/Dockerfile when missing")
    p.add_argument("--tls-cert", help="PEM chain for sso.DOMAIN and wk.DOMAIN (e.g. from your CA); with --tls-key. Otherwise a private CA is created")
    p.add_argument("--tls-key")
    p.add_argument("--ca-file", help="CA bundle for --tls-cert when it is a private CA (default: the cert itself)")
    p.add_argument("--volumes", action="store_true", help="down: also delete database and Keycloak data")
    args = p.parse_args()
    if bool(args.tls_cert) != bool(args.tls_key):
        p.error("--tls-cert and --tls-key go together")
    getattr(Stack(args), args.command)()


if __name__ == "__main__":
    main()
