import os
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class CodeContextHostTests(unittest.TestCase):
    @unittest.skipIf(os.name == "nt", "fixture executable uses a POSIX shebang")
    def test_guarded_corruption_is_not_reported_as_unchanged(self):
        script = Path(__file__).resolve().parents[1] / "code-context-host.py"
        with tempfile.TemporaryDirectory() as directory:
            client = Path(directory) / "claude"
            client.write_text("#!" + sys.executable + "\n" + '''
import json, os, sys
from pathlib import Path
import urllib.request
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
if sys.argv[1:] == ['--version']:
    print('2.1.296 (synthetic adversarial client)')
    raise SystemExit(0)
settings = Path(sys.argv[sys.argv.index('--settings') + 1])
guarded = bool(json.loads(settings.read_text()).get('hooks'))
for _ in range(3):
    request = urllib.request.Request(os.environ['ANTHROPIC_BASE_URL'] + '/v1/messages', data=b'{}')
    with opener.open(request, timeout=5) as response:
        response.read()
if guarded:
    (settings.parent / 'hook-invoked').write_text('invoked')
Path('retry.txt').write_text('CORRUPTED\\n' if guarded else 'after confirmation\\n')
print('{}')
''')
            client.chmod(0o700)
            result = subprocess.run(
                [sys.executable, str(script), "--claude", str(client), "--agent", sys.executable],
                text=True, capture_output=True, timeout=20,
            )
            self.assertEqual(result.returncode, 1, "Corrupt guarded writes must fail acceptance")
            self.assertNotIn('"passed": true', result.stdout)
            records = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
            self.assertEqual([record['case'] for record in records], ['baseline', 'missing_enrollment'])
            self.assertTrue(records[1]['hook_invoked'])
            self.assertTrue(records[1]['target_changed'])
            self.assertEqual(records[1]['model_requests'], 3)

    @unittest.skipIf(os.name == "nt", "fixture executable uses a POSIX shebang")
    def test_unsupported_client_is_refused_before_any_model_or_edit_flow(self):
        script = Path(__file__).resolve().parents[1] / "code-context-host.py"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / "client-invoked"
            client = root / "claude"
            client.write_text(
                "#!" + sys.executable + "\nimport sys\nfrom pathlib import Path\n"
                "if sys.argv[1:] == ['--version']:\n    print('2.1.15 (synthetic)')\n"
                "else:\n    Path(" + repr(str(marker)) + ").write_text('invoked')\n"
            )
            client.chmod(0o700)
            result = subprocess.run(
                [sys.executable, str(script), "--claude", str(client), "--agent", sys.executable],
                text=True, capture_output=True, timeout=10,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("2.1.295", result.stderr)
            self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()
