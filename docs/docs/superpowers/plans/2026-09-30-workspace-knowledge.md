# Workspace Knowledge Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans where available to implement this plan task-by-task. Other agents should follow the same sequential test/implementation/review workflow without requiring that skill. Steps use checkbox (`- [ ]`) syntax for tracking. This handoff does not authorize deployment, repository publication, client configuration changes on a real user's machine, or collection of actual conversations.

**Goal:** Build an open-source, on-premises, workspace-scoped knowledge service that automatically learns from supported AI sessions and exposes authorized summaries to other agents.

**Architecture:** A native local collector enforces enrollment, captures connector events into an encrypted private queue, and generates sanitized summaries. An authenticated server stores shared cards and exposes retrieval through MCP, CLI, and a web console. A separate broker handles approved environment secrets.

**Tech Stack:** Rust/Tokio/Serde/Axum/SQLx, SQLite, PostgreSQL, MCP SDK, React/TypeScript/Vite, local inference endpoint, optional pgvector and OpenBao. Exact supported versions are pinned in Task 1.

**Spec:** `../specs/2026-09-30-workspace-knowledge-design.md` (normative). This package is a new-project handoff, not an existing implementation.

**Revision:** Follow the [capture addendum](../../../CONVERSATION-CAPTURE-ADDENDUM.md); execute Tasks 9A/9B after Task 9 and Task 10A after Task 10. These replace the cooperative-only assumption for eligible Enterprise Desktop chats.

## Global Constraints

1. Support native clients on Windows, macOS, and Linux; certify exact OS/client versions in a release matrix.
2. No collection before local developer approval and server-side product membership.
3. No raw conversation payloads in shared knowledge storage, embeddings, application logs, or telemetry.
4. No personal-folder traversal; no implicit parent, sibling, symlink, junction, worktree, or submodule approval.
5. Never expose secret values through knowledge search or MCP responses.
6. Automatic publication requires both team policy and local developer opt-in; uncertain content remains private.
7. All shared reads and writes enforce authenticated organization/product/service permissions.
8. Capture failures must not block development; publication and secret access fail closed.
9. No hidden reasoning collection; unsupported client capabilities are reported explicitly.
10. Existing client configurations and agent instructions must be preserved during setup and uninstall.
11. All configuration examples and CLI commands in this package are proposed interfaces, not currently installed commands.

## Review Focus

- Reused sessions containing personal context: keep later derived summaries private until an attributable clean boundary (Tasks 2, 4, 6).
- Windows junctions, case variants and symlink replacement races: no out-of-scope reads (Task 2).
- Duplicate, truncated, interrupted and parallel-agent events: no invented pairing or completion (Tasks 3–4).
- Permission revocation during search/indexing/upload: no stale-result or publication leakage (Tasks 5, 7).
- Secret echoes and restored backups: secrets never enter knowledge; deleted cards do not reappear (Tasks 11, 14).

## Repository layout to create

```text
Cargo.toml / Cargo.lock / rust-toolchain.toml
crates/
  contracts/src/{event,card,policy,error}.rs
  scope/src/{enrollment,paths,services}.rs
  local/src/{ipc,crypto,store,outbox}.rs
  capture/src/{adapter,correlate,transcripts}.rs
  summarize/src/{provider,redact,extract,publish}.rs
  knowledge/src/{auth,cards,search,context,evidence}.rs
  environments/src/{profiles,openbao,process,import}.rs
  cli/src/{main,workspace,agent,doctor,env}.rs
  server/src/{main,routes,mcp}.rs
  worker/src/main.rs
  mcp/src/{main,tools}.rs
  adapters/src/{cursor,claude_code,codex,antigravity,gemini}.rs
web/src/{app,workspaces,cards,environments,health}/
migrations/server/ / migrations/local/
tests/{contracts,scope,capture,privacy,auth,mcp,env,system}/
fixtures/{synthetic,clients}/
deploy/{compose,systemd,launchd,windows}/
docs/{architecture,adapters,operations,security}/
```

Only create files when their task needs them. Names identify responsibilities; avoid empty package scaffolds. Each task ends with scoped tests, a short review, and a commit if executing inside an authorized Git repository. Do not commit user data, credentials, or generated raw test transcripts containing real conversations.

