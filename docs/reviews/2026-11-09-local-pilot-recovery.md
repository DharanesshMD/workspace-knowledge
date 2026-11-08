# Local pilot recovery qualification — 2026-11-09

The user selected **local only**: no second Mac or pilot Linux server was supplied. This step uses disposable local infrastructure and prepares the pilot/release packet. No production resources, new hosted deployment, repository visibility change or release publication are part of it.

## Reproduced blocker and fix

At baseline `95af3df`, the real encrypted recovery drill failed with `Unsupported source schema`. The backup and restore manifests named schema 21, but the backup source guard and transactional restore guard both still required migrations 1–18. A current migrated database therefore could not use the documented recovery path.

Commit `b419704` makes both guards require exactly 1–21, keeping unknown/incomplete schemas and incompatible manifests refused. No legacy compatibility check was disabled. The PostgreSQL CI job now runs the real recovery drill in its own service container, with `age` installed there.

The drill now inserts a synthetic personal-token hash before the snapshot, verifies the row exists, and verifies restore removes it. Removing the quarantine DELETE deliberately made this regression fail with `Restored personal bearer token survived`; restoring the deletion made the complete drill pass again. No normal user credential or application database was used.

## Local results

| Check | Evidence |
|---|---|
| Current source debug workspace build | Passed with pinned Rust 1.98.1 and bounded build settings |
| Encrypted backup, transactional restore and access quarantine | All 15 recovery checks passed |
| New personal-token quarantine regression | Failed with quarantine removed, passed with it restored |
| Recovery tooling | PostgreSQL pg_dump/pg_restore 17.11; age 1.1.1 in the disposable Debian test container |
| Production-mode private-CA Keycloak acceptance | 85 passed, zero failed, Keycloak 26.4.7 |
| Python checks with explicit Gitleaks binary | 58 passed, no skips |
| Focused recovery/CI code review | No actionable findings |
| Fresh local cross-persona retrieval | Both normal client proxies retrieved the known opposite developer's card with published=true and lifecycle=proposed; both documented project files exist |

Recovery checks cover encrypted snapshot creation, atomic authority quarantine, personal-token removal, collector authentication cutoff, preflight invalidation, PUBLIC-function denial, both latest deletion journals, orphan deleted-key protection, unrelated-note preservation, repeat merges, nonempty-target refusal, malformed-ledger refusal, pre-commit fault rollback, ciphertext tamper refusal and incompatible-manifest refusal.

The authentication run covers the existing private-CA OIDC, cookie/CSRF, audiences, token failures, native collector/MCP login and renewal, fail-closed startup, key rotation/withdrawal, and starter-realm checks. It is a new local run, not a public-CA provider qualification or a physical-machine test.

The recovery harness dropped its synthetic databases; an explicit query found zero remaining `wk_hrestore_%` databases before its disposable container was stopped. The Keycloak harness removed its own temporary containers and synthetic credentials. The normal `wk-stack` deployment and its volumes were not restored, reset or reconfigured.

Local evidence files are `/tmp/wk-local-pilot-20261109-{build,recovery-red,recovery-final,token-regression-red,oidc,python}.log` and `/tmp/wk-local-pilot-20261109-oidc.json`. They are local receipts, not release attachments.

The refreshed release draft and pilot guide received a focused review. A circular upgrade instruction was corrected: start the app with its restricted connection while traffic remains stopped, check readiness, then resume traffic. No operator connection is used to serve the application.

## Remote regression verification

Both workflows passed at exact candidate `b4197043e24552e72c6a4a841244bffcee6b9fed`:

- [Foundation checks](https://github.com/DharanesshMD/workspace-knowledge/actions/runs/42515281498): all seven jobs passed, including Linux/macOS/Windows, PostgreSQL, console, OpenBao and the amd64 image.
- [Dependency audit](https://github.com/DharanesshMD/workspace-knowledge/actions/runs/42515284145): both history and dependency jobs passed.

The new PostgreSQL step `Verify deletion-safe encrypted backup and restore` executed successfully and printed all 15 passing recovery checks, including `personal_tokens_invalidated`. It used the owned CI PostgreSQL service container; age was installed there. Its log is retained locally at `/tmp/wk-local-pilot-20261109-ci-postgres.log` as well as in the linked run.

The subsequent documentation-only commit records this evidence and refreshes the pilot/release guides; it is not a claim that CI ran at that later SHA. No Rust application code or normal stack configuration changed in this recovery step.

## Remaining boundary

The [current alpha.4 draft](../operations/alpha4-release.md) and [local pilot protocol](../operations/local-pilot.md) describe the supported evaluation flow and its exit criteria. Neither a source archive nor fresh-HOME tests certify signed distribution, a clean OS account/physical machine, real-team deployment, public-provider behavior or independent security review. Native capture remains separately experimental and version-pinned. See the [readiness ledger](../operations/production-readiness.md).
