"""Exercise source installation/removal in a disposable user prefix (POSIX)."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


@unittest.skipIf(os.name == "nt", "POSIX source installer; Windows uses manual installation")
class SourceInstallTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wk-install-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "source checkout"
        self.prefix = self.base / "user prefix"
        self.home = self.base / "user home"
        self.home.mkdir()
        scripts = self.root / "deploy/scripts"
        scripts.mkdir(parents=True)
        repository = Path(__file__).resolve().parents[2]
        for name in ("install-source.sh", "uninstall.sh"):
            shutil.copy2(repository / "deploy/scripts" / name, scripts / name)
        # Compilation is the expensive dependency. Real shell/install/removal run
        # unchanged; the build fixture emits only binaries for requested packages.
        build = self.root / "scripts/cargo"
        build.parent.mkdir()
        build.write_text("""#!/usr/bin/env python3
import pathlib, sys
packages = {'wk-cli': 'wk', 'wk-agent': 'wk-agent', 'wk-environments': 'wk-env',
            'wk-server': 'wk-server', 'wk-mcp': 'wk-mcp'}
out = pathlib.Path('target/release')
out.mkdir(parents=True, exist_ok=True)
for index, arg in enumerate(sys.argv[1:], 1):
    if arg == '-p':
        binary = packages[sys.argv[index + 1]]
        (out / binary).write_text('#!/bin/sh\\nprintf "%s\\\\n" ' + binary + '\\n')
""", encoding="utf-8")
        build.chmod(0o755)
        # The old installer uses system Cargo; route only that dependency to the
        # same fixture so a missing executable is a regression, not a build error.
        self.env = dict(os.environ, HOME=str(self.home), WK_PREFIX=str(self.prefix),
                        PATH=str(build.parent) + os.pathsep + os.environ["PATH"])

    def install(self, cwd):
        return subprocess.run(["sh", str(self.root / "deploy/scripts/install-source.sh")],
                              cwd=cwd, env=self.env, check=True, capture_output=True, text=True)

    def test_clean_install_prints_exact_path_and_keeps_setup_opt_in(self):
        result = self.install(self.root)
        self.assertIn(f"Add the install directory ({self.prefix}/bin) to PATH", result.stdout)
        self.assertIn("No capture consent, hooks or startup services were enabled.", result.stdout)
        self.assertEqual(sorted(path.name for path in self.prefix.iterdir()), ["bin"])
        self.assertEqual(sorted(path.name for path in (self.prefix / "bin").iterdir()),
                         ["wk", "wk-agent", "wk-env", "wk-mcp", "wk-server"])
        self.assertEqual(list(self.home.iterdir()), [])

    def test_installs_every_user_command_including_mcp(self):
        self.install(self.root)
        for binary in ("wk", "wk-agent", "wk-env", "wk-server", "wk-mcp"):
            self.assertTrue(os.access(self.prefix / "bin" / binary, os.X_OK), binary)
            result = subprocess.run([str(self.prefix / "bin" / binary)],
                                    check=True, capture_output=True, text=True)
            self.assertEqual(result.stdout.strip(), binary)

    def test_install_resolves_checkout_from_another_directory(self):
        self.install(self.base)
        self.assertTrue(os.access(self.prefix / "bin/wk", os.X_OK))
        self.assertFalse((self.base / "target").exists())

    def test_uninstall_removes_mcp_and_preserves_unrelated_files(self):
        (self.prefix / "bin").mkdir(parents=True)
        for binary in ("wk", "wk-agent", "wk-env", "wk-server", "wk-mcp", "unrelated"):
            (self.prefix / "bin" / binary).write_text("fixture", encoding="utf-8")
        state = self.prefix / "private-state"
        state.write_text("preserved", encoding="utf-8")
        subprocess.run(["sh", str(self.root / "deploy/scripts/uninstall.sh")],
                       env=self.env, check=True, capture_output=True)
        self.assertEqual(sorted(p.name for p in (self.prefix / "bin").iterdir()), ["unrelated"])
        self.assertEqual(state.read_text(encoding="utf-8"), "preserved")

    def test_uninstall_and_same_revision_reinstall_restores_exact_binary_hashes(self):
        self.install(self.root)
        binaries = ("wk", "wk-agent", "wk-env", "wk-server", "wk-mcp")

        def hashes():
            return {
                binary: hashlib.sha256((self.prefix / "bin" / binary).read_bytes()).hexdigest()
                for binary in binaries
            }

        original = hashes()
        subprocess.run(["sh", str(self.root / "deploy/scripts/uninstall.sh")],
                       env=self.env, check=True, capture_output=True)
        self.install(self.root)
        self.assertEqual(hashes(), original)


if __name__ == "__main__":
    unittest.main()
