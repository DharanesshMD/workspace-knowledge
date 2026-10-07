# Cursor Native Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Recover supported Cursor desktop conversations privately and deliver reviewed findings to authorized teammates, with explicit capture gaps and corrected server privacy controls.

**Architecture:** Add one version-specific single-transcript parser, one secure source reader, and one encrypted native-session state machine to the existing collector. Hook receipt records bounded input after authorization; background reconciliation and inference use existing services. Do not build another daemon or discover conversations by scanning global history.

**Tech Stack:** Rust, Tokio/Axum, encrypted SQLite via wk-local, SQLx/PostgreSQL, existing local OpenAI-compatible/Ollama inference, existing MCP bridge.

**Spec:** `docs/superpowers/specs/2026-10-06-cursor-native-capture-design.md` (approved by user on 2026-10-06).

## Global Constraints

- The first supported target is Cursor desktop 3.23.23 on macOS, using local execution in the Agents window.
- Other Cursor versions and surfaces remain unverified until tested.
- No automatic publication is enabled by this work.
- Native Cursor capture is explicit and scoped to a product/service enrollment.
- Existing installations do not silently begin reading transcripts after upgrade.
- Do not read payload content before path and enrollment checks pass.
- Do not scan Cursor storage to find a substitute.
- Preserve the fixed 24-hour raw retention boundary; recovery cannot recreate closed or expired session IDs.
- Do not reread old source text after consent revocation, enrollment change, hold, or expiry.
- Retention cleanup must run independently of remote membership/inference availability.
- A final assistant message alone is not proof of completion.
- Expose the additional native status locally first.
- Test fixtures contain synthetic data only, not developer transcripts or personal paths.
- Do not change global hooks or enroll existing projects without explicit user authorization.

## Clarification included in this plan's review

The spec asks both to move remote membership checks out of hook acknowledgement and to preserve current fail-closed, per-event authorization. Those cannot both be guaranteed with an unconditional durable local queue. Preserve the stronger privacy requirement: check membership at receipt under the existing bounded timeout, then atomically persist and acknowledge. No permission cache, lease, or offline capture permission is introduced. Transcripts are not read and inference does not run in the hook path. On authorization failure, persist a content-free gap and return the client's non-blocking observational response. The cost is that a slow server can prevent capture. Background recovery checks membership before source reads and submission checks it again.

This is an explicit proposed refinement to the acknowledged spec, not a hidden implementation exception.

## Review Focus

1. Repeated identical prompts are distinct turns; replay must not merge them (Task 2).
2. A source with correct filename but replaced inode or symlinked ancestor must not be read (Task 3).
3. Crash immediately after persistence but before acknowledgement must not duplicate a candidate (Task 4).
4. Policy loss during an already-running inference call must prevent the result becoming publishable (Task 5).
5. Concurrent service-scope changes to an activity row must not leak its accumulated paths (Task 1).

## File map and task ordering

1. Server prerequisites: shared card privacy check and immutable activity-session scope.
2. `crates/adapters/src/cursor_session.rs`: strict single-source parser and typed coverage. No filesystem access.
3. `crates/agent/src/cursor_source.rs`: enrollment and safe descriptor-based source access. No inference.
4. `crates/agent/src/cursor_capture.rs`: receipt, atomic native-session state, bounded reconciliation, diagnostics.
5. `crates/agent/src/process.rs`: native candidate generation and submission revalidation.
6. `scripts/cursor-native-integration.py` plus Rust tests: complete synthetic team handoff and real local-model check.
7. Live Cursor exercise, compatibility documentation and final verification report.

Use the existing checkout only to prepare this plan. At execution, create/reuse an isolated worktree per the worktree skill, based on the actual current HEAD, and carry the approved untracked spec, plan and investigation report into it without deleting the originals. Reuse the pinned toolchain through the original checkout's wrapper if needed; do not copy secrets or user state. Run implementation inline unless the user selects delegation. Never stage unrelated files.

## Task 1: Close the two reproduced server privacy defects

**Files:** modify `crates/knowledge/src/activity.rs`, `crates/knowledge/src/lib.rs`, `crates/knowledge/src/lifecycle.rs`, `crates/knowledge/Cargo.toml`, `crates/server/src/lib.rs`; test `crates/knowledge/tests/access.rs`, `crates/server/tests/http.rs`, `scripts/integration.py`.

