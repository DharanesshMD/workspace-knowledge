# Codex and Antigravity Native Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce private, correctly attributed knowledge drafts from enrolled Codex CLI and Antigravity desktop conversations, then demonstrate reviewed publication and retrieval by a second authorized identity.

**Architecture:** Keep vendor parsing in separate adapters and persist their evidence in the existing encrypted local store. Add a narrowly scoped native evidence path beside the working Cursor path; keep server authorization and reviewed publication unchanged. Sequence both adapters in one plan because they share the evidence, enrollment, retention and submission gates.

**Tech Stack:** Rust, serde/serde_json, Tokio, existing SQLite/XChaCha20Poly1305 local store, existing HTTP collector and PostgreSQL-backed knowledge service. No new runtime dependencies planned.

**Spec:** [Approved capture design](../specs/2026-10-08-codex-antigravity-capture-design.md).

## Global Constraints

- Start with Codex CLI 0.162.0-alpha.2 and Antigravity desktop 2.19.1 on macOS. Other versions and surfaces remain unverified.
- No claim of universal coverage, unattended operation or production certification is permitted.
- Broad history scanning, screen scraping and network interception are excluded.
- Codex installations must use normal hook review/trust; the diagnostic bypass is not an installation default.
- Do not use unattributed-turn for either adapter.
- Retain only recognized visible message fields and scope evidence, never reasoning fields, system prompts, raw tool output or arbitrary attachment text.
- Keep reviewed publication; no automatic sharing.
- Tool and mutation record formats require synthetic live examples before adding them to the parser.
- Five-developer load, signing/notarization, other platforms, enterprise deployment and broad client certification remain separate release gates.
- Work in `/Users/dharun/.codex/worktrees/cursor-native-capture/workspace-knowledge`, branch `codex/cursor-native-capture`. Preserve unrelated changes in the original checkout.
- Use `/Users/dharun/Personal/Projects/workspace-knowledge/scripts/cargo` for the `cargo` commands below. Do not change user login credentials or global model routing.
- Limits: hook body 256 KiB; transcript snapshot 4 MiB; JSONL record 256 KiB; encrypted record below existing 1 MiB store limit; raw evidence expires 24 hours after first observation without extension; drafts expire no later than their source and within one hour of creation. Overflow holds the source; never silently truncate evidence.

## Review Focus

1. A delayed Antigravity Stop with repeated counters must not complete a later turn (Tasks 1 and 5).
2. A client update, workspace alias or config edit must not silently widen enrollment (Task 2).
3. Cancellation followed by replayed final text must remain cancelled across restart (Tasks 3 and 6).
4. File replacement or permission revocation during inference must discard the result and prevent submission (Tasks 4 and 7).
5. A held source must expire even during outages or repeated callbacks; sensitive content must stay absent from shared data (Tasks 6–8).

## File and interface map

Create `crates/adapters/src/native_evidence.rs` for the shared value types only, `codex_session.rs` for direct-hook state, and `antigravity_session.rs` for pure transcript parsing. Export these from `crates/adapters/src/lib.rs` without changing legacy normalization.

Create `crates/agent/src/native_enrollment.rs` for explicit enrollment and version checks, `antigravity_source.rs` for exact callback-bound file access, `native_capture.rs` for persistence/reconciliation, and `native_process.rs` for private draft lifecycle. Wire modules from `main.rs`; route native clients before legacy normalization in `daemon.rs`; dispatch native drafts before legacy handling in `process.rs`.

Modify `crates/adapters/src/install.rs` only for vendor-shaped config merging/removal with ownership receipts. Extract descriptor-walk helpers from `cursor_source.rs` into `source_file.rs` only if needed by Antigravity; preserve their semantics and all Cursor tests. No local-store schema change is expected.

