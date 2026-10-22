import contextlib
import importlib.util
import io
import json
import os
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[2] / "deploy/keycloak/enable-remote-mcp-signin.py"
spec = importlib.util.spec_from_file_location("enable_signin", SCRIPT)
enable_signin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(enable_signin)

POLICY = "org.keycloak.services.clientregistration.policy.ClientRegistrationPolicy"


class FakeKeycloak:
    """Just enough of the Keycloak admin API, kept in memory."""

    def __init__(self):
        self.scopes = [{"id": "s-roles", "name": "roles"}, {"id": "s-basic", "name": "basic"},
                       {"id": "s-service_account", "name": "service_account", "attributes": {"include.in.openid.provider.metadata": "true"}}]
        self.defaults = ["roles", "basic"]
        self.policy = {"id": "p1", "providerId": "trusted-hosts", "subType": "anonymous",
                       "config": {"host-sending-registration-request-must-match": ["true"], "trusted-hosts": [], "client-uris-must-match": ["true"]}}
        self.scopes_policy = {"id": "p2", "providerId": "allowed-client-templates", "subType": "anonymous", "config": {"allow-default-scopes": ["true"]}}
        self.log = []
        self.lifespan = 300
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, body=None, status=200):
                raw = json.dumps(body).encode() if body is not None else b""
                self.send_response(status)
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def handle_any(self, method):
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                path = self.path.split("?")[0]
                fake.log.append((method, path))
                if path.endswith("/openid-connect/token"):
                    return self.reply({"access_token": "t"} if b"password=pw" in body else {"error": "bad"}, 200 if b"password=pw" in body else 401)
                if self.headers.get("Authorization") != "Bearer t":
                    return self.reply(None, 401)
                if path.endswith("/admin/realms/wk") and method == "GET":
                    return self.reply({"realm": "wk", "accessTokenLifespan": fake.lifespan})
                if path.endswith("/client-scopes") and method == "GET":
                    return self.reply(fake.scopes)
                if path.endswith("/client-scopes") and method == "POST":
                    scope = json.loads(body)
                    scope["id"] = "s-" + scope["name"]
                    fake.scopes.append(scope)
                    return self.reply(None, 201)
                if path.endswith("/default-default-client-scopes") and method == "GET":
                    return self.reply([{"id": "s-" + n, "name": n} for n in fake.defaults])
                if "/client-scopes/" in path and method == "PUT":
                    update = json.loads(body)
                    fake.scopes[[x["id"] for x in fake.scopes].index(update["id"])] = update
                    return self.reply(None, 204)
                if "/default-default-client-scopes/" in path:
                    name = next(s["name"] for s in fake.scopes if s["id"] == path.rsplit("/", 1)[1])
                    if method == "PUT":
                        fake.defaults.append(name)
                    else:
                        fake.defaults.remove(name)
                    return self.reply(None, 204)
                if path.endswith("/components") and method == "GET":
                    return self.reply([fake.policy, fake.scopes_policy])
                if "/components/" in path and method == "PUT":
                    update = json.loads(body)
                    if update["id"] == "p2":
                        fake.scopes_policy = update
                    else:
                        fake.policy = update
                    return self.reply(None, 204)
                return self.reply(None, 404)

            do_GET = lambda self: self.handle_any("GET")
            do_POST = lambda self: self.handle_any("POST")
            do_PUT = lambda self: self.handle_any("PUT")
            do_DELETE = lambda self: self.handle_any("DELETE")

        self.server = HTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class EnableRemoteMcpSigninTests(unittest.TestCase):
    def setUp(self):
        self.fake = FakeKeycloak()
        self.addCleanup(self.fake.close)

    def run_script(self, *extra, password="pw"):
        args = ["--keycloak", self.fake.url, "--audience", "https://knowledge.example", *extra]
        out = io.StringIO()
        with mock.patch.dict(os.environ, {"KC_ADMIN_PASSWORD": password}), contextlib.redirect_stdout(out):
            self.last_output = out
            return enable_signin.main(args)

    def test_realm_gets_the_audience_scope_without_roles_and_a_loopback_only_policy(self):
        self.run_script()
        self.assertEqual(sorted(self.fake.defaults), ["basic", "wk-mcp-resource"])
        scope = next(s for s in self.fake.scopes if s["name"] == "wk-mcp-resource")
        mapper = scope["protocolMappers"][0]
        self.assertEqual(mapper["protocolMapper"], "oidc-audience-mapper")
        self.assertEqual(mapper["config"]["included.custom.audience"], "https://knowledge.example")
        self.assertEqual(mapper["config"]["access.token.claim"], "true")
        self.assertEqual(mapper["config"]["id.token.claim"], "false")
        self.assertEqual(self.fake.policy["config"], {"trusted-hosts": ["localhost", "127.0.0.1"],
                                                      "host-sending-registration-request-must-match": ["false"],
                                                      "client-uris-must-match": ["true"]})
        # VS Code asks for every scope the provider lists; Keycloak refuses these two for a normal client.
        hidden = {x["name"]: x.get("attributes", {}).get("include.in.openid.provider.metadata") for x in self.fake.scopes}
        self.assertEqual((hidden["roles"], hidden["service_account"]), ("false", "false"))
        self.assertIsNone(hidden["basic"], "other scopes are left alone")
        # A client that asks for the plain `openid` scope when registering must not be refused for it.
        self.assertEqual(self.fake.scopes_policy["config"], {"allow-default-scopes": ["true"], "allowed-client-scopes": ["openid"]})

    def test_it_warns_about_an_access_token_lifespan_that_sends_vs_code_into_a_refresh_burst(self):
        self.run_script()
        self.assertIn("burst of refreshes", self.last_output.getvalue())
        self.fake.lifespan = 600
        self.run_script()
        self.assertNotIn("burst of refreshes", self.last_output.getvalue())

    def test_running_twice_changes_nothing_more(self):
        self.run_script()
        before = len(self.fake.log)
        self.run_script()
        writes = [m for m, _ in self.fake.log[before:] if m in ("POST", "PUT", "DELETE") and not _.endswith("/token")]
        self.assertEqual(writes, [], self.fake.log[before:])

    def test_a_provider_that_returns_multivalued_settings_in_another_order_is_not_rewritten(self):
        self.run_script("--trusted-host", "localhost", "--trusted-host", "127.0.0.1")
        # Keycloak keeps multivalued attributes as an unordered set; a database may hand them back reversed.
        self.fake.policy["config"]["trusted-hosts"].reverse()
        before = len(self.fake.log)
        self.run_script("--trusted-host", "localhost", "--trusted-host", "127.0.0.1")
        writes = [(m, p) for m, p in self.fake.log[before:] if m in ("POST", "PUT", "DELETE") and not p.endswith("/token")]
        self.assertEqual(writes, [])

    def test_trusted_hosts_and_source_check_are_configurable(self):
        self.run_script("--trusted-host", "localhost", "--trusted-host", "10.0.0.5", "--require-trusted-source")
        config = self.fake.policy["config"]
        self.assertEqual(config["trusted-hosts"], ["localhost", "10.0.0.5"])
        self.assertEqual(config["host-sending-registration-request-must-match"], ["true"])

    def test_wrong_password_or_a_conflicting_existing_scope_stops_without_changes(self):
        with self.assertRaises(SystemExit):
            self.run_script(password="wrong")
        self.assertEqual(self.fake.defaults, ["roles", "basic"])
        self.fake.scopes.append({"id": "s-wk-mcp-resource", "name": "wk-mcp-resource", "protocolMappers": [
            {"protocolMapper": "oidc-audience-mapper", "config": {"included.custom.audience": "https://other.example"}}]})
        with self.assertRaises(SystemExit):
            self.run_script()
        self.assertEqual(self.fake.defaults, ["roles", "basic"])

    def test_inputs_are_refused_before_any_request(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(SystemExit):
                enable_signin.main(["--keycloak", self.fake.url, "--audience", "https://knowledge.example"])
        for audience in ("http://knowledge.example", "https://knowledge.example/", "https://knowledge.example/mcp", "knowledge.example"):
            with mock.patch.dict(os.environ, {"KC_ADMIN_PASSWORD": "pw"}):
                with self.assertRaises(SystemExit):
                    enable_signin.main(["--keycloak", self.fake.url, "--audience", audience])
        for url in ("http://sso.example", "https://sso.example/realms/wk", "ftp://sso.example"):
            with mock.patch.dict(os.environ, {"KC_ADMIN_PASSWORD": "pw"}):
                with self.assertRaises(SystemExit):
                    enable_signin.main(["--keycloak", url, "--audience", "https://knowledge.example"])
        self.assertEqual(self.fake.log, [])


if __name__ == "__main__":
    unittest.main()