**Interfaces:** preserve public API shapes. Add private `validate_shared_body(body: &serde_json::Value) -> Result<(), Error>` in wk-knowledge, using the existing wk-summarize sensitive detector. Adding wk-summarize to wk-knowledge introduces no cycle: wk-summarize depends on contracts, not knowledge. Keep authorization checks and content-free errors.

- [ ] Add regressions to existing access tests. Use the existing `setup()` and `card()` helpers:

```rust
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn sensitive_correction_is_rejected_without_new_revision() {
    let (k, a, _, p) = setup().await;
    let original = card("privacy", vec!["backend"]);
    let saved = k.submit(&a, &p, original).await.unwrap();
    let mut corrected = card("privacy", vec!["backend"]);
    corrected.base_revision = Some(saved.revision as u64);
    corrected.rationale = "password = SYNTHETIC-NOT-A-CREDENTIAL".into();
    assert!(k.correct(&a, &p, saved.id, corrected).await.is_err());
    assert_eq!(k.get(&a, &p, saved.id).await.unwrap().revision, saved.revision);
}
```

Add tests that submit activity under backend+frontend and attempt updating the same session to backend-only; expect conflict and unchanged scope/files. Repeat with two concurrent updates and a non-owner RLS reader. Reversed service ordering is equal scope; normalize/deduplicate services before comparison. A new session with a narrower scope succeeds without old paths. Seed a legacy sensitive draft directly in the disposable owner database and assert publication is rejected and published remains false. Add HTTP checks that the rejected canary never appears in errors.

- [ ] Run regressions before implementation:

```sh
./scripts/cargo test -p wk-knowledge --test access --locked -- --ignored --test-threads=1
```

Expected: the new privacy regressions fail against current code. Use only an explicitly disposable `/wk_test` database; never infer credentials from a valued database.

- [ ] Implement scope immutability in the existing atomic upsert:

```sql
ON CONFLICT(org,product,user_id,session_id) DO UPDATE SET
  files=(SELECT COALESCE(array_agg(f),'{}') FROM
    (SELECT DISTINCT f FROM unnest(activity.files||EXCLUDED.files) f ORDER BY f LIMIT 200) x),
  intent=EXCLUDED.intent, status='active', last_seen=now(), expires_at=EXCLUDED.expires_at
WHERE activity.services @> EXCLUDED.services
  AND EXCLUDED.services @> activity.services
```

Require one affected row; otherwise return `Error::Conflict`. Never replace `services` while retaining paths. Validate ended-session updates against the same scope contract.

- [ ] Apply shared privacy validation in submit and correct before mutation; in publish, lock and authorize the current card, screen its JSON body, then publish and build the graph in the same transaction. Do not inspect unauthorized content or insert a second pool acquisition while holding the first transaction. Protect against correction/publication races by checking the locked revision.

```rust
fn validate_shared_body(body: &serde_json::Value) -> Result<(), Error> {
    if wk_summarize::sensitive(&body.to_string()) { Err(Error::Invalid) } else { Ok(()) }
}
```

Do not retroactively modify or delete existing cards automatically. Legacy sensitive drafts remain unpublishable until corrected.

- [ ] Run access, HTTP, and integration suites; expect all existing and new tests to pass. Commit only these files with message `fix: preserve activity scopes and screen corrected cards`.

## Task 2: Parse one Cursor transcript without filesystem discovery

**Files:** create `crates/adapters/src/cursor_session.rs`, `crates/adapters/tests/cursor_session.rs`, `crates/adapters/tests/fixtures/cursor-session-v3.jsonl`; modify `crates/adapters/src/lib.rs` and `crates/adapters/src/cursor_history.rs` only to share safe visible-text normalization where behavior matches.

**Interfaces:** the new module exports the following serde-serializable types and functions:

```rust
pub enum ScopeState { Verified, Unknown, Violated }
pub enum MessageState { Partial, Complete }
pub struct CursorTurn {
    pub index: u64,
    pub prompt: String,
    pub response: String,
    pub files_read: Vec<String>,
    pub files_changed: Vec<String>,
    pub messages: MessageState,
    pub scope: ScopeState,
    pub digest: String,
}
pub struct CursorSnapshot {
    pub consumed: usize,
    pub prefix_digest: String,
    pub turns: Vec<CursorTurn>,
    pub held_reason: Option<String>,
}
pub fn parse_transcript(bytes: &[u8], binding: &wk_scope::enrollment::Binding,
                        service: &str) -> Result<CursorSnapshot, String>;
```