Tests live in `crates/adapters/tests/native_evidence.rs`, `codex_session.rs`, `antigravity_session.rs`, and `native_install.rs`; agent binary tests belong in each new module's `#[cfg(test)]` block. Extend `crates/server/tests/native_handoff.rs` for authorization/publication regression. Keep only synthetic fixtures in `crates/adapters/tests/fixtures/`.

### Task 1: Freeze vendor evidence and completion boundaries

**Files:** Create `docs/reviews/2026-10-08-native-capture-fixtures.md` and synthetic fixtures under `crates/adapters/tests/fixtures/native/`; create `native_evidence.rs` and its integration test; export module in `lib.rs`.

**Interfaces:** Produce the following serializable types and predicate, using `serde::{Serialize, Deserialize}` derives and `#[serde(deny_unknown_fields)]` on structs:

```rust
pub enum Client { Codex, Antigravity }
pub enum Scope { Verified, Unknown, Violated }
pub enum Terminal { Open, Complete, Cancelled, Held }
pub struct Evidence {
    pub client: Client,
    pub client_version: String,
    pub parser_version: u32,
    pub session: String,
    pub agent: String,
    pub turn: String,
    pub prompt: String,
    pub response: String,
    pub messages_complete: bool,
    pub scope: Scope,
    pub terminal: Terminal,
    pub digest: String,
    pub files_read: Vec<String>,
    pub files_changed: Vec<String>,
    pub hold_reason: Option<String>,
}
pub fn eligible(e: &Evidence) -> bool {
    e.messages_complete && e.scope == Scope::Verified
        && e.terminal == Terminal::Complete && e.hold_reason.is_none()
}
```

Derive Clone/Debug/PartialEq/Eq on enums and Clone/Debug on Evidence. Digests cover normalized visible fields, identity, scope and terminal evidence; they are computed by adapters, never accepted from the model.

- [ ] Recover only the known synthetic probe fixtures identified in the live-results report. If the full USER_REQUEST wrapper is unavailable, generate a new synthetic no-tool conversation and read only its callback-named source. Record exact field shapes and visible UI comparison without retaining hidden reasoning.
- [ ] Run bounded synthetic initial/continuation and read/edit probes on each pinned client, using normal hook trust. Record cancellation, tool identity/path fields and all terminal fields. Stop adding support for any unobserved format; its test fixture must expect a hold.
- [ ] Specifically establish whether an Antigravity terminal callback can be bound to a unique turn through documented fields plus observed transcript data. `executionNum`, `invocationNum`, arrival time and `fullyIdle` are insufficient. If there is no reliable correlation, keep Antigravity completion disabled and record this limitation; complete the remaining private-capture work without claiming end-to-end acceptance.
- [ ] Write a predicate test before implementation:

```rust
#[test]
fn cancelled_evidence_is_never_publishable() {
    use wk_adapters::native_evidence::*;
    let e = Evidence { client: Client::Codex,
        client_version: "0.162.0-alpha.2".into(), parser_version: 1,
        session: "s".into(), agent: "main".into(), turn: "t".into(),
        prompt: "synthetic".into(), response: "ok".into(),
        messages_complete: true, scope: Scope::Verified,
        terminal: Terminal::Cancelled, digest: "test".into(),
        files_read: vec![], files_changed: vec![], hold_reason: None };
    assert!(!eligible(&e));
}
```

- [ ] Run `cargo test -p wk-adapters --test native_evidence`; expect unresolved module, then implement the types/predicate above and rerun to pass. Extend the test by changing one condition at a time: incomplete, unknown scope, violated scope, open, held, hold reason; all must reject.
- [ ] Commit fixture metadata and evidence contract: `git commit -m "feat: define bounded native capture evidence"` after staging only this task's files.

### Task 2: Explicit enrollment and reversible hook installation

**Files:** Create `crates/agent/src/native_enrollment.rs`; modify `main.rs`, `crates/adapters/src/lib.rs`, `install.rs`; create `tests/native_install.rs`.

