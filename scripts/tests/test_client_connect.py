"""Run the onboarding command against real files; only the network proxy is a fixture."""
import json
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "deploy/scripts/wk-connect.py"


@unittest.skipUnless(os.name == "posix", "POSIX onboarding")
class ClientConnectTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="wk-connect-")
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name).resolve()
        self.config = self.home / ".gemini/config/mcp_config.json"
        self.config.parent.mkdir(parents=True)
        self.original = {"mcpServers": {"existing": {"command": "keep-me"}}, "setting": 7}
        self.config.write_text(json.dumps(self.original))
        self.proxy = self.home / "wk-mcp"
        self.proxy.write_text('''#!/usr/bin/env python3
import json, pathlib, sys
token = pathlib.Path(sys.argv[sys.argv.index("--token-file") + 1]).read_text().strip()
for line in sys.stdin:
    request = json.loads(line)
    if token == "bad-token":
        reply = {"error": {"code": -32001, "message": "HTTP 401"}}
    else:
        reply = {"result": {"tools": [{"name": "knowledge.team_work"}], "seen_token": token}}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], **reply}), flush=True)
''')
        self.proxy.chmod(0o700)
        self.env = dict(os.environ, HOME=str(self.home), CODEX_HOME=str(self.home / ".codex"))

    def run_setup(self, action="connect", token="test-token", extra=(), client="antigravity"):
        command = [sys.executable, str(SCRIPT), action, "--client", client, "--binary", str(self.proxy)]
        if action == "connect":
            command += ["--url", "https://wk.example/mcp/demo", "--token-stdin"]
        return subprocess.run(command + list(extra), env=self.env, input=token + "\n",
                              capture_output=True, text=True, timeout=15)

    def test_connect_preserves_configuration_and_keeps_credentials_out_of_it(self):
        result = self.run_setup()
        self.assertEqual(result.returncode, 0, result.stderr)
        config = json.loads(self.config.read_text())
        self.assertEqual(config["setting"], 7)
        self.assertEqual(config["mcpServers"]["existing"], {"command": "keep-me"})
        entry = config["mcpServers"]["workspace-knowledge"]
        self.assertTrue(os.path.samefile(entry["command"], self.proxy))
        token_path = Path(entry["args"][entry["args"].index("--token-file") + 1])
        self.assertEqual(token_path.read_text().strip(), "test-token")
        self.assertEqual(token_path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(token_path.parent.stat().st_mode & 0o777, 0o700)
        self.assertNotIn("test-token", self.config.read_text() + result.stdout + result.stderr)
        self.assertEqual(self.run_setup("check").returncode, 0)

    def test_reconnect_and_failed_credentials_preserve_a_working_setup(self):
        self.assertEqual(self.run_setup().returncode, 0)
        self.assertEqual(self.run_setup(token="replacement-token").returncode, 0)
        before = self.config.read_bytes()
        result = self.run_setup(token="bad-token")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("bad-token", result.stdout + result.stderr)
        self.assertEqual(self.config.read_bytes(), before)
        self.assertEqual(self.run_setup("check").returncode, 0)

    def test_cached_endpoint_command_never_reads_a_replacement_endpoints_token(self):
        self.assertEqual(self.run_setup(token="endpoint-a-token").returncode, 0)
        old = json.loads(self.config.read_text())["mcpServers"]["workspace-knowledge"]
        result = self.run_setup(token="endpoint-b-token", extra=("--url", "https://other.example/mcp/demo"))
        self.assertEqual(result.returncode, 0, result.stderr)
        cached = subprocess.run([old["command"]] + old["args"], env=self.env,
                                input=json.dumps({"id": 1, "method": "tools/list"}) + "\n",
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(cached.returncode, 0, cached.stderr)
        self.assertEqual(json.loads(cached.stdout)["result"]["seen_token"], "endpoint-a-token")
        self.assertEqual(self.run_setup("disconnect").returncode, 0)
        old_token = Path(old["args"][old["args"].index("--token-file") + 1])
        self.assertFalse(old_token.exists(), "disconnect clears retired endpoint credentials too")

    def test_legacy_cached_endpoint_does_not_read_a_new_endpoints_token(self):
        self.assertEqual(self.run_setup(token="legacy-endpoint-token").returncode, 0)
        data = json.loads(self.config.read_text())
        old = data["mcpServers"]["workspace-knowledge"]
        state = self.home / ".local/share/workspace-knowledge/clients/antigravity/workspace-knowledge"
        legacy = state / "token"
        legacy.write_text("legacy-endpoint-token\n")
        legacy.chmod(0o600)
        old["args"][old["args"].index("--token-file") + 1] = str(legacy)
        self.config.write_text(json.dumps(data))
        (state / "connection.json").write_text(json.dumps({"url": "https://wk.example/mcp/demo", "entry": old}))
        result = self.run_setup(token="new-endpoint-token", extra=("--url", "https://other.example/mcp/demo"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(legacy.read_text().strip(), "legacy-endpoint-token")

    def test_disconnect_preserves_other_servers_and_refuses_user_modified_entry(self):
        self.assertEqual(self.run_setup().returncode, 0)
        config = json.loads(self.config.read_text())
        owned = config["mcpServers"]["workspace-knowledge"]
        token = Path(owned["args"][owned["args"].index("--token-file") + 1])
        config["mcpServers"]["workspace-knowledge"] = {"command": "user-modified"}
        self.config.write_text(json.dumps(config))
        self.assertNotEqual(self.run_setup("disconnect").returncode, 0)
        self.assertTrue(token.exists())
        config["mcpServers"]["workspace-knowledge"] = owned
        self.config.write_text(json.dumps(config))
        self.assertEqual(self.run_setup("disconnect").returncode, 0)
        self.assertEqual(json.loads(self.config.read_text()), self.original)
        self.assertFalse(token.exists())
        self.assertEqual(self.run_setup("disconnect").returncode, 0)

    def test_unowned_name_and_symlinked_configuration_are_not_overwritten(self):
        config = {"mcpServers": {"workspace-knowledge": {"command": "unowned"}}}
        self.config.write_text(json.dumps(config))
        self.assertNotEqual(self.run_setup().returncode, 0)
        self.assertEqual(json.loads(self.config.read_text()), config)
        target = self.home / "other.json"
        target.write_text(json.dumps(self.original))
        self.config.unlink()
        self.config.symlink_to(target)
        self.assertNotEqual(self.run_setup().returncode, 0)
        self.assertEqual(json.loads(target.read_text()), self.original)

    def test_disconnect_refuses_a_symlinked_credential_directory_before_changing_settings(self):
        self.assertEqual(self.run_setup().returncode, 0)
        before = self.config.read_bytes()
        entry = json.loads(before)["mcpServers"]["workspace-knowledge"]
        token = Path(entry["args"][entry["args"].index("--token-file") + 1])
        external = self.home / "unmanaged"
        external.mkdir()
        (external / "token").write_text("unmanaged-token")
        (token.parent.parent / ("endpoint-" + "f" * 64)).symlink_to(external, target_is_directory=True)
        self.assertNotEqual(self.run_setup("disconnect").returncode, 0)
        self.assertEqual(self.config.read_bytes(), before)
        self.assertTrue(token.exists())
        self.assertEqual((external / "token").read_text(), "unmanaged-token")

    def test_failed_final_state_write_rolls_back_the_ide_configuration(self):
        spec = importlib.util.spec_from_file_location("wk_connect", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        original_write = module.write_file

        def fail_manifest(path, content):
            if path.name == "connection.json":
                raise OSError("synthetic disk failure")
            original_write(path, content)

        args = [str(SCRIPT), "connect", "--client", "antigravity", "--binary", str(self.proxy),
                "--url", "https://wk.example/mcp/demo", "--token-stdin"]
        before = self.config.read_bytes()
        with mock.patch.dict(os.environ, self.env), mock.patch.object(sys, "argv", args), \
                mock.patch.object(sys, "stdin", io.StringIO("test-token\n")), \
                mock.patch.object(module, "write_file", fail_manifest), contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(OSError):
                module.main()
        self.assertEqual(self.config.read_bytes(), before)

    def test_oversized_stdin_is_refused_instead_of_truncating_a_valid_prefix(self):
        before = self.config.read_bytes()
        result = self.run_setup(token="test-token" + " " * 600)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.config.read_bytes(), before)

    def test_failed_final_write_preserves_an_intervening_ide_edit(self):
        spec = importlib.util.spec_from_file_location("wk_connect", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        original_write = module.write_file
        edited = []

        def fail_manifest(path, content):
            if path.name == "connection.json":
                data = json.loads(self.config.read_text())
                data["setting"] = "changed while connecting"
                edited.append(json.dumps(data).encode())
                self.config.write_bytes(edited[0])
                raise OSError("synthetic disk failure")
            original_write(path, content)

        args = [str(SCRIPT), "connect", "--client", "antigravity", "--binary", str(self.proxy),
                "--url", "https://wk.example/mcp/demo", "--token-stdin"]
        with mock.patch.dict(os.environ, self.env), mock.patch.object(sys, "argv", args), \
                mock.patch.object(sys, "stdin", io.StringIO("test-token\n")), \
                mock.patch.object(module, "write_file", fail_manifest), contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(OSError):
                module.main()
        self.assertEqual(self.config.read_bytes(), edited[0])

    def test_each_explicit_replacement_keeps_the_displaced_configuration(self):
        self.assertEqual(self.run_setup().returncode, 0)
        state = self.home / ".local/share/workspace-knowledge/clients/antigravity/workspace-knowledge"
        for version in (1, 2):
            data = json.loads(self.config.read_text())
            data["mcpServers"]["workspace-knowledge"] = {"command": "custom-" + str(version)}
            before = json.dumps(data).encode()
            self.config.write_bytes(before)
            result = self.run_setup(extra=("--replace",))
            self.assertEqual(result.returncode, 0, result.stderr)
            backups = list(state.glob("replaced-config-*.backup"))
            self.assertEqual(len(backups), version)
            self.assertIn(before, [p.read_bytes() for p in backups])
            self.assertTrue(all(p.stat().st_mode & 0o777 == 0o600 for p in backups))

    def test_codex_visible_and_hidden_tool_policies_are_not_erased(self):
        codex = shutil.which("codex")
        if not codex:
            self.skipTest("requires the real Codex configuration CLI")
        result = self.run_setup(client="codex", extra=("--codex", codex))
        self.assertEqual(result.returncode, 0, result.stderr)
        config = self.home / ".codex/config.toml"
        original = config.read_text()
        section = "[mcp_servers.workspace-knowledge]\n"
        self.assertIn(section, original)
        for policy in ('disabled_tools = ["knowledge.share_session"]\n',
                       'tool_timeout_sec = 45\n',
                       'destructive_enabled = false\n',
                       '[mcp_servers.workspace-knowledge.tools."knowledge.share_session"]\napproval_mode = "prompt"\n'):
            with self.subTest(policy=policy):
                changed = (original + "\n" + policy if policy.startswith("[") else
                           original.replace(section, section + policy))
                config.write_text(changed)
                result = self.run_setup(client="codex", extra=("--codex", codex))
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(config.read_text(), changed)
                self.assertNotEqual(self.run_setup("disconnect", client="codex", extra=("--codex", codex)).returncode, 0)
                self.assertEqual(config.read_text(), changed)

    def test_codex_native_rewrite_cannot_drop_unrelated_settings(self):
        codex = shutil.which("codex")
        if not codex:
            self.skipTest("requires the real Codex configuration CLI")
        config = self.home / ".codex/config.toml"
        config.parent.mkdir()
        before = '[mcp_servers.existing]\ncommand = "/usr/bin/true"\ndestructive_enabled = false\n'
        config.write_text(before)
        result = self.run_setup(client="codex", extra=("--codex", codex))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(config.read_text(), before)

    def test_concurrent_codex_edit_during_staging_is_preserved_on_rejection(self):
        codex = shutil.which("codex")
        if not codex:
            self.skipTest("requires the real Codex configuration CLI")
        self.assertEqual(self.run_setup(client="codex", extra=("--codex", codex)).returncode, 0)
        config = self.home / ".codex/config.toml"
        original_config = config.read_text()
        state = self.home / ".local/share/workspace-knowledge/clients/codex/workspace-knowledge"
        token_args = json.loads((state / "connection.json").read_text())["entry"]["args"]
        original_token_file = Path(token_args[token_args.index("--token-file") + 1])
        original_token = original_token_file.read_bytes()
        spec = importlib.util.spec_from_file_location("wk_connect", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        original_command = module.command
        changed_config = original_config + "\n[features]\ncollaboration_modes = true\n"

        def edit_during_stage(*args, **kwargs):
            result = original_command(*args, **kwargs)
            config.write_text(changed_config)
            return result

        args = [str(SCRIPT), "connect", "--client", "codex", "--codex", codex,
                "--binary", str(self.proxy), "--url", "https://other.example/mcp/demo", "--token-stdin"]
        with mock.patch.dict(os.environ, self.env), mock.patch.object(sys, "argv", args), \
                mock.patch.object(sys, "stdin", io.StringIO("replacement-token\n")), \
                mock.patch.object(module, "command", edit_during_stage), contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(ValueError):
                module.main()
        self.assertEqual(config.read_text(), changed_config)
        self.assertEqual(original_token_file.read_bytes(), original_token)


if __name__ == "__main__":
    unittest.main()