Types derive Serialize/Deserialize and equality/debug traits needed by tests. Error and hold strings come from a fixed allowlist, never input text. Initial bounds: 4 MiB source, 256 KiB record, 100 turns, 768 KiB serialized retained native record. Crossing a bound holds the session.

- [ ] Add a synthetic fixture matching observed records: role=user/assistant, message.content arrays, text, Read/StrReplace tool_use, and `{"type":"turn_ended","status":"success"}`. The terminal form was observed in all three live probe turns. No real transcript is checked into Git.

```rust
#[test]
fn assistant_text_without_terminal_record_is_partial() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("frontend")).unwrap();
    let b = wk_scope::enrollment::Binding::approve(
        root.path(), "demo", vec!["frontend".into()]).unwrap();
    let bytes = b"{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}}\n{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"world\"}]}}\n";
    let s = wk_adapters::cursor_session::parse_transcript(bytes, &b, "frontend").unwrap();
    assert_eq!(s.turns[0].messages, wk_adapters::cursor_session::MessageState::Partial);
}
```

Add table-driven fixtures for repeated identical user messages (two indices), injected context wrappers, reasoning blocks, unknown records, interrupted/error terminal status, incomplete trailing JSON, malformed complete JSON, user-without-response, external paths, attachment markers, shell/MCP calls, and subagent/compaction records. Unknown tools yield Unknown scope; outside paths yield Violated. Scope uncertainty is session-wide and monotonic, so later text-only turns cannot launder earlier ambiguous context.

- [ ] Run `./scripts/cargo test -p wk-adapters --test cursor_session --locked`; expect missing-module failure before implementation.
- [ ] Implement a pure parser. Consume only through the final newline; track every structural record; finalize only supported terminal success with one prompt and a nonempty response. No terminal status means Partial. Preserve read and changed path metadata after validation. Strip only recognized injected wrappers, retaining actual developer text. Unknown user-context forms hold rather than silently discard. Use transcript position for identity and SHA-256 of canonical turn data for edit detection.

```rust
let consumed = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
let prefix_digest = format!("{:x}", sha2::Sha256::digest(&bytes[..consumed]));
```

Do not borrow the broad importer's rule that any final assistant response implies completeness, or its behavior of simply dropping outside paths.

- [ ] Run all adapter tests, including existing history tests. Expect no change to explicit history import behavior. Commit `feat: parse scoped Cursor session transcripts`.

## Task 3: Explicit enrollment and secure source access

**Files:** create `crates/agent/src/cursor_source.rs`; modify `crates/agent/src/main.rs`, `crates/agent/src/state.rs`; unit tests in `cursor_source.rs`.

**Interfaces:** expose internally:

```rust
pub struct CursorConsent {
    pub policy_hash: String,
    pub version: String,
    pub surface: String,
    pub transcript_root: std::path::PathBuf,
}
pub struct SourceBinding {
    pub conversation_id: String,
    pub path: std::path::PathBuf,
    pub device: u64,
    pub inode: u64,
    pub first_seen: i64,
    pub expires_at: i64,
    pub policy_hash: String,
}
pub fn enroll(dir: &std::path::Path) -> Result<(), String>;
pub fn validate_hook(config: &crate::state::Config, consent: &CursorConsent,
    payload: &serde_json::Value) -> Result<String, String>;
pub fn bind(config: &crate::state::Config, consent: &CursorConsent,
    payload: &serde_json::Value, now: i64) -> Result<Option<SourceBinding>, String>;
pub fn read_bound(source: &SourceBinding, now: i64) -> Result<Vec<u8>, String>;
```

Add `wk-agent --state-dir DIR cursor-enroll --yes`, limited to macOS native source support. Enrollment uses current local approval and server membership; displays a fixed boundary description and writes owner-private consent. Derive the default Cursor source root from the user's home through an existing safe platform mechanism, not from hook input. No arbitrary CLI path option in production. Test functions receive explicit temporary allowed roots through private helpers. Existing configs stay valid and consent defaults absent.

- [ ] Add unit tests for missing consent, version mismatch, multiroot input, wrong session filename, project slug mismatch, parent traversal, symlink ancestor/final file, nonregular file, file identity change and expiry. Use temporary owned fixtures; test ownership validation through a factored metadata predicate where unprivileged tests cannot create other-owner files. Keep live ownership behavior unclaimed on untested platforms.

