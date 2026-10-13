import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


class SecretHistoryTests(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("WK_TEST_GITLEAKS_BIN"), "requires explicit Gitleaks executable")
    def test_credential_added_only_by_merge_is_rejected_by_ci_scan_options(self):
        workflow = Path(__file__).resolve().parents[2] / ".github/workflows/security.yml"
        match = re.search(r'--log-opts="([^"]+)"', workflow.read_text())
        self.assertIsNotNone(match, "History step must specify its traversal options")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.run(
                    ["git", "-C", str(root), "-c", "user.name=Synthetic Fixture", "-c", "user.email=fixture@example.invalid", *args],
                    text=True, capture_output=True, check=True,
                )
            git("init", "--initial-branch=main")
            config = root / ".gitleaks.toml"
            config.write_bytes((workflow.parents[2] / ".gitleaks.toml").read_bytes())
            (root / "base.txt").write_text("synthetic\n")
            git("add", "."); git("commit", "-m", "base")
            git("checkout", "-b", "side")
            (root / "side.txt").write_text("side\n")
            git("add", "."); git("commit", "-m", "side")
            git("checkout", "main")
            (root / "main.txt").write_text("main\n")
            git("add", "."); git("commit", "-m", "main")
            git("merge", "--no-ff", "--no-commit", "side")
            target = root / "crates/knowledge/tests/personal.rs"
            target.parent.mkdir(parents=True)
            # Stable nonfunctional fixture: random hex can miss the scanner's
            # entropy/stopword filters, making the traversal control flaky.
            canary = hashlib.sha256(b"workspace-knowledge-merge-fixture-v1").hexdigest()[:48]
            target.write_text("API_KEY=" + canary + "\n")
            git("add", "."); git("commit", "-m", "synthetic merge-only canary")
            report = root / "report.json"
            result = subprocess.run(
                [os.environ["WK_TEST_GITLEAKS_BIN"], "git", "--no-banner", "--redact=100", "--config", str(config), "--log-opts=" + match.group(1), "--report-format=json", "--report-path=" + str(report), str(root)],
                text=True, capture_output=True, timeout=20,
            )
            self.assertEqual(result.returncode, 1, "Merge-only credentials must fail the history gate")
            findings = json.loads(report.read_text())
            self.assertTrue(any(item["File"] == "crates/knowledge/tests/personal.rs" for item in findings))
            self.assertTrue(any(item["RuleID"] == "generic-api-key" for item in findings))
            self.assertTrue(all(item["Secret"] == "REDACTED" for item in findings))


if __name__ == "__main__":
    unittest.main()
