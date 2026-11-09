# Private integration qualification — 2026-11-09

This follow-up integrates the local IDE evaluation branch after a fresh review of the complete change from `main`. The preceding documentation candidate was `4e68b05`. The user authorized review, integration and a frozen private evaluation packet; this is not authorization for a public release or a hosted deployment.

## Review corrections

- Schema 22 permanently reserves historical principal names, including retained ownership left by accounts removed under schema 21. Removal retains the identity; creating a different account with that name is refused. The append-only migration backfills the reservation table, and the schema-21 upgrade test exercises actual old deletion behavior. Existing administrative function grants remain valid; the restricted runtime role has no direct reservation-table access.
- Personal-token creation holds the existing administration gate from authorization through insertion. Concurrent removal cannot leave a late-issued token that revives after membership is re-added.
- Team-work name resolution includes safe, authorized live activity even when the developer has no published card. Active-only text remains useful; service, product, tenant, device, expiry and content screening remain enforced.
- Managed client credentials have distinct paths per endpoint. Cached commands retain their original endpoint's credentials when the URL changes. Disconnect removes current, retired and legacy managed credentials and refuses symlinked endpoint directories before changing settings.

The ownership, issuance-race, activity-only and cached-endpoint regressions were observed failing before their fixes. Separate read-only reviewers found no remaining blockers in the corrected database and client paths. This is engineering review, not independent security certification.

## Fresh local verification

The results below qualify the initial schema-22 integration at `3e4a5f2`, before the dependency follow-up described below.

| Check | Result |
|---|---|
| Full Rust workspace including PostgreSQL-backed ignored tests | 809 passed, zero failed or ignored; pinned Rust 1.98.1, disposable PostgreSQL 17 with pgvector |
| Strict workspace Clippy, formatting and debug workspace build | Passed |
| Python suite with explicit Gitleaks binary | 61 passed, no skips |
| Schema-22 encrypted recovery | All 16 checks passed, including preservation of retired principal reservations and removal of personal tokens |
| Linux arm64 runtime image and source-installed tools | Built and installed successfully; no capture or startup services enabled |
| Actual local HTTPS stack upgrade | Schema 21 to 22, with a verified encrypted pre-upgrade database snapshot retained privately; runtime remains `wk_app`, readiness passed |
| Live Keycloak portal administration | Provision, scope denial, reissue, immediate revocation, removal and retired-name refusal passed |
| Live two-persona team-work queries | Name/module/file/filter checks passed and their synthetic card was withdrawn |
| Fresh HOME/prefix onboarding lifecycle | All 12 scenarios passed, including invalid CA, native proxies, service isolation, outage/restart, revocation, expiry, disconnect and uninstall/reinstall; synthetic account removed and card withdrawn |
| Normal Codex and Antigravity configuration | Reconnected through endpoint-specific token/CA files; both discovered 12 tools and retrieved the known opposite persona's card as published with implementation lifecycle `proposed` |

The local stack now runs `wk-server:integration-20261109`, also tagged `wk-server:local`, image ID `sha256:e0ee240f6c04ee24c9f4f7983a698ded5826c6d60dca1707b45ade78f212ca2d`. Its earlier schema-21 image remains retained. An old executable must not be started against schema 22 without established compatibility; rollback uses the isolated-restore procedure.

Local receipts are `/tmp/wk-integration-{rust,python-final,clippy,build,recovery,image,install,live-admin,live-team,live-onboarding}.log`. Fresh-HOME lifecycle metadata is retained in the private `wk-clean-user-45a8hvlz/results.json` temporary directory. The pre-upgrade snapshot and its private recovery identity are outside the repository in the local stack's private backup directory; neither is a release artifact.

## Integration and release boundary

The initial PR dependency audit reported four vulnerabilities, reproduced locally without suppressions. `rusqlite` is now 0.41.1 with bundled `libsqlite3-sys` 0.39.1, addressing [RUSTSEC-2026-0459](https://raw.githubusercontent.com/RustSec/advisory-db/main/crates/libsqlite3-sys/RUSTSEC-2026-0459.md). `rmcp` and its macros are now 3.9.0, addressing [RUSTSEC-2026-0457](https://raw.githubusercontent.com/RustSec/advisory-db/main/crates/rmcp/RUSTSEC-2026-0457.md), [RUSTSEC-2026-0461](https://raw.githubusercontent.com/RustSec/advisory-db/main/crates/rmcp/RUSTSEC-2026-0461.md) and [RUSTSEC-2026-0462](https://raw.githubusercontent.com/RustSec/advisory-db/main/crates/rmcp/RUSTSEC-2026-0462.md). The fresh unsuppressed local audit reports zero vulnerabilities and zero unsoundness warnings, retaining four unmaintained-package notices. The all-target normal dependency graph still excludes RSA. The SQLite feature selection preserves bundled native storage and caching without adding the new optional WASM backend. Final rebuilt/runtime and regression results for this dependency follow-up are recorded in [PR #25](https://github.com/DharanesshMD/workspace-knowledge/pull/25) and the frozen private packet; the earlier image above is retained as baseline evidence.

The integration PR and subsequent merged-commit workflows must pass before the merged SHA is frozen. Their exact revision, run URLs and artifact hashes belong in the private evaluation packet. Earlier CI at `b419704` remains historical baseline evidence; it does not qualify schema 22.

Repository visibility remains private. No alpha.4 tag or release is published. The current work supports local source-based evaluation; it does not certify a clean physical machine, signing/notarization, a hosted identity provider, a real-team deployment, universal IDE compatibility or flawless operation on arbitrary systems. See the [readiness ledger](../operations/production-readiness.md), [local pilot](../operations/local-pilot.md) and [upgrade instructions](../operations/upgrade.md).