```rust
#[test]
fn expired_source_is_not_opened() {
    let s = SourceBinding { conversation_id: "test".into(),
        path: "/nonexistent/do-not-open".into(), device: 1, inode: 1,
        first_seen: 1, expires_at: 2, policy_hash: "p".into() };
    assert_eq!(read_bound(&s, 2).unwrap_err(), "source_expired");
}
```

- [ ] Run `./scripts/cargo test -p wk-agent --locked cursor_source`; expect the new checks to fail before implementation.
- [ ] Validate exactly one canonical workspace root matching the configured workspace or enrolled service root. Require Cursor 3.23.23 and a bounded valid conversation identifier. Require `<allowed-root>/<workspace-slug>/agent-transcripts/<id>/<id>.jsonl` for the observed target layout; verify the slug corresponds to this approved workspace, but never treat it as sufficient authorization by itself. A null transcript_path returns None, not a search request.
- [ ] On macOS open each path component with `openat` plus `O_NOFOLLOW`, using an owned trusted directory descriptor. Reject `..`, unexpected separators and symlink ancestors. Use `fstat` for owner/type/device/inode, bounded reads, then identity/length rechecks. Close descriptors with RAII. Expiry and consent checks occur before opening. Other platforms return an explicit unsupported native-source result.
- [ ] Run the tests and compilation for existing platform-gated code. Commit `feat: enroll and securely bind Cursor transcript sources`.

## Task 4: Atomic native-session recovery and coverage diagnostics

**Files:** create `crates/agent/src/cursor_capture.rs`; modify `crates/agent/src/daemon.rs`, `crates/agent/src/main.rs`, `crates/local/src/lib.rs`, `crates/local/tests/scanning.rs` and adapter config generation.

**Interfaces:** new module functions:

```rust
pub async fn receive(dir: &std::path::Path, payload: &serde_json::Value)
    -> Result<serde_json::Value, String>;
pub async fn reconcile(dir: &std::path::Path) -> Result<serde_json::Value, String>;
pub fn status(dir: &std::path::Path) -> Result<serde_json::Value, String>;
```

Store a serde-serializable `NativeSession` under `native:<sha256(workspace fingerprint,service,conversation)>`. Fields: version, optional SourceBinding, policy_hash, expires_at, prefix_digest, consumed bytes, Vec<CursorTurn>, bounded pending hook observations, last_checked time, fixed-label health/hold reason. Put checkpoint and parsed turns in the SAME encrypted LocalStore::update record. Existing guarded update prevents reopening after expiry/reset. Do not use plaintext JSON for transcript paths or content.

- [ ] Add LocalStore `purge_expired(now: i64) -> Result<usize,String>` and generic `next_ids(namespace: &str, prefix: &str, now: i64, limit: u32) -> Result<Vec<String>,String>`; keep next_session_ids as its compatibility wrapper. Only opaque namespace/cursor IDs are plaintext. Test expiry without any membership call and fair pagination across held sessions.

```rust
#[test]
fn cleanup_does_not_need_a_server() {
    let d = tempfile::tempdir().unwrap();
    let mut s = wk_local::LocalStore::open(&d.path().join("capture.db"), [7;32], 4096).unwrap();
    s.update("native:test", 100, 10, |_| Ok(b"synthetic".to_vec())).unwrap();
    assert_eq!(s.purge_expired(110).unwrap(), 1);
    assert!(s.update("native:test", 111, 10, |_| Ok(vec![])).is_err());
}
```

- [ ] Add state-machine tests using temporary encrypted stores and the Task 2/3 types: duplicate receive, duplicate reconciliation, simulated restart after commit/before acknowledgement, partial line growth, changed prefix, same-size replacement, expiry, policy mismatch, failed membership, unavailable file, and many sessions rotating fairly. Inject authorization results in a private test seam; production always calls network::policy.
- [ ] Run targeted local/agent tests; expect failures for absent methods/state handling.
- [ ] In receive: authenticate IPC, check local consent/version/scope, perform bounded current membership check, then update the encrypted native record atomically. Store only allowlisted hook fields. For opted-in Cursor route before legacy normalize, so postToolUse/stop can bind recovery even without visible text. Non-opted-in behavior remains unchanged. Limit pending observations to 100 and the whole record to 768 KiB; overflow creates a content-free hold.
- [ ] In the existing worker, purge first even on global capture-gap/network failure; then reconcile before inference. Read at most 10 bound sources per tick, each at most 4 MiB. Validate prior consumed-prefix digest before and after parsing. Recheck local binding and membership before committing. Any uncertain or violated session invalidates all its pending candidates. Persist only approved visible text and minimal evidence; deterministic input sensitivity screening holds the session before persistence.
- [ ] Native turns use transcript position as authoritative identity. Direct hook messages never produce independent native candidates. Callback generation IDs are evidence only; associate them with a transcript turn only when unambiguous, otherwise keep them unassociated. This eliminates duplicate callback/transcript summaries without merging identical turns.
- [ ] Add postToolUse to typed source-binding handling and preserve observational outputs. Do not register reasoning hooks. Read-path/tool metadata handling must not silently discard outside paths. Hook failures never block Cursor or request another model turn.
- [ ] Doctor/health expose bounded counts and last-check state without messages, paths, IDs, credentials or email. Compute stale from failed/missed worker checks, not time since the last user message. Report waiting_for_binding and an explicit missed-initial-session limitation. Reset acknowledges native records through the existing guards; pause or policy mismatch closes/holds sources before another read.
- [ ] Run local, capture and agent suites. Commit `feat: recover bound Cursor sessions with durable coverage state`.

