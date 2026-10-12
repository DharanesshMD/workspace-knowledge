"""The bounded soak CLI must reach the owned image check without starting resources."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]

class HostedPilotTests(unittest.TestCase):
    def test_ten_minute_paced_soak_reaches_image_check(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            docker = directory / 'docker'
            docker.write_text('#!/bin/sh\n[ "$1 $2" = "image inspect" ] && echo owned-image-check >&2\nexit 1\n')
            docker.chmod(0o755)
            result = subprocess.run(['python3', str(ROOT / 'scripts/hosted-pilot.py'),
                '--duration', '600', '--cycle-pause-ms', '1000', '--output', str(directory / 'result.json')],
                env=dict(os.environ, PATH=str(directory)+os.pathsep+os.environ['PATH']),
                capture_output=True, text=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('owned-image-check', result.stderr)
            self.assertFalse((directory / 'result.json').exists())

if __name__ == '__main__':
    unittest.main()