**Interfaces:** `native_enrollment::enroll(dir: &Path, client: Client) -> Result<(), String>`, `load(dir: &Path, client: Client) -> Result<Enrollment, String>`, `disable(dir: &Path, client: Client) -> Result<(), String>`. `Enrollment` stores client, version, surface, device_id, canonical_workspace, service, policy_hash and enabled. Strings except Client client, PathBuf canonical_workspace and bool enabled. Derive Clone/Debug/Serialize/Deserialize with unknown-field rejection. Existing `config(client, executable, state) -> Result<Value,String>` remains the public config API.

- [ ] Write an installation regression using the existing API:

```rust
#[test]
fn codex_install_is_idempotent_and_reversible() {
    use serde_json::json;
    use wk_adapters::{config, install};
    let before = json!({"custom": "preserve", "hooks": {}});
    let fragment = config("codex", "/tmp/wk-agent", "/tmp/wk-state").unwrap();
    let once = install::merge(&before, &fragment).unwrap();
    assert_eq!(install::merge(&once, &fragment).unwrap(), once);
    assert_eq!(install::remove(&once, &fragment).unwrap(), before);
    assert!(!fragment.to_string().contains("dangerously-bypass-hook-trust"));
}
```

- [ ] Run `cargo test -p wk-adapters --test native_install`; expect unsupported client failure. Add corresponding Antigravity named-hook tests, conflicting ownership, malformed existing JSON and unrelated hooks preservation before changing merge/remove.
- [ ] Add `native-enroll --client codex|antigravity --yes` and `native-disable --client ...`. Use the already loaded workspace binding; verify installed version from the selected executable/app, persist canonical workspace and device identity, reject unsupported OS/version. Disable enrollment before removing owned hook entries; disable remains effective even if config removal fails.
- [ ] Generate only observed vendor hook formats. Register prompt/final/cancel/tool/compaction hooks for Codex and invocation/stop/tool hooks for Antigravity. Keep command escaping restrictions. Preserve user hooks; refuse conflicting named Antigravity entries instead of overwriting. Save ownership receipts and atomic config writes using existing installation machinery.
- [ ] Add enrollment unit tests with injected installed-version and workspace values: wrong version, changed policy, different service, extra root, disabled enrollment and symlink alias mismatch all reject before content storage. Test canonical alias normalization only during explicit enrollment, not as an alternate authorization path.
- [ ] Run `cargo test -p wk-adapters --test native_install` and `cargo test -p wk-agent native_enrollment`; require pass, then commit `feat: enroll pinned native clients safely`.

### Task 3: Codex direct-hook turn state

**Files:** Create `crates/adapters/src/codex_session.rs`, `tests/codex_session.rs`; export module.

**Interfaces:** `apply(previous: Option<&Evidence>, payload: &Value, workspace: &Path) -> Result<Evidence,String>`. Caller validates enrollment and supplies the canonical service boundary; parser validates cwd and exact session/agent/turn identity. Absent agent maps to main only for the observed main-session schema; any subagent indicators hold.

- [ ] Write the cancellation test:

```rust
#[test]
fn interrupt_is_sticky() {
    use serde_json::json;
    use std::path::Path;
    use wk_adapters::{codex_session::apply, native_evidence::eligible};
    let root = std::env::temp_dir().canonicalize().unwrap();
    let base = json!({"session_id":"synthetic", "turn_id":"one",
        "cwd":root, "hook_event_name":"UserPromptSubmit", "prompt":"hello"});
    let first = apply(None, &base, Path::new(&root)).unwrap();
    let mut interrupted = base.clone();
    interrupted["hook_event_name"] = json!("Interrupt");
    let cancelled = apply(Some(&first), &interrupted, &root).unwrap();
    let mut stop = base;
    stop["hook_event_name"] = json!("Stop");
    stop["last_assistant_message"] = json!("done");
    let replay = apply(Some(&cancelled), &stop, &root).unwrap();
    assert!(!eligible(&replay));
}
```