## Task 5: Generate reviewable native candidates and revalidate on submission

**Files:** modify `crates/agent/src/process.rs`, `crates/agent/src/cursor_capture.rs`, `crates/agent/src/main.rs`; add unit tests in process.rs; update `crates/summarize/tests/privacy.rs` only if shared card construction changes.

**Interfaces:** add private `native_eligible(turn: &CursorTurn, held: bool) -> bool`. Native drafts carry source_id, source_digest, native_turn_index, source_boundary, policy_hash, model, and existing card. Legacy drafts remain readable. Do not allow a caller-provided agent_id string to grant native completeness.

```rust
fn native_eligible(turn: &CursorTurn, held: bool) -> bool {
    !held && turn.messages == MessageState::Complete && turn.scope == ScopeState::Verified
}
```

- [ ] Write tests for complete+verified acceptance; complete+unknown/violated rejection; partial rejection; identical turns with different indices; source mutation after drafting; expiry during inference; revoked membership; and replay after successful submission. Assert that raw transcript and paths never enter card metadata or diagnostics.
- [ ] Run `./scripts/cargo test -p wk-agent --locked process`; expect missing eligibility/native flow failures before implementation.
- [ ] Keep legacy managed-chat logic; process only eligible native turns. Build draft identity from native session ID, turn index+digest, policy revision and model/prompt version. Recheck source and current policy after inference before saving the candidate. Use the earlier of existing one-hour draft expiry and the source's remaining lifetime. Reject zero/negative remaining lifetime.
- [ ] At `submit --reviewed`, reread and reconcile the native source under current authorization, verify the reviewed turn's digest, all coverage dimensions and current consent, then submit. Source disappearance, changes or unknown scope require a new review. Do not require that later safe turns leave the entire transcript byte-identical: compare the reviewed turn and session safety state, not only a whole-file hash.
- [ ] Include validated workspace-relative file observations in the reviewed card for overlap lookup; retain proposed lifecycle. Trusted code/test verification remains independent. Run agent and summarize tests and commit `feat: review and submit verified native capture candidates`.

## Task 6: Prove the private-to-shared team handoff

**Files:** create `scripts/cursor-native-integration.py`; extend `crates/knowledge/tests/access.rs` and `scripts/integration.py` where reusable fixtures exist. Add an executable Rust integration test under `crates/server/tests/native_handoff.rs` for API identity injection without weakening production authentication.

**Interfaces:** the Python harness accepts an explicit disposable WK_TEST_DATABASE_URL ending in /wk_test, an explicit loopback inference endpoint/model, and paths to the freshly built binaries. Use subprocess argument arrays, random synthetic tokens held in process memory and separate 0700 state directories. Never print tokens or use real developer history. The Rust server test uses the existing router(knowledge, tokens) to provide A(owner), R(reviewer), B(teammate) and U(unauthorized) identities; no production dev-mode identity shortcuts.

- [ ] Write the handoff regression around a native-generated reviewed card fixture:

```python
assert owner_raw_canary not in json.dumps(shared_card)
assert reviewer_publish_status == 200
assert teammate_search_ids == [shared_card_id]
assert shared_card_id in teammate_overlap_card_ids
assert unauthorized_search_ids == []
assert unauthorized_get_status in (403, 404)
```

The harness obtains each variable from actual API results; fail rather than fabricate them. Add a unique canary to unrelated synthetic source content and assert the unsafe source is held rather than shared. For a safe source use a visible design decision, e.g. retries require idempotency, and assert its meaning survives manual review of real-model output.

