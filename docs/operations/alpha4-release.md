# v0.1.0-alpha.4: source-only evaluation draft

**Draft, not published or tagged.** The latest published prerelease remains `v0.1.0-alpha.3`. These notes describe the current evaluation work, including the schema-21 recovery fix. They are not approval to publish or a production-support claim.

The first evaluation workflow is a Linux Docker server and macOS developers using MCP to retrieve team knowledge and explicitly share screened summaries. Real model-driven Codex CLI and Antigravity IDE workflows passed locally; exact versions and surfaces are in the [client report](../reviews/2026-11-07-local-ide-acceptance.md). Automatic conversation capture remains separately experimental and version-pinned.

## Verification

The application baseline `49adaaf` passed all seven Foundation and both Dependency CI jobs across Linux, macOS and Windows, PostgreSQL, the web console, OpenBao and the amd64 server image. Local evidence includes 803 Rust tests, 58 Python checks without skips, 77 browser tests and a 12-check fresh-HOME onboarding lifecycle. See the [qualification report](../reviews/2026-11-08-release-qualification.md).

The subsequent recovery correction at `b419704` fixes current-schema backup/restore and adds its real drill to PostgreSQL CI. Local checks passed 15 encrypted-recovery checks and 85 private-CA Keycloak checks. The [local pilot/recovery report](../reviews/2026-11-09-local-pilot-recovery.md) records that candidate's remote result and exact limits. Select and verify the final release revision before creating a tag; neither a later documentation commit nor a moving branch inherits an exact-SHA CI result.

## Changes since alpha.3

- **Repeatable Codex/Antigravity setup.** The source installer includes five native commands and `wk-connect`. Its connect/check/disconnect commands validate authenticated MCP access before updating settings, preserve other servers and tool policies, retain private backups, and use a native proxy with private token/CA files. Python 3.11+ is required for the setup helper. See [onboarding](client-onboarding.md).
- **Ask about people, files and modules.** `knowledge.team_work` retrieves published work by developer, paths, service, keywords and time window. Results distinguish publication from implementation lifecycle. Shared summaries can name the files actually changed.
- **Personal MCP tokens and product administration.** Developers can use revocable, product-scoped tokens. Product admins can provision scoped token-only developers through the portal. The operator must enable personal tokens. Tokens do not authorize other products or the ordinary REST API.
- **Login and private-CA support.** Native MCP sign-in and collector login support private-CA Keycloak. The Connect page supplies agent setup instructions. Public-provider acceptance remains open.
- **Explicit sharing and publication policy.** Agents share when asked. Eligible assistant-written cards publish after screening by default; `WK_AUTO_PUBLISH_PRODUCTS=none` requires review instead. Existing collectors retain their recorded policy, and new collector setup can use `--no-auto-publish`. Rule drafts, partial captures and hook output do not gain automatic publication.
- **Folder approval and revocation.** The bridge's approval page requires a developer click. Linux receipts now use generation-aware filesystem handles; other Unix receipts include creation time. Legacy Unix inode-only approvals require fresh consent and collector reenrollment. Unsupported filesystems fail closed.
- **Recovery.** Backup/restore requires migrations 1–21, consistent with the version-2/schema-21 manifest. Restored access is quarantined, including personal MCP tokens. Both latest deletion journals must be replayed before granting access. CI now exercises actual encrypted recovery.
- **Maintenance and CI fixes.** Tokio 1.53.3 removes its recorded I/O-safety warning, and dependency CI denies unsoundness warnings. The latest recorded audit has zero vulnerability findings and four unmaintained-package notices. Windows fixture/path checks and PostgreSQL runner build bounds were corrected. Older search and summary improvements remain included.

## Installation and upgrade

Follow [install and deploy](install.md), then [client onboarding](client-onboarding.md). The source installer does not enable capture, hooks or startup services. No signed/notarized binaries are attached to this proposed source release.

Before upgrading an existing server, retain a trusted backup created with tools matching its schema. Stop the application, apply migrations 1–21 with the protected operator connection, and reapply `deploy/scripts/application-role.sql`. Start the application using the restricted runtime connection. Keep client/proxy traffic stopped until `/health/ready` succeeds, then resume traffic. Current backup/restore tools accept exactly schema 21; earlier snapshots require the [offline upgrade procedure](restore.md). Do not edit a manifest to bypass compatibility, and take a new snapshot after migration.

Review publication policy before onboarding: alpha.3 required review, while current eligible assistant-written cards can publish immediately. Reapprove legacy Unix folder bindings and reenroll affected collectors. Personal-token MCP proxy onboarding does not depend on local capture receipts.

## Evaluation limits

The [local pilot protocol](local-pilot.md) uses synthetic work, one Mac and local Docker infrastructure. A fresh HOME is not a new OS account or clean physical machine. Passing Windows build/tests does not qualify the POSIX onboarding helper or Windows capture. Native capture pins were not broadened to make client-drift checks pass.

Public-CA identity-provider acceptance, independent security review, a consented real-team target-server pilot, clean-machine installation and signed/notarized macOS distribution remain open. Local synthetic capacity measurements are not a production forecast. Audits, checksums and dependency inventories do not certify security or publisher identity. Read the [readiness ledger](production-readiness.md) before making support claims.

## Before any publication

1. Choose the release scope and exact revision; close the relevant gates and rerun its complete checks. Keep the repository private until the owner deliberately changes visibility.
2. Confirm a monitored private security-reporting channel and review retained Actions logs/artifacts, license notices and intentional history disclosures. Do not rewrite old tags or delete evidence as a shortcut.
3. Update README, SECURITY, changelog and these notes to the selected revision and actual evidence. Alpha.4 is still a draft; do not move Unreleased changes into a published heading yet.
4. Only after explicit release authorization, create a new prerelease tag and source-only release for that revision. Do not retag alpha.3. Verify the public view separately if the owner chooses to change visibility.
