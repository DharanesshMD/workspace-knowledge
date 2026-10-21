from pathlib import Path
import re
from urllib.parse import unquote, urlsplit
import unittest


ROOT = Path(__file__).resolve().parents[2]


class ReleaseDocumentationTests(unittest.TestCase):
    def read(self, relative_path):
        return (ROOT / relative_path).read_text(encoding="utf-8")

    def test_readme_identifies_published_private_alpha_not_production_release(self):
        readme = self.read("README.md").lower()
        self.assertIn("v0.1.0-alpha.3", readme)
        self.assertIn("private source-only prerelease", readme)
        self.assertIn("not production-certified", readme)
        self.assertNotIn("not yet published", readme)

    def test_changelog_does_not_claim_no_release_exists(self):
        changelog = self.read("CHANGELOG.md").lower()
        self.assertIn("v0.1.0-alpha.3", changelog)
        self.assertNotIn("no tag or release has been published", changelog)

    def test_security_policy_distinguishes_prerelease_from_supported_release(self):
        security = self.read("SECURITY.md").lower()
        self.assertIn("v0.1.0-alpha.3", security)
        self.assertIn("not a production-supported release", security)

    def test_old_alpha_packet_cannot_direct_evaluators_to_publish_alpha_one(self):
        packet = self.read("docs/operations/public-alpha.md").lower()
        self.assertIn("superseded", packet)
        self.assertIn("v0.1.0-alpha.3", packet)
        self.assertNotIn("proposed label: **v0.1.0-alpha.1**", packet)
        self.assertNotIn("git clone --branch v0.1.0-alpha.1", packet)

    def test_release_checklist_does_not_mark_private_alpha_unpublished(self):
        checklist = self.read("docs/operations/v0.1-release.md").lower()
        self.assertIn("v0.1.0-alpha.3", checklist)
        self.assertIn("production readiness", checklist)
        self.assertNotIn("it is not a published release", checklist)

    def test_install_guide_points_to_current_prerelease_without_overstating_support(self):
        install = self.read("docs/operations/install.md").lower()
        self.assertIn("v0.1.0-alpha.3", install)
        self.assertNotIn("v0.1.0-alpha.1", install)
        self.assertIn("production", install)

    def test_local_markdown_links_in_release_pages_resolve(self):
        pages = (
            "README.md",
            "CHANGELOG.md",
            "SECURITY.md",
            "docs/operations/install.md",
            "docs/operations/public-alpha.md",
            "docs/operations/v0.1-release.md",
            "docs/operations/production-readiness.md",
            "docs/operations/oidc-keycloak.md",
            "docs/operations/quick-start-keycloak.md",
            "docs/adapters/setup.md",
            "docs/architecture/remote-mcp.md",
            "docs/reviews/2026-10-19-remote-mcp-signin-results.md",
            "docs/architecture/completion-matrix.md",
            "docs/reviews/2026-10-19-privacy-acceptance.md",
            "docs/reviews/2026-10-19-summary-quality-comparison.md",
        )
        link_pattern = re.compile(r"(?<!!)\[[^\]]*\]\(([^)]+)\)")
        for relative_page in pages:
            page = ROOT / relative_page
            for target in link_pattern.findall(page.read_text(encoding="utf-8")):
                parsed = urlsplit(target)
                if parsed.scheme or parsed.netloc or not parsed.path:
                    continue
                linked_path = (page.parent / unquote(parsed.path)).resolve()
                with self.subTest(page=relative_page, target=target):
                    self.assertTrue(linked_path.exists(), f"Broken local link: {relative_page} -> {target}")


if __name__ == "__main__":
    unittest.main()