## Core contracts

Use JSON Schema for serialized contracts and Rust types for internal behavior. Unknown top-level fields are rejected for public writes. Timestamps use UTC RFC3339 and IDs use UUIDs; native client identifiers remain strings. The spec's event example is the private input contract, not an upload payload.

```rust
pub enum ScopeDecision { Approved, Excluded, Mixed, Unknown }
pub enum CaptureCompleteness { Complete, Partial, Truncated }
pub enum PublicationDecision { Publish, NeedsReview, Reject }
pub enum Lifecycle { Proposed, InProgress, Implemented, Verified, Superseded, Deprecated }
pub enum PublicationState { PrivateDraft, PolicyChecked, Published, NeedsReview, Rejected, Withdrawn }
```

`private_payload` must be a separate local-only type. A `CardSubmission` has `schema_version`, `idempotency_key`, `policy_revision`, `title`, `intent`, `rationale`, `decisions`, `constraints`, `open_questions`, `service_ids`, `evidence`, `lifecycle`, `capture_completeness`, and `base_revision` (nullable). It has no raw event field and no secret-value field. Author/product identity comes from the authenticated workspace binding.

Use a single `AppError` taxonomy: `Unauthenticated`, `NotFound`, `ForbiddenOperation`, `Validation`, `Conflict`, `RateLimited`, `Unavailable`. Map errors to the HTTP behavior in the spec and redact internal causes before serializing. CLI exit codes: 0 success, 2 validation/usage, 3 unauthenticated/permission failure, 4 unavailable, 5 conflict; JSON errors contain `code`, safe `message`, and `retryable`.

## Task 1 — Bootstrap contracts and synthetic test harness

**Files:** root Rust toolchain/workspace files; `crates/contracts/src/{lib,event,card,policy,error}.rs`; `fixtures/synthetic/{approved-session,mixed-session,parallel-session}.json`; `crates/contracts/tests/schema.rs`; `.github/workflows/ci.yml`.

**Consumes:** specification. **Produces:** versioned `PrivateEvent`, `CardSubmission`, `EffectivePolicy`, `AppError`, JSON schemas and repeatable contract tests.

- [ ] Define golden fixtures with fake developer IDs and deliberately fake secret canaries. Include full/partial sessions and two same-time parallel turns.
- [ ] Write a failing serialization test asserting that a raw `private_payload` field is rejected by `CardSubmission`, and an unknown schema version fails with `Validation`.
- [ ] Pin supported Rust/dependency versions and implement strict schemas; expose JSON schema documents under `docs/architecture/schemas/`.
- [ ] Run `cargo test -p wk-contracts` and `cargo fmt --check`; verify errors are field-specific and contain no rejected raw content.
- [ ] Add OS CI jobs for the contract tests and commit the bootstrap.

Representative contract test:

```rust
#[test]
fn shared_submission_rejects_raw_content() {
    let mut input: serde_json::Value = serde_json::from_str(
        include_str!("../../../fixtures/synthetic/card-submission.json")
    ).unwrap();
    input["private_payload"] = serde_json::json!({"text": "PRIVATE_CANARY"});
    assert!(serde_json::from_value::<wk_contracts::CardSubmission>(input).is_err());
}
```

Create `card-submission.json` with all fields listed in Core contracts, empty arrays for optional collections, `lifecycle: "proposed"`, `capture_completeness: "complete"`, and `base_revision: null`.

## Task 2 — Enrollment, folder boundaries, and service mapping

**Files:** `crates/scope/src/{lib,enrollment,paths,services}.rs`; `crates/scope/tests/boundaries.rs`; `crates/cli/src/{main,workspace}.rs`; `fixtures/synthetic/workspace.yaml`.

**Consumes:** `EffectivePolicy`. **Produces:** `WorkspaceBinding` containing local root identity, product ID, approval hash and services; `classify_path(binding, candidate) -> ScopeDecision`; `resolve_services(binding, paths) -> Vec<ServiceId>`.

