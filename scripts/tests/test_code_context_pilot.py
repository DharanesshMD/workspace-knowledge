import json
import os
import subprocess
import sys
from pathlib import Path
import unittest


class CodeContextPilotTests(unittest.TestCase):
    def test_missing_private_fixture_is_reported_unverified_without_network(self):
        script = Path(__file__).resolve().parents[1] / "code-context-pilot.py"
        env = dict(os.environ)
        env.pop("WK_CODE_CONTEXT_PILOT_FIXTURE", None)
        result = subprocess.run(
            [sys.executable, str(script)], capture_output=True, text=True, env=env, timeout=5
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            json.loads(result.stdout),
            {"status": "unverified", "reason": "WK_CODE_CONTEXT_PILOT_FIXTURE is not configured"},
        )


if __name__ == "__main__":
    unittest.main()
