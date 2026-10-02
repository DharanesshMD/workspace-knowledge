# Implementation ledger

Branch: `feat/foundation`. Dedicated new repository; existing projects were not modified. Rust was installed locally under ignored `.tooling`, without shell configuration changes.

## Working development slice

- Strict knowledge-card contracts and conservative implementation-status handling.
- Local consent and service-folder scope classification, pause and revoke.
- Encrypted SQLite outbox library with bounded retention and duplicate protection.
- Offline JSONL replay correlation, mixed-session rejection, metadata-only results.
- PostgreSQL draft/publication storage, tenant and all-service membership checks, idempotency and search.
- Authenticated loopback development API and official-SDK stdio MCP retrieval.
- Synthetic local demo, database authorization tests, and real HTTP/MCP integration test.
- Linux/macOS/Windows CI matrix plus PostgreSQL integration job. Remote CI has not run.

These are partial implementations of the detailed plan, not completion of all its milestones. The capture addendum governs the Desktop capture direction.

## Verification and review

Local macOS arm64 verification passed: 37 workspace tests plus 3 explicitly run PostgreSQL tests (40 total), with no failures. Formatting, Clippy with warnings denied, and workspace build also passed. Verification covers workspace tests, formatting, Clippy, builds, synthetic scope/replay demo, PostgreSQL 17 integration, and MCP protocol initialization/discovery/retrieval. A disposable Docker database was used; initial registry timeout was resolved through the official-image ECR mirror.

A partial independent review found a service-symlink retargeting bypass. Service enrollment now rejects symlink components, with a passing regression test. The reviewer hit a usage limit before completing the full review; no completed independent security audit is claimed. CLI private-directory tests also drove rejection of unsafe directories without altering their permissions or creating state inside the product.

## Historical pending list at foundation checkpoint

- Native credential storage, Windows ACL/junction certification, authenticated daemon IPC and secure file-opening primitives.
- Live adapters and durable session coverage tracking; current replay completeness only describes the supplied fixture.
- Local summarization, semantic secret/personal-data filtering, approved automatic publication, provenance and evidence promotion.
- Production OIDC, administrative enrollment, database roles/RLS and migration lifecycle, revocation/concurrency hardening.
- Knowledge graph traversal, revision/withdrawal workflows and evidence refresh.
- Environment sharing through a distinct permissioned secret broker; never search indexing of environment values.
- Enterprise Desktop capture broker / instrumented chat, client registration certification, console and signed distribution.

At that foundation checkpoint, binaries did not install hooks, observe network traffic, capture desktop chats, or read personal folders. The outbox was not yet connected to the CLI. No remote repository or package has been published.

## 2026-10-01 — private capture journal

Ruling: continue in the existing dedicated `feat/foundation` checkout, with a clean baseline at `663237a`. Implement the bounded storage/correlation integration before exposing live capture; native key storage and authenticated IPC remain hard prerequisites for real data. This advances Tasks 3–4 without marking either full milestone complete.

Added transactional encrypted updates and a normalized-event session journal. Tests were first run with missing APIs (red), then passed after implementation. A fresh review found that payload expiry reopened held sessions; a regression reproduced the failure. Separate bounded content-free guards now close expired/acknowledged IDs permanently, including through the older enqueue API. The guard-reset/forget workflow remains pending.

Ruling: fail closed after a session's first 24-hour capture window, rather than automatically treating a reconnect as a new session. Adapters must preserve source IDs across reconnects and create a new ID only at a genuine session boundary. No claim of automatic live capture, full transcript coverage, or publication follows from journal receipts.

Verification: `cargo test --workspace --locked` passed 49 tests; the 3 PostgreSQL tests were skipped and not rerun because this increment does not change shared storage/API behavior. `cargo clippy --workspace --all-targets --locked -- -D warnings`, formatting, workspace build, synthetic CLI demo, and diff checks passed locally on macOS arm64. Cross-platform CI remains unrun.

## Full implementation pass — 2026-10-01

Ruling: the user's instruction to finish implementation before testing overrides skill sequencing. Write coverage alongside implementation, but defer test/build execution to final integration. Use subagent-driven development for independent backend, console, and environment/release components; root owns capture, summarization, adapters, CLI/MCP integration. Keep external account access, real-chat collection, publication, and host-agent configuration outside this implementation pass.

Delegated workers reached their account usage limit during this pass. Root resumed their uncommitted code and integrated the backend, console, and environment components. Source, docs, and candidate release workflows are now present across the remaining plan areas. Tasks requiring unavailable native clients, a configured real local model, Enterprise entitlement, live OpenBao/OIDC, multi-device OS runners or signing keys remain explicit acceptance gates in `docs/architecture/completion-matrix.md`. No task checkbox is marked complete without its acceptance evidence.

## Integration checkpoint — 2026-10-01

Implemented automatic graph creation inside the publication transaction, with conservative summary-only provenance. Trusted evidence ingestion now retains implemented status when an owner attestation follows a code observation; matching test evidence promotes to verified. Added PostgreSQL regression coverage for both behaviors and exercised graph writes through a non-owner RLS runtime role.

Validated the separate environment broker against disposable OpenBao 2.4.3 over verified TLS: scoped import, CAS protection, child delivery, token removal, output suppression, rotation and revoked-token denial all passed. The repeatable synthetic script and CI job are included. Fixed the console development API proxy to port 4318.