- [ ] Build temporary sibling roots `product` and `product-personal`; test that string-prefix similarity grants nothing. Add symlink/junction escape, renamed root, drive mismatch, Unicode, excluded `.env`, nested repo and unapproved worktree cases.
- [ ] Run `cargo test -p wk-scope --test boundaries`; confirm the negative cases fail before implementation.
- [ ] Implement real-path/file-identity checks and local consent storage. Commands `init`, `approve`, `pause`, `resume`, `revoke`, `forget`, and `status` must never execute hooks from repository files.
- [ ] Test manifest scope expansion invalidates the previous grant, while a narrower grant remains usable. Overlapping service paths use the most specific approved service; unknown paths return Unknown.
- [ ] Run the test on all three OS families. Publish supported filesystem assumptions in `docs/security/workspace-boundaries.md` and commit.

## Task 3 — Local daemon, encrypted capture queue, and crash recovery

**Files:** `crates/local/src/{lib,ipc,crypto,store,outbox}.rs`; `migrations/local/0001.sql`; `crates/local/tests/{encryption,recovery,ipc}.rs`.

**Consumes:** approved `WorkspaceBinding`, `PrivateEvent`. **Produces:** `enqueue(event) -> Receipt`, `pending_batch(limit)`, `ack(receipt)`, `expire(now)` on `LocalStore`; authenticated local IPC.

- [ ] Test plaintext canary absence in SQLite main/WAL files after enqueue, duplicate enqueue after crash, missing OS key store, capacity overflow, and unauthorized local IPC connection.
- [ ] Run `cargo test -p wk-local` and record failing cases.
- [ ] Encrypt payloads with a maintained AEAD implementation, bind event IDs/schema version as associated data, store keys in native secure storage, and restrict IPC to the current user. Do not log deserialization inputs.
- [ ] Implement 24-hour retention and 100-MiB cap, content-free gap events, restart-safe outbox and bounded writes. Use injectable key-store test doubles; live secure-store tests remain part of release certification.
- [ ] Verify retention cleans persisted payloads according to SQLite/crypto erasure behavior and document filesystem/backup limitations. Run tests and commit.

## Task 4 — Adapter interface, event correlation, and transcript permissions

**Files:** `crates/capture/src/{lib,adapter,correlate,transcripts}.rs`; `crates/capture/tests/{replay,transcript_scope}.rs`; `fixtures/clients/synthetic.jsonl`.

**Consumes:** client payload, adapter capability descriptor, binding. **Produces:** `normalize(payload, context) -> Vec<PrivateEvent>`; `assemble(events) -> SessionSegment`; a synthetic adapter for testing.

- [ ] Replay interleaved sessions, retries, same parent session with two child agents, out-of-order chunks, missing stop, truncated payloads, and duplicate reconnect deliveries.
- [ ] Assert each response maps to its own turn, incomplete turns never become Complete, and replay is idempotent. Run `cargo test -p wk-capture` to verify failure.
- [ ] Implement stable composite IDs, persisted adapter cursors, explicit gap markers, and contamination tracking. Never join sessions by timestamp alone.
- [ ] Test a hook-provided transcript path outside the root: deny without session-specific consent; deny even with consent if session/workspace metadata disagrees. Read only granted segments and never scan a global history directory.
- [ ] Verify ignored reasoning event types and capture gaps are visible. Commit adapter contract and replay evidence.

## Task 5 — Authenticated shared card storage and permission enforcement

**Files:** `crates/knowledge/src/{lib,auth,cards}.rs`; `crates/server/src/{main,routes}.rs`; `migrations/server/0001.sql`; `crates/knowledge/tests/authorization.rs`.

**Consumes:** authenticated identity, `CardSubmission`, effective policy. **Produces:** versioned Card/Evidence storage, API endpoints from spec and PostgreSQL job queue.

- [ ] Seed organizations A/B, two products, and a cross-service card. Test forbidden reads/writes, guessed IDs, forged authors, cross-tenant foreign keys, stale policy and revision conflicts.
- [ ] Run database tests against isolated PostgreSQL (`WK_TEST_DATABASE_URL` points only to a disposable DB). Prohibit tests against non-test database names.
- [ ] Implement OIDC verification with issuer/audience/expiry checks, server-derived identity, membership/service filtering, RLS defense, migrations, idempotent submissions and optimistic revisions.
- [ ] Verify a stale device cannot upload after revocation; distinguish 409 revision conflict from 404 inaccessible card without leaking existence.
- [ ] Run `cargo test -p wk-knowledge --test authorization`; commit schema/API contract and an OpenAPI document generated from handlers.