- [ ] Run `cargo test -p wk-adapters --test codex_session`; expect missing module failure.
- [ ] Implement a strict state transition table: prompt creates Open, exact duplicate is unchanged, changed duplicate holds, matching Stop completes only a supported consistent sequence, orphan Stop holds, Interrupt cancels permanently, PreCompact holds. Never infer completeness from elapsed time. Reject missing/oversized identifiers and mismatched identity; retain no mismatched content.
- [ ] Add tests for two turns, duplicate Stop, Stop-before-prompt, changed duplicate, foreign cwd, sibling path, absent identity, nested agent, attachment, shell, external MCP, unknown hook and reasoning canary. Unknown input fields that represent unsupported context hold; reasoning fields are excluded from retained data. Known harmless metadata is explicitly allowlisted.
- [ ] Implement only file-tool formats frozen in Task 1. Canonicalize existing reads and parent directories of new files against the enrolled service, reject symlink escapes, track matching pre/post tool calls, hold unmatched or unresolved calls. If no format passed the probe, every tool use holds and capability metadata says no-tool only.
- [ ] Rerun adapter tests to pass; commit `feat: capture attributed Codex visible turns`.

### Task 4: Safe Antigravity source binding

**Files:** Create `crates/agent/src/antigravity_source.rs`; optionally extract unchanged file primitives to `source_file.rs`; modify module declarations and Cursor imports only for that extraction.

**Interfaces:** `bind(enrollment: &Enrollment, payload: &Value, now: i64) -> Result<SourceBinding,String>` and `read_bound(source: &SourceBinding, now: i64) -> Result<Vec<u8>,String>`. Reuse the existing `cursor_source::SourceBinding` shape through the small helper extraction, keeping its serde representation stable. It contains conversation_id, path, device, inode, first_seen, expires_at and policy_hash. Do not reuse Cursor's path-layout rules.

- [ ] Add an expiry test to the new module before implementing the reader:

```rust
#[test]
fn expired_source_is_rejected_before_open() {
    let source = SourceBinding { conversation_id: "synthetic".into(),
        path: "/nonexistent/synthetic".into(), device: 0, inode: 0,
        first_seen: 10, expires_at: 20, policy_hash: "p".into() };
    assert_eq!(read_bound(&source, 20).unwrap_err(), "source_expired");
}
```

- [ ] Run `cargo test -p wk-agent antigravity_source`; require compile failure before implementation.
- [ ] Bind only `$HOME/.gemini/antigravity/brain/<conversationId>/.system_generated/logs/transcript_full.jsonl`, with strict conversation-ID characters, exact single workspace root and enrolled policy. Use descriptor-relative no-follow traversal, file owner/type checks and device/inode identity from the working Cursor implementation. Reject traversal components and files larger than 4 MiB. Never enumerate directories.
- [ ] Add temporary-directory tests for symlinked parent and final path, wrong owner/type where the platform permits, inode replacement, truncation, oversized source and boundary expiry. Compare file metadata before and after bounded reads; inconsistent reads hold.
- [ ] Run `cargo test -p wk-agent` to include Cursor source regression; commit `feat: bind Antigravity transcripts to enrolled callbacks`.

### Task 5: Antigravity visible transcript reconciliation

**Files:** Create `crates/adapters/src/antigravity_session.rs`, `tests/antigravity_session.rs`; export module.

**Interfaces:** `parse(bytes: &[u8]) -> Result<Snapshot,String>`; `Snapshot { consumed: usize, prefix_digest: String, turns: Vec<Evidence> }`; `reconcile(previous: Option<&Snapshot>, bytes: &[u8], stop: Option<&Value>) -> Result<Snapshot,String>`. Session/workspace binding stays in the agent; parser evidence receives the bound session before storage. Parsing alone never produces eligible evidence.

- [ ] Write the incomplete-record test:

