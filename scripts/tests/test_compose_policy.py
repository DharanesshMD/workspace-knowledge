"""The documented operator opt-out must reach the application container."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import unittest


ROOT = Path(__file__).resolve().parents[2]


class ComposePolicyTests(unittest.TestCase):
    def test_publication_opt_out_reaches_the_container(self):
        if not shutil.which("docker"):
            self.skipTest("Docker Compose CLI is required")
        available = subprocess.run(
            ["docker", "compose", "version"], capture_output=True, timeout=15
        )
        if available.returncode:
            self.skipTest("Docker Compose CLI is required")
        env = dict(os.environ, WK_POSTGRES_PASSWORD="synthetic-only",
                   WK_DATABASE_URL="postgres://synthetic:synthetic@postgres/wk_test",
                   WK_OIDC_ISSUER="https://id.example.test", WK_OIDC_AUDIENCE="wk-api",
                   WK_OIDC_JWKS_URL="https://id.example.test/jwks",
                   WK_COLLECTOR_AUDIENCE="wk-collector", WK_COLLECTOR_CLIENT_ID="collector",
                   WK_BROWSER_CLIENT_ID="portal", WK_CONSOLE_ORIGIN="https://wk.example.test",
                   WK_AUTO_PUBLISH_PRODUCTS="none")
        result = subprocess.run(
            ["docker", "compose", "--env-file", os.devnull, "-f",
             str(ROOT / "deploy/compose/compose.yaml"), "config", "--format", "json"],
            env=env, capture_output=True, text=True, check=True, timeout=30,
        )
        environment = json.loads(result.stdout)["services"]["app"]["environment"]
        self.assertEqual(environment.get("WK_AUTO_PUBLISH_PRODUCTS"), "none")


if __name__ == "__main__":
    unittest.main()