## Task 6 — Summarization and automatic publication

**Files:** `crates/summarize/src/{lib,provider,redact,extract,publish}.rs`; `crates/worker/src/main.rs`; `crates/summarize/tests/policy.rs`; `fixtures/synthetic/privacy-corpus.json`.

**Consumes:** `SessionSegment`, `EffectivePolicy`. **Produces:** `summarize(segment) -> CardSubmission`; `evaluate(submission, scope, policy) -> PublicationDecision` plus reason codes.

- [ ] Build privacy cases: literal secret, encoded secret example, personal pasted text, unrelated folder reference, misleading success claim, embedded instruction to ignore policy, hallucinated evidence, and mixed-session later turn. Specify expected Publish/NeedsReview/Reject per case.
- [ ] Implement a deterministic fake provider for pipeline tests; run `cargo test -p wk-summarize` before real inference integration.
- [ ] Add local inference adapter with JSON schema validation, output limits, timeouts and no tools/file/network permissions for the summarizer. Retry at most twice, then keep a private draft. Store model/prompt/policy versions on approved output.
- [ ] Implement two-sided secret checks, source/claim validation and 15-second coalescing. Automatic publication cannot promote evidence status or bypass missing membership/consent. Provider unavailability never selects a cloud fallback.
- [ ] Evaluate real configured local model against the synthetic corpus, report false positives/negatives and latency, and require zero seeded-secret leaks before enabling policy-auto in a pilot. This is a test gate, not a proof of perfect privacy. Commit.

## Task 7 — Fast retrieval, context assembly, and MCP

**Files:** `crates/knowledge/src/{search,context}.rs`; `crates/mcp/src/{main,tools}.rs`; `crates/server/src/mcp.rs`; `crates/mcp/tests/{protocol,permissions}.rs`.

**Consumes:** authorized card store. **Produces:** seven MCP tools in spec, stdio bridge, authenticated remote MCP, and `wk search --json` / `wk context --json`.

- [ ] Seed relevant, irrelevant, outdated and unauthorized cards. Test permission filtering before ranking/counts/snippets; inject a secret canary into a query and confirm no shared persistence.
- [ ] Test tool discovery, input validation, 20-result cap, 2,000-token context budget, citation IDs, missing evidence, cancellation, and prompt-injection text treated as data.
- [ ] Implement PostgreSQL full-text retrieval, service/status filters, weighted freshness/evidence ranking, bounded context and revision citations. Use conservative token counting if the consumer tokenizer is unknown.
- [ ] Add stdio MCP binding to one enrolled workspace and remote authorization according to negotiated protocol. Verify tenant-scoped tokens and no token passthrough. No secret-value tools.
- [ ] Run `cargo test -p wk-mcp`; execute one real-client search after installation in a test account. Benchmark the spec's load target; report measured results and commit.

## Task 8 — Cursor and Claude Code capture, setup and doctor

**Files:** `crates/adapters/src/{lib,cursor,claude_code}.rs`; `crates/cli/src/{agent,doctor}.rs`; `fixtures/clients/{cursor,claude_code}/`; `docs/adapters/{cursor,claude-code}.md`.

**Consumes:** adapter contract, local IPC, MCP bridge. **Produces:** version-gated hook adapters and non-destructive installer/uninstaller.

- [ ] Capture synthetic payload fixtures from each installed test client, record version/platform and verify schemas against current official docs. Avoid real developer transcripts.
- [ ] Test config merge round-trip, JSON-with-comments preservation where applicable, duplicate installation, user edits after installation, paths with spaces, and uninstall that removes only owned entries.
- [ ] Install signed/verified native `wk` hook executable references; no Bash-only dependency. Hook responses follow each client's success schema and never trigger continuation loops.
- [ ] Implement `doctor`: separately report config present, MCP handshake, scope approved, hook received, prompt captured, response captured, and published card retrieved. Missing fields produce partial capability, not a green success badge.
- [ ] Live test user A creates a fake feature in approved root; user B retrieves its card. Repeat outside root and verify no capture/publication. Test unavailable daemon exits quickly without blocking the agent. Commit fixtures and proof.