- [ ] Exercise deterministic model fixtures first for schema/error paths, then a configured real local model. Do not download a model or silently switch to a cloud provider. If no local model is available, preserve passing deterministic results and mark the real-model acceptance gate blocked, not complete.
- [ ] Run the new handoff test with the full existing database suite. Test a denied B service and a second organization. Confirm no raw source API exists and no raw canary is in stored cards/revisions/feedback or MCP output. Check overlap notices carry the correct card revision or update their contract to include revision before claiming that requirement met.
- [ ] Build binaries and run the Python harness with native keyring storage. Do not add a plaintext-key production fallback to facilitate tests. Record a keyring/OS permission blocker if encountered.
- [ ] Commit `test: cover native capture publication and team retrieval` after passing deterministic checks, leaving real-model/live acceptance explicitly separate if unavailable.

## Task 7: Live verification, documentation and final review

**Files:** update `docs/adapters/setup.md`, `docs/adapters/compatibility.json`, `docs/architecture/completion-matrix.md`, `README.md`; create `docs/reviews/2026-10-06-cursor-native-capture-results.md`.

- [ ] In an isolated synthetic workspace with a service subfolder, enroll the new collector and install only owned project hooks. Use desktop automation through cua_repl. Never edit global hooks. Record Cursor version, surface, model, event counts, source-binding recovery and all failure labels.
- [ ] Perform a file-edit turn and two no-tool turns; verify returned hook marker context as well as source recovery. Restart the collector, append another turn, and confirm no duplicate candidates. Cancel an in-progress turn and confirm it is not complete. Resume safely and verify held/partial status transitions follow the spec. Revoke membership and confirm capture and source reads stop. Restore test permissions only to finish cleanup, not to reopen held sessions.
- [ ] Verify diagnostics while the server is unavailable, and verify TTL cleanup with test-controlled clock in unit tests rather than waiting a day. Live compaction/subagent behavior is unsupported until separately tested and must remain held/partial.
- [ ] Review the actual locally generated card, publish through the reviewer identity, and show B retrieving it through MCP and receiving a file overlap notice. Distinguish API injection, real native hooks, and model quality evidence in the results report.
- [ ] Run final checks once after the last fixes:

```sh
./scripts/cargo fmt --all --check
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
./scripts/cargo test --workspace --locked
./scripts/cargo test -p wk-knowledge --test access --locked -- --ignored --test-threads=1
./scripts/cargo test -p wk-server --test native_handoff --locked -- --ignored
./scripts/cargo build --workspace --locked
python3 scripts/integration.py
git diff --check
```

Run web build/browser checks only if console code changes. Inspect dependency audit if dependency graph changes in Task 1; retain the existing documented unused-rsa exception only if its non-reachability remains verified.

- [ ] Update compatibility with exact test evidence; never mark all Cursor surfaces or native clients certified. Describe configured-but-unbound capture and recovery limits. Mark Codex/Antigravity as adapter-not-implemented, not vendor-hooks-unavailable. Include rollback instructions for native consent and owned hooks, and the fixed 24-hour source lifetime.
- [ ] Perform a final security review of path reads, source snapshots, auth boundaries, content screening, revision races and tests. Fix important findings and rerun the affected checks. Leave a concise results report with implemented features, observed live behavior, outstanding gates and risks. Disable test hooks and stop only test services created for this work. Commit `docs: record Cursor capture limits and acceptance evidence`.

## Self-review and execution handoff

Spec coverage: security prerequisites→1; parsing/correlation→2; consent/source safety→3; persistence/TTL/recovery/diagnostics→4; summary/review boundaries→5; shared handoff→6; live checks/rollout→7. Every Review Focus condition has a owning task. One explicit spec refinement remains the request-time membership check described above; plan approval includes that choice.

Task 2/3 types feed Task 4. Task 4's NativeSession is the sole authority for Task 5 native drafts. Task 6 tests the real Task 5 output contract. Independent storage cleanup occurs before any Task 4/5 remote call. No interface grants completeness based on a self-reported native agent label.

Recommended execution: native/inline implementation because parser, source binding, session state and candidate validation share a small set of security-critical invariants. An independent review after integration is valuable. Subagent-driven execution is an alternative if the user prefers per-task reviews and the additional coordination cost. No implementation starts until this plan is reviewed and an execution method is selected.