```rust
#[test]
fn partial_line_never_completes_a_turn() {
    use wk_adapters::{antigravity_session::parse, native_evidence::eligible};
    let snapshot = parse(b"{\"step_index\":0").unwrap();
    assert_eq!(snapshot.consumed, 0);
    assert!(snapshot.turns.iter().all(|t| !eligible(t)));
}
```

- [ ] Run `cargo test -p wk-adapters --test antigravity_session`; expect missing module failure.
- [ ] Implement complete-line parsing with 256 KiB record/4 MiB input limits and retained-prefix SHA256. Reject invalid complete JSON, duplicate/decreasing step indexes, changed prefix and incompatible schemas. Use USER_INPUT step_index for turn identity; do not use callback counters.
- [ ] Parse the exact wrapper grammar frozen in Task 1. Permit only documented, observed non-content metadata around USER_REQUEST; unknown wrappers and attachments hold. Accumulate only observed visible DONE PLANNER_RESPONSE text. Exclude hidden planner/reasoning fields with a unique canary test proving absence from serialized evidence.
- [ ] Add the critical delayed-Stop fixture: two USER_INPUT turns, repeated zero invocation/execution counters, old Stop arriving after turn two's response. Assert turn two remains ineligible. A stable reread alone must not make this pass incorrectly. Unless Task 1 established an unambiguous terminal binding, `reconcile` leaves all terminal states Open/Held and reports `terminal_attribution_unverified`.
- [ ] Add first-missed-invocation, unknown terminal, explicit error, cancellation, multiple visible responses, incomplete final line, record reordering, prefix rewrite and appended next-turn tests. If correlation was proven, require the observed normal reason NO_TOOL_CALL, exact correlated turn and unchanged snapshot before setting Complete. Future terminal reasons remain held.
- [ ] Add read/edit tool records only from Task 1 fixtures; unknown records hold. Run adapter tests and commit `feat: reconcile bounded Antigravity visible transcripts`.

### Task 6: Durable ingestion, recovery and truthful diagnostics

**Files:** Create `crates/agent/src/native_capture.rs`; modify `daemon.rs`, `main.rs` and doctor output.

**Interfaces:** `receive(dir: &Path, client: Client, payload: &Value) -> Result<Value,String>` async; `reconcile(dir: &Path) -> Result<Value,String>` async; `read(store: &wk_local::LocalStore, key: &str, now: i64) -> Result<Option<Record>,String>`; `status(dir: &Path) -> Result<Value,String>`; `revalidate(dir: &Path, key: &str) -> Result<Record,String>` async. `Record { enrollment: Enrollment, expires_at: i64, source: Option<SourceBinding>, snapshot: Option<Snapshot>, turns: Vec<Evidence>, hold_reason: Option<String> }` has public fields. Derive Clone/Debug/Serialize/Deserialize with unknown-field rejection, including on Snapshot.

- [ ] Add a local-store restart/expiry test before ingestion implementation:

```rust
#[test]
fn replay_does_not_extend_retention() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("capture.db");
    let mut store = wk_local::LocalStore::open(&path, [7; 32], 1_000_000).unwrap();
    store.update("native2:test", 100, 10, |_| Ok(b"cancelled".to_vec())).unwrap();
    drop(store);
    let mut reopened = wk_local::LocalStore::open(&path, [7; 32], 1_000_000).unwrap();
    reopened.update("native2:test", 105, 5, |old| Ok(old.unwrap().to_vec())).unwrap();
    assert!(reopened.read_record("native2:test", 110).unwrap().is_none());
}
```

