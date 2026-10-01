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

## Pending before real data or production

- Native credential storage, Windows ACL/junction certification, authenticated daemon IPC and secure file-opening primitives.
- Live adapters and durable session coverage tracking; current replay completeness only describes the supplied fixture.
- Local summarization, semantic secret/personal-data filtering, approved automatic publication, provenance and evidence promotion.
- Production OIDC, administrative enrollment, database roles/RLS and migration lifecycle, revocation/concurrency hardening.
- Knowledge graph traversal, revision/withdrawal workflows and evidence refresh.
- Environment sharing through a distinct permissioned secret broker; never search indexing of environment values.
- Enterprise Desktop capture broker / instrumented chat, client registration certification, console and signed distribution.

Current binaries do not install hooks, observe network traffic, capture desktop chats, or read personal folders. The outbox is not connected to the CLI until production key management exists. No remote repository or package has been published.

## 2026-10-01 — private capture journal

Ruling: continue in the existing dedicated `feat/foundation` checkout, with a clean baseline at `663237a`. Implement the bounded storage/correlation integration before exposing live capture; native key storage and authenticated IPC remain hard prerequisites for real data. This advances Tasks 3–4 without marking either full milestone complete.

Added transactional encrypted updates and a normalized-event session journal. Tests were first run with missing APIs (red), then passed after implementation. A fresh review found that payload expiry reopened held sessions; a regression reproduced the failure. Separate bounded content-free guards now close expired/acknowledged IDs permanently, including through the older enqueue API. The guard-reset/forget workflow remains pending.

Ruling: fail closed after a session's first 24-hour capture window, rather than automatically treating a reconnect as a new session. Adapters must preserve source IDs across reconnects and create a new ID only at a genuine session boundary. No claim of automatic live capture, full transcript coverage, or publication follows from journal receipts.

Verification: `cargo test --workspace --locked` passed 49 tests; the 3 PostgreSQL tests were skipped and not rerun because this increment does not change shared storage/API behavior. `cargo clippy --workspace --all-targets --locked -- -D warnings`, formatting, workspace build, synthetic CLI demo, and diff checks passed locally on macOS arm64. Cross-platform CI remains unrun.