Validation: workspace tests, seven explicit PostgreSQL integration tests, Clippy with warnings denied, full workspace build, real HTTP/MCP smoke with all seven tools discovered, console production build, and three browser checks passed. The browser checks cover authorization failure, empty search and a 390px viewport. CI jobs now include the console and TLS environment exercise; remote runs remain unverified.

This is an integrated development build, not full completion of the original plan. The completion matrix remains authoritative for unfinished implementation and external certification. No real chat was captured, no personal agent configuration was modified, and no release was published. Independent review of the broad implementation remains incomplete because delegated reviewers hit their usage limit.

## Reliability and integration contracts — 2026-10-01

Continued implementation in the existing feature checkout, preserving the earlier uncommitted work. The private collector now selects at most ten session IDs per pass using a durable rotating SQLite cursor, reads one private session at a time, and filters draft pagination before limiting results. This removes starvation caused by the original shared first-100-record scan. `drafts --after CURSOR` exposes subsequent summary pages. Generated drafts use durable closure guards, and their identity binds source, model and policy so unchanged submitted/expired drafts are not automatically recreated while changed inputs receive distinct identities.

OIDC keys now refresh on one scheduled worker every 60 seconds. Key replacement is atomic across request handlers. Failed refreshes do not extend the last successful snapshot's five-minute lifetime; expired snapshots reject authentication until recovery. Unknown token key IDs cause no network access. Added duplicate key-ID, algorithm and intended-purpose checks. The provider must overlap old and new keys during rotation; real-provider and load certification remain outstanding.

Published generated JSON schemas for sanitized card submissions, private normalized capture events and effective membership policy. Snapshot tests detect drift from Rust wire types. Runtime scope, byte-limit, privacy and authorization checks remain authoritative.

Verification completed after implementation: 72 workspace tests passed; all seven PostgreSQL tests passed against a freshly migrated disposable database, including the non-owner RLS role; HTTP/MCP smoke passed; full workspace build, formatting, Clippy with warnings denied and diff checks passed. Synthetic OIDC tests exercise shared-cache rotation, invalid refresh preservation, fixed expiry and recovery. Queue tests exercise 135 retained sessions across reopen, filtering/pagination, expiry and submitted-draft closure. No real keychain, chat or identity-provider account was used. This checkpoint does not close the remaining gates in the completion matrix.

## Reviewed knowledge relationships — 2026-10-02

Added reviewer-only, revision-checked conflict, dependency and supersession relationships through the product API. Both published card endpoints require current all-service permissions. Stable card lock ordering protects concurrent edits. Supersession atomically adds a historical target revision, marks its evidence stale, and excludes superseded/deprecated cards from ordinary search and MCP context retrieval. Explicit authorized history remains accessible; withdrawing a replacement does not silently restore the superseded card. Same-card graph refreshes preserve reviewed links and verified provenance instead of deleting them. Trusted evidence cannot promote superseded/deprecated cards back to an active state.

Added an evidence-reference endpoint that excludes internal connector metadata and checks historical as well as current service scopes. A regression proves that narrowing a card's scope cannot expose evidence from the old broader revision. Relationship authoring is available through the API; console authoring and signed source connectors remain unfinished.

Validation: workspace tests passed; all nine PostgreSQL integration tests passed, including relationship mutations under a non-owner RLS role, hidden endpoint denial, stale revisions, preserved links/history, retrieval after supersession, withdrawal and evidence-scope privacy. Clippy with warnings denied, the workspace build, formatting and diff checks passed. HTTP/MCP smoke now exercises revision-safe supersession and confirms the replacement is the only search result. Disposable PostgreSQL was removed afterward. No live user data or host agent configuration changed.

## Remote MCP, signed evidence, Jev decisions and release tooling — 2026-10-02

Added stateless authenticated Streamable HTTP MCP (`/mcp/{product}`) with OAuth protected-resource metadata, mirroring the stdio tools. Added Ed25519-signed evidence ingestion (`wk-evidence`) with pinned trust policy, 15-minute expiry, durable replay guard and signed revocation. The console now authors reviewed relationships and shows trusted evidence separately from author-reported references.

Per the user's request, semantic decision points use Jev (TypeSafe) as an advisory layer: one batched request per assessment covers routing, review priority and pairwise conflict suggestions over at most four authorized published cards. Results never grant access, publish, verify code or mutate the graph; uncalibrated or uncertain outcomes route to review. The paper's maximum-label-probability signal is kept separate from the API `confidence` field. The integration is disabled until an endpoint is configured: whether to use a private deployment or approved hosted processing of sanitized summaries is still an open deployment decision. No hosted request was made, so latency/accuracy gains are unmeasured. `scripts/jev-calibrate.py` derives a candidate threshold offline from labeled data.

The restore drill exposed a real bug: pg_restore clears the search path, so post-restore access removal could not find the tables. Statements now use explicit `public.` names. The cargo audit flagged RUSTSEC-2023-0071 (rsa); JWT verification moved to the aws-lc-rs backend and tests use fixed synthetic key fixtures, removing rsa from the build graph. A documented exception covers the uncompiled sqlx-mysql lockfile entry, and CI asserts rsa stays out of the graph.

Verification: 77 workspace tests; 10 PostgreSQL tests (including signed evidence and non-owner RLS); HTTP plus stdio and remote MCP integration; encrypted restore drill; console build and 5 browser tests; Clippy (warnings denied), formatting, build, synthetic demo; cargo audit and npm audit clean. OpenBao integration was not rerun (unchanged since its last pass). All checks ran locally on macOS arm64; remote CI has not run.