- [ ] Run `cargo test -p wk-agent native_capture`. Add a failing routing test that confirms enrolled native clients bypass legacy `normalize` and never create `unattributed-turn` events.
- [ ] Authenticate and validate size/enrollment/scope before storing any content. Hash client/device/session/agent identity into `native2:` record keys; Codex turns remain individually identified inside the record. Atomically update using `LocalStore::update`; retain original expires_at and bound total encoded size. On conflicts, retain only a safe hold reason where content cannot be trusted.
- [ ] Hook handling performs no inference; persist bounded evidence and acknowledge. Reconcile only previously bound sources in the existing background loop. Use rotating bounded scans and existing fixed retention purge; mark open Codex turns stale after the configured inactivity threshold without completing them. Expose threshold as a documented constant initially (15 minutes).
- [ ] Add races/restarts tests: concurrent duplicate hooks, cancelled record reopen, expiry under repeated callbacks, payload quota overflow, read failure, changed enrollment, revoked membership, collector outage then recovery. Revocation clears raw content; outage must not fabricate a completed missed turn.
- [ ] Doctor reports enrolled client/version/surface, last callback/reconcile times, complete/partial/held/cancelled counts and reasons, disabled state, no-auto-publication, and missed-initial-sessions undetectable. Do not print message text or paths outside approved workspace.
- [ ] Run `cargo test -p wk-agent` and `cargo test -p wk-local`; commit `feat: persist native capture with bounded recovery`.

### Task 7: Private inference and reviewed submission

**Files:** Create `crates/agent/src/native_process.rs`; modify `process.rs`; extend `crates/server/tests/native_handoff.rs`.

**Interfaces:** `run(dir: &Path) -> Result<usize,String>` async; `validate_draft(dir: &Path, draft: &Value) -> Result<(),String>` async; `matches(draft: &Value, record: &Record, now: i64, policy_revision: u64) -> bool`. Existing public `process::run`, `list`, `submit` APIs remain unchanged. Native drafts include `native_adapter: "codex"|"antigravity"`, source_id, source_digest, turn_id, parser_version, policy_hash, policy_revision and model; Cursor's native_turn_index branch remains intact.

- [ ] Write this regression before the matcher, then add separately named tests for expired records, held turns, parser changes and policy changes:

```rust
#[test]
fn changed_source_digest_blocks_draft() {
    use wk_adapters::native_evidence::*;
    use crate::{native_capture::Record, native_enrollment::Enrollment};
    let record = Record {
        enrollment: Enrollment { client: Client::Codex,
            version: "0.162.0-alpha.2".into(), surface: "cli".into(),
            device_id: "synthetic-device".into(),
            canonical_workspace: "/tmp/synthetic".into(), service: "app".into(),
            policy_hash: "p".into(), enabled: true },
        expires_at: 200, source: None, snapshot: None, hold_reason: None,
        turns: vec![Evidence { client: Client::Codex,
            client_version: "0.162.0-alpha.2".into(), parser_version: 1,
            session: "s".into(), agent: "main".into(), turn: "t".into(),
            prompt: "synthetic".into(), response: "ok".into(),
            messages_complete: true, scope: Scope::Verified,
            terminal: Terminal::Complete, digest: "new".into(),
            files_read: vec![], files_changed: vec![], hold_reason: None }],
    };
    let draft = serde_json::json!({"native_adapter":"codex", "turn_id":"t",
        "source_digest":"old", "parser_version":1, "policy_hash":"p",
        "policy_revision":1, "client_version":"0.162.0-alpha.2"});
    assert!(!matches(&draft, &record, 100, 1));
}
```