## Task 9 — Codex, Antigravity, Gemini, and Claude Desktop compatibility

**Files:** `crates/adapters/src/{codex,antigravity,gemini}.rs`; `fixtures/clients/{codex,antigravity,gemini}/`; `docs/adapters/{codex,antigravity,gemini,claude-desktop}.md`; `docs/adapters/compatibility.json`.

**Consumes:** established adapters/install/doctor contracts. **Produces:** each spec compatibility row with tested, partial or unsupported status.

- [ ] Verify Codex hook fields on CLI and desktop separately, including parent/child identity. Prefer prompt and last-response hook fields; do not rely on a global transcript scanner.
- [ ] Test Antigravity invocation/tool/stop hooks on each supported surface. Full-content extraction remains partial until scoped transcript consent and parser fixtures pass. Do not mistake invocation metadata for prompt content.
- [ ] Add Gemini CLI lifecycle adapter with its own fixture replay and completion semantics.
- [ ] Register local MCP for Claude Desktop general chat, test retrieval and cooperative `knowledge.record`; label capture cooperative. Claude Code mode is a separate row. Provide CLI fallback for clients without supported MCP.
- [ ] Run `cargo test -p wk-adapters`; attach manual end-to-end evidence for each certified client/OS. If unavailable on a platform, record not tested/unsupported and continue supported rows. No universal-capture marketing. Commit.

## Task 10 — Evidence lifecycle, conflicts, and knowledge feedback

**Files:** `crates/knowledge/src/evidence.rs`; `crates/knowledge/tests/lifecycle.rs`; repository connector module under `crates/knowledge/src/evidence/`; migrations for revision/evidence updates.

**Consumes:** cards, authorized repository metadata, test results. **Produces:** evidence-backed lifecycle transitions and revision-safe feedback.

- [ ] Test response says “done” with no diff, unmerged branch, merged PR with failing tests, stale test SHA, force-pushed branch, and conflicting decisions from two authors.
- [ ] Implement link validation and source-host allowlists; prevent server-side requests to arbitrary evidence URLs. Store provider identity and immutable revision when available.
- [ ] Promote status only through stated evidence rules. File changes alone are not Verified. Mark affected cards potentially stale when linked revisions change.
- [ ] Implement corrections with optimistic revision checks, conflict links and superseding history; do not silently deduplicate distinct decisions using embeddings.
- [ ] Run `cargo test -p wk-knowledge --test lifecycle`, demonstrate corrected answers after withdrawal/supersession, and commit.

## Task 11 — Approved environment sharing through OpenBao

**Files:** `crates/environments/src/{lib,profiles,openbao,process,import}.rs`; `crates/cli/src/env.rs`; `crates/environments/tests/{broker,redaction}.rs`; `deploy/compose/openbao-test.yaml`.

**Consumes:** separate environment permission, profiles with vault references, developer identity. **Produces:** describe/import/run/export CLI and metadata-only MCP describe tool.

- [ ] Start a disposable OpenBao instance for tests; development mode is never a production example. Test unauthorized profile, revoked lease, rotated value and production-default denial.
- [ ] Import a synthetic `.env` only after explicit confirmation in the test fixture. Assert values exist in the vault but not cards, SQLite outbox, API bodies to the knowledge service or captured logs.
- [ ] Spawn a child process with an injected variable and verify its value internally without printing it. Test argument arrays with spaces/metacharacters without invoking a shell.
- [ ] Test tracked `.env` export refusal and Windows/POSIX file permissions. Add a deliberate child echo case: suppression of known values must work; document that arbitrary transformations/exfiltration are not preventable by this broker.
- [ ] Run `cargo test -p wk-environments`; test rotation/revocation and metadata-only audit. Commit environment setup documentation and threat limits.

## Task 12 — Minimal developer and administrator console

**Files:** `web/package.json`, lockfile, `web/src/{app,workspaces,cards,environments,health}/`, `web/tests/flows.spec.ts`.

