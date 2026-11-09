#!/usr/bin/env python3
"""Bounded local-demo acceptance: fresh HOME/prefix, real tools/HTTPS, disposable developer.

Requires the existing local `wk-stack` evaluation stack and its operator state. Briefly stops and
restarts its app container for an outage check. Only the disposable account/token/card are changed;
normal client configuration and database volumes are preserved. This is not a fresh OS account.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--stack-dir", type=Path, default=Path.home() / ".local/share/workspace-knowledge/stack")
    p.add_argument("--output-dir", type=Path, help="parent for a new private acceptance directory")
    args = p.parse_args()
    spec = importlib.util.spec_from_file_location("wk_stack_verify", ROOT / "deploy/stack/verify.py")
    verify = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(verify)
    base = Path(tempfile.mkdtemp(prefix="wk-clean-user-", dir=args.output_dir)).resolve()
    base.chmod(0o700)
    home, prefix = base / "fresh user home", base / "user tools"
    home.mkdir(mode=0o700)
    env = dict(os.environ, HOME=str(home), CODEX_HOME=str(home / ".codex"), WK_PREFIX=str(prefix),
               CARGO_BUILD_JOBS="2", CARGO_INCREMENTAL="0")
    for key in ("WK_TOKEN", "WK_ACCESS_TOKEN", "WK_DEV_TOKEN", "WK_SERVER_CA_FILE", "WK_MCP_URL"):
        env.pop(key, None)
    env["PATH"] = str(prefix / "bin") + os.pathsep + env["PATH"]
    passed = []

    def ok(label):
        passed.append(label)
        print("PASS: " + label, flush=True)

    def run(command, **kwargs):
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=900, **kwargs)
        if result.returncode:
            raise AssertionError("acceptance command failed: " + str(command[0]))
        return result

    def install():
        result = run(["sh", str(ROOT / "deploy/scripts/install-source.sh")], cwd=str(base))
        (base / "install.log").write_text(result.stdout + result.stderr)

    install()
    binaries = ["wk", "wk-agent", "wk-env", "wk-server", "wk-mcp", "wk-connect"]
    hashes = {name: hashlib.sha256((prefix / "bin" / name).read_bytes()).hexdigest() for name in binaries}
    run([str(prefix / "bin/wk-connect"), "--help"])
    ok("real source installer works from another directory into a fresh prefix (six commands)")

    codex_config = home / ".codex/config.toml"
    codex_config.parent.mkdir(mode=0o700)
    codex_config.write_text('# preserved user setting\npersonality = "pragmatic"\n[mcp_servers.existing]\ncommand = "/usr/bin/true"\n')
    ag_config = home / ".gemini/config/mcp_config.json"
    ag_config.parent.mkdir(parents=True)
    ag_original = {"mcpServers": {"existing": {"command": "/usr/bin/true"}}, "setting": "preserved"}
    ag_config.write_text(json.dumps(ag_original))

    origin = "https://wk.localhost:8444"
    ca = str(args.stack_dir / "certs/ca.pem")
    private = json.loads((args.stack_dir / "secrets.json").read_text())
    browser, me = verify.portal_sign_in(ca, origin, "owner", private["users"]["owner"])
    headers = {"Origin": origin, "X-CSRF-Token": me["csrf_token"], "Content-Type": "application/json"}
    api = origin + "/v1/admin/products/demo/users"
    user = "onboard-" + secrets.token_hex(5)
    created, card_id, app_stopped = False, None, False

    def post(url, body):
        status, text, _ = browser.call(url, json.dumps(body).encode(), headers)
        assert status in (200, 201), "operator request failed: HTTP " + str(status)
        return json.loads(text)

    def setup(client, action="connect", token=None, expect=True, ca_path=None):
        cmd = [str(prefix / "bin/wk-connect"), action, "--client", client]
        if action == "connect":
            cmd += ["--url", origin + "/mcp/demo", "--token-stdin", "--ca-file", str(ca_path or handoff_ca)]
        result = subprocess.run(cmd, input=(token or "") + "\n", env=env,
                                capture_output=True, text=True, timeout=45)
        assert (result.returncode == 0) == expect, "unexpected onboarding result: " + result.stderr
        if token:
            assert token not in result.stdout + result.stderr, "credential was printed"
        return result

    def entry(client):
        return json.loads((home / ".local/share/workspace-knowledge/clients" / client /
                           "workspace-knowledge/connection.json").read_text())["entry"]

    def tool(client, name, arguments):
        connection = entry(client)
        request = {"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": arguments}}
        process_env = dict(env, WK_TOKEN="invalid-inherited-token")
        process_env.update(connection.get("env", {}))
        result = subprocess.run([connection["command"]] + connection["args"], input=json.dumps(request) + "\n",
                                env=process_env, capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, "native proxy failed"
        reply = json.loads(result.stdout.strip())
        assert "error" not in reply and not reply["result"].get("isError"), "MCP tool failed"
        return json.loads(reply["result"]["content"][0]["text"])

    handoff_ca = base / "provided-ca.pem"
    handoff_ca.write_bytes(Path(ca).read_bytes())
    try:
        made = post(api, {"user": user, "services": ["backend"], "can_review": False, "days": 1})
        created = True
        token = made["token"]
        ok("a new scoped developer receives an actual server-issued token")
        bad_ca = base / "bad-ca.pem"
        bad_ca.write_text("not a CA certificate")
        before = codex_config.read_bytes()
        setup("codex", token=token, expect=False, ca_path=bad_ca)
        assert codex_config.read_bytes() == before
        ok("invalid trust bundle fails before changing IDE settings")

        for client in ("codex", "antigravity"):
            setup(client, token=token)
            setup(client, "check")
            connection = entry(client)
            assert connection["command"] == str(prefix / "bin/wk-mcp")
            assert "--token-file" in connection["args"]
            path = codex_config if client == "codex" else ag_config
            assert token not in path.read_text() and "existing" in path.read_text()
        handoff_ca.unlink()
        for client in ("codex", "antigravity"):
            setup(client, "check")
        ok("real Codex CLI and Antigravity configuration preserve other servers, without launchers or embedded tokens")
        ok("fresh proxy processes work after the original CA handoff file is removed")

        marker = "wkonboarding" + secrets.token_hex(8)
        shared = tool("codex", "knowledge.share_session", {"service": "backend", "title": marker,
                      "intent": "Synthetic clean-user acceptance", "rationale": "Confirm installed client onboarding",
                      "decisions": ["Use the installed proxy"], "files": ["backend/onboarding.py"]})
        card_id = str(uuid.UUID(shared["card_id"]))
        assert shared["published"] is True
        found = tool("antigravity", "knowledge.team_work", {"developer": user, "query": marker})
        assert found["entries"][0]["card_id"] == card_id and found["entries"][0]["published"] is True
        assert tool("codex", "knowledge.team_work", {"service": "frontend"})["entries"] == []
        ok("installed clients share/retrieve as the new developer and keep the backend service boundary")

        # The local demonstration app only: preserve all Docker volumes and restart even on failure.
        labels = json.loads(run(["docker", "inspect", "wk-stack-app-1", "--format", "{{json .Config.Labels}}"] ).stdout)
        assert labels.get("com.docker.compose.project") == "wk-stack" and labels.get("com.docker.compose.service") == "app"
        run(["docker", "stop", "wk-stack-app-1"])
        app_stopped = True
        for client in ("codex", "antigravity"):
            setup(client, "check", expect=False)
        run(["docker", "start", "wk-stack-app-1"])
        app_stopped = False
        for _ in range(30):
            try:
                if verify.Browser(ca).call(origin + "/health/ready")[0] == 200:
                    break
            except Exception:
                pass
            time.sleep(1)
        for client in ("codex", "antigravity"):
            setup(client, "check")
        ok("server outage is reported and both configurations recover after restart")

        for label in ("revocation", "expiry"):
            if label == "revocation":
                replacement = post(api + "/" + user + "/token", {"days": 1})["token"]
            else:
                status, text, _ = browser.call(api)
                assert status == 200
                row = next(u for u in json.loads(text)["users"] if u["user"] == user)
                live = next(t for t in row["tokens"] if t.get("revoked_at") is None)
                token_id = str(uuid.UUID(live["id"]))
                sql = "UPDATE personal_tokens SET expires_at=now()-interval '1 second' WHERE id='" + token_id + "' AND user_id='" + user + "';"
                run(["docker", "exec", "-i", "wk-stack-postgres-1", "psql", "-X", "-v", "ON_ERROR_STOP=1",
                     "-U", "wk_owner", "-d", "workspace_knowledge"], input=sql)
                replacement = None
            for client in ("codex", "antigravity"):
                setup(client, "check", expect=False)
            if replacement is None:
                replacement = post(api + "/" + user + "/token", {"days": 1})["token"]
            for client in ("codex", "antigravity"):
                managed_args = entry(client)["args"]
                managed_ca = Path(managed_args[managed_args.index("--ca-file") + 1])
                setup(client, token=replacement, ca_path=managed_ca)
                setup(client, "check")
            ok(label + " is enforced; credential replacement restores both clients")

        for client in ("codex", "antigravity"):
            setup(client, "disconnect")
            setup(client, "disconnect")
            client_state = home / ".local/share/workspace-knowledge/clients" / client / "workspace-knowledge"
            assert not any(client_state.rglob("token"))
        assert json.loads(ag_config.read_text()) == ag_original
        assert "existing" in codex_config.read_text() and "workspace-knowledge" not in codex_config.read_text()
        ok("disconnect is repeatable, removes managed credentials and preserves unrelated IDE settings")
        marker_file = prefix / "bin/unrelated"
        marker_file.write_text("preserve")
        run(["sh", str(ROOT / "deploy/scripts/uninstall.sh")])
        assert marker_file.read_text() == "preserve" and all(not (prefix / "bin" / name).exists() for name in binaries)
        install()
        assert hashes == {name: hashlib.sha256((prefix / "bin" / name).read_bytes()).hexdigest() for name in binaries}
        ok("uninstall preserves unrelated files; same-source reinstall restores all six exact hashes")
    finally:
        if app_stopped:
            subprocess.run(["docker", "start", "wk-stack-app-1"], capture_output=True, timeout=30)
        if card_id:
            status, _, _ = browser.call(origin + "/v1/cards/" + card_id + "/withdraw",
                                       json.dumps({"product": "demo", "revision": 1}).encode(), headers)
            assert status == 200, "could not withdraw disposable acceptance card"
        if created:
            status, _, _ = browser.call(api + "/" + user, headers=headers, method="DELETE")
            assert status == 200, "could not remove disposable acceptance developer"
    ok("cleanup withdrew the synthetic card and removed the disposable developer")
    (base / "results.json").write_text(json.dumps({"passed": passed, "fresh_home": str(home),
                                                   "prefix": str(prefix), "test_user": user, "withdrawn_card": card_id,
                                                   "not_a_fresh_os_account": True}, indent=2))
    print("Acceptance evidence: " + str(base / "results.json"), flush=True)


if __name__ == "__main__":
    main()