- [ ] Add a matching positive control by changing source_digest to new; it must pass the pure matcher. Network membership and source revalidation remain separate mandatory gates in `validate_draft`.
- [ ] Run `cargo test -p wk-agent native_process`; expect missing implementation failure.
- [ ] Use Task 1 eligible evidence only. Revalidate enrollment/source/membership before every inference attempt, after inference and at submission. On membership revocation discard raw evidence and derived drafts; on temporary authorization-network failure block processing/submission without misclassifying it as confirmed revocation. Source mutation invalidates derived drafts.
- [ ] Deduplicate with SHA256 over `(source_id, turn_id, digest, parser_version, client_version, model, policy_hash, policy_revision, prompt_version)`. Supply only visible prompt/response to extraction. Apply existing sensitive-content checks and card validation. A model result cannot set completeness, scope or terminal evidence.
- [ ] Add fake-inference barrier tests: suspend inference, revoke membership or replace source, resume result, assert no retained draft and no successful submission. Add before-submit revocation, expired draft, duplicate processing, inference failure/retry and one ineligible turn beside a valid turn. Reuse existing inference budget rather than permitting unbounded per-client work.
- [ ] Keep reviewer confirmation mandatory; published corrections follow existing revision rules. Extend native_handoff test to prove second identity cannot retrieve a private draft but can retrieve the authorized published revision, and a nonmember cannot retrieve either.
- [ ] Run `cargo test -p wk-agent`, `cargo test -p wk-adapters`, and the existing database harness for `cargo test -p wk-server --test native_handoff`; require the test database configuration used by the current harness, never a production DB. Commit `feat: gate native drafts on current capture evidence`.

### Task 8: Live acceptance, capability labels and cleanup

**Files:** Update `docs/adapters/compatibility.json`, adapter setup docs located by `rg --files docs/adapters`, and `docs/reviews/2026-10-08-multiclient-live-results.md`; create `docs/reviews/2026-10-08-native-capture-acceptance.md`.

**Interfaces:** No new production API. Acceptance consumes installed client, authenticated collector, local inference, reviewer publication and existing MCP search/get.

- [ ] Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`. Run existing PostgreSQL and native-handoff tests with the project's synthetic test service. Record actual failures; do not silently omit environment-dependent suites.
- [ ] In separate synthetic enrolled workspaces, test normal-trust initial turn, continuation, supported read/edit, cancellation, restart, collector outage/recovery and repeated callback delivery. Test an unknown tool/context to prove a hold. Record exact executable versions and hook configuration hashes; no diagnostic trust bypass is permitted for this acceptance.
- [ ] For each completion-capable adapter, produce one visible synthetic decision, verify private evidence, run inference, inspect/correct its private draft, publish as reviewer, then search/get as another authorized identity. Check unique marker, card revision, policy revision and membership isolation. Verify no hidden-reasoning canary or source transcript appears in shared summaries, search, embeddings or MCP.
- [ ] If Antigravity terminal attribution is unresolved, mark private capture partial and publication blocked; do not weaken Task 5 to make the demo pass. If a required read/edit, cancellation or trust test cannot run, mark that row unverified and list the exact blocker.
- [ ] Re-run Cursor's scoped regression to confirm its native draft and revocation behavior remains intact. Claude's prior hook/MCP result is historical evidence only; this change does not upgrade its capture coverage.
- [ ] Update setup instructions with enrollment, normal hook trust, version pinning, disable/revoke, retention, unsupported contexts, missing-hook recovery limits and troubleshooting. Compatibility labels distinguish implementation, automated tests and live acceptance per surface/version; never replace them with a blanket supported flag.
- [ ] Remove owned synthetic hook entries using receipts, stop owned services, delete temporary test credentials and leave the user's subscription login intact. Record cleanup outcomes without credentials. Commit `docs: record native capture acceptance and remaining limits`.

## Self-review and completion definition

Spec mapping: enrollment/config → Task 2; Codex identity/scope/cancellation → Task 3; Antigravity source/turn/parser/terminal → Tasks 1, 4 and 5; private retention/recovery → Task 6; policy/inference/review lifecycle → Task 7; real clients and cross-identity handoff → Task 8. Every Review Focus item is assigned above.

Shared interfaces are declared here; implementation may refine internal helpers without widening the approved scope. Any vendor field needed for completion must first be demonstrated by Task 1 and represented in an actual fixture. Missing evidence results in a documented held/partial implementation, not invented attribution.

This plan is ready for user review. Implementation is not yet started. Recommend native execution in this session because the tasks share evidence and lifecycle interfaces, followed by one independent whole-branch review. A successful build alone does not satisfy live acceptance.