**Consumes:** authenticated API. **Produces:** workspace/service policy view, pause status, knowledge search/detail/revision, review queue, environment schema, connector health.

- [ ] Write browser tests for permission denial, private draft visibility, auto-published label, mixed-session review reason, raw-transcript absence, and no secret values in DOM/network responses.
- [ ] Implement accessible forms and clear scope/publication/inference-destination labels. Reuse API authorization; frontend hiding is not a permission control.
- [ ] Display evidence freshness and incomplete coverage prominently. Allow correction/withdrawal; environment pages expose schema and access state only.
- [ ] Run `npm ci`, `npm run test`, and `npm run build` from `web/`; validate desktop and narrow viewport layouts with keyboard navigation.
- [ ] Commit verified UI and screenshots containing only synthetic data.

## Task 13 — Optional semantic search without a new database

**Files:** `crates/knowledge/src/search/embeddings.rs`; vector migration; `crates/knowledge/tests/hybrid_search.rs`.

**Consumes:** approved card revisions and on-premises embedding endpoint. **Produces:** optional hybrid retrieval; lexical search remains available without the feature.

- [ ] Test unauthorized neighbors, revoked membership, mixed embedding dimensions/model versions, unavailable embedding server and deleted cards.
- [ ] Generate embeddings from approved summaries only; use a versioned namespace per model/dimension. Never embed raw chats or env values.
- [ ] Apply permission filters before returning candidates and combine semantic/lexical ranks; falling back to lexical is explicit in diagnostics.
- [ ] Evaluate retrieval quality with a synthetic feature-reuse set against the lexical baseline; keep the feature optional if quality/latency does not improve.
- [ ] Run `cargo test -p wk-knowledge --test hybrid_search` and commit benchmark results.

## Task 14 — Cross-platform distribution, operations, and open-source release

**Files:** `deploy/{compose,systemd,launchd,windows}/`; `.github/workflows/{release,security}.yml`; root open-source files; `docs/operations/{install,backup,restore,upgrade,uninstall}.md`; `docs/security/threat-model.md`.

**Consumes:** tested components. **Produces:** signed native release artifacts, Linux container deployment, source installation instructions and tested runbooks.

- [ ] Test fresh install/update/uninstall on Windows x64, macOS arm64/x64 where runners are available, and Linux x64/arm64. Publish exact OS/runtime versions; unsupported combinations remain source-build/community targets.
- [ ] Bundle web assets in server build; provide Docker Compose for a Linux on-prem host and native binaries for developer machines. macOS/Windows container hosts require a Linux VM; do not imply native Linux containers run without it.
- [ ] Test service startup through systemd user service, launchd agent, and Windows user-session startup; support foreground mode when secure storage/user session is unavailable. Avoid running the collector as root/system.
- [ ] Exercise encrypted backup/restore, withdrawal tombstone replay, policy revocation, schema upgrade and failed migration recovery. Restore must not reopen deleted cards or roll back consent silently.
- [ ] Run full `cargo test --workspace`, formatting/linting, UI tests and dependency/license/secret scans. Generate SBOM, checksums and signatures; no default telemetry or default production credentials.
- [ ] Add Apache-2.0 LICENSE, NOTICE, contributor/DCO policy, SECURITY reporting process, adapter guide, issue templates requesting synthetic repros, and a complete offline bundle recipe. Publish only after a separate explicit release authorization.

## Acceptance demonstration

Use two users, two devices, and one product with frontend/backend/ai-engine services. User A implements a synthetic shared rate limiter with a tested connector. Within the measured publication interval, User B's different agent retrieves its summary/evidence and reuses it. Neither user can retrieve the other's prompts. A personal sibling-folder interaction produces no card. A mixed session stays private. An excluded `.env` is not collected, while an explicit approved environment import enables a backend process without exposing values through MCP. Revoking B removes retrieval/environment access. Server outage allows work to continue and eventual recovery creates no duplicate cards.

## Agent execution report format

After each task report: changed files, tested commands/results, observed limitations, and next dependency. Never claim a mock or fixture proves live integration. Record unresolved client/platform compatibility as a capability status. Keep task checkboxes accurate. Do not begin a later feature that depends on a failing scope/privacy/authorization gate.
