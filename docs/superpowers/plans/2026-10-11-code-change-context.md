# Code Change Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Preserve task boundaries and record evidence before marking a task complete.

**Goal:** Give a developer's agent contributor names, confirmed reasons and known impact references for the code it is about to change, with developer confirmation required for missing, stale or high-risk context in certified edit workflows.

**Architecture:** Add explicit repository/contribution/rationale records to the existing PostgreSQL knowledge core and observe approved Git snapshots locally. One deterministic assembler serves REST, collector, CLI, MCP and portal consumers. Separate short-lived, single-use preflight state and a version-tested local edit adapter from the existing nonblocking capture pipeline.

**Tech Stack:** Rust, serde/schemars, SQLx/PostgreSQL, existing encrypted local storage, Axum, rmcp, React/TypeScript/Vite, Playwright, Python acceptance harnesses and Git CLI. Reuse dependencies; no new model service or repository/vector database.

**Spec:** [Approved architectural design](../specs/2026-10-11-code-change-context-design.md). The user approved proceeding on 2026-10-11 and selected confirmation for missing/stale/high-risk context. This plan is ready for review; product implementation has not started.

**Workspace:** `/path/to/isolated/workspace-knowledge`, branch `codex/cursor-native-capture`, design commit `d4442ef`. Reuse this clean isolated checkout after verifying status. Preserve `/path/to/original/workspace-knowledge` and its unrelated untracked documentation. Commit with the configured identity and `git commit -s`; do not merge, release, message contributors, or change production/client installations for acceptance.

## Global Constraints

- "require developer confirmation before proceeding when rationale or dependencies are missing, outdated, or high-risk".
- "Default bound: 20 targets, 20 cited records, 64 KiB request and 16 KiB response".
- "Bound graph traversal to one reviewed dependency hop and at most 20 references in V1; disclose truncation."
- "a two-second combined interactive target and a five-second hard request deadline"; targets are not measured guarantees.
- "Acknowledgement is single-use and expires after five minutes, as does the brief."
- "Keep brief/acknowledgement payloads for at most 24 hours"; existing minimal audit retention is 90 days and must not be broadened.
- "V1 has no offline protected-edit override"; authorization denial is never an overridable risk.
- "Report dependency coverage as `reviewed records only` and therefore incomplete in V1."
- "do not upload code, patches, raw tool inputs, raw prompts, commit messages, absolute paths, Git emails, or unscreened author names."
- Identity mapping alone never proves a commit's contributor. Require a reviewed, developer-confirmed revision/target contribution attestation before showing a trusted contributor name.
- Only published/current/authorized records feed shared retrieval; no owner summaries, raw journals, hidden-service names/counts, or broad member directory.
- Preserve existing MCP contracts, authentication audiences, native capture capability labels and nonblocking capture errors. No supported-version/protection claim without live acceptance.
- The local trust boundary is a cooperating agent and trusted desktop. Browser authentication cannot independently prove human presence against an agent controlling the browser/OS.

## Review Focus

- A forged author field matches an already-verified profile: require a commit-specific contribution attestation, not merely an identity map (Task 2).
- A Windows case collision, symlink, worktree Git directory or command-looking path escapes enrollment: canonicalize and validate before fixed-argument Git reads; reject ambiguity (Task 3).
- Permission narrowing, profile removal or a changed dependency happens between acknowledgement and edit: reject stale revision sets and recheck snapshot immediately before dispatch (Tasks 4, 5, 8).
- Browser logout/product switch occurs with an outstanding brief/confirmation request: abort obsolete requests and refuse a stale or foreign acknowledgement (Task 7).
- The hook crashes, times out or sees an unsupported tool shape: stop the covered edit or withhold the protected label; never inherit capture's success fallback (Task 8).

## File and interface map

Create focused modules rather than extending large entry points with business logic:

| New file | Responsibility |
| --- | --- |
| `crates/contracts/src/change_context.rs` | Strict, versioned retrieval DTOs, validation and deterministic rendering |
| `crates/knowledge/src/code_provenance.rs` | Repository registration, profiles, mappings, contributions, rationale anchors and maintainers |
| `crates/knowledge/src/change_context.rs` | Authorized source selection and context assembly |
| `crates/knowledge/src/preflight.rs` | Short-lived briefs, acknowledgement and atomic receipt consumption |
| `crates/server/src/change_routes.rs` | Thin REST/portal handlers and browser-only human mutations |
| `crates/agent/src/repository.rs` | Safe bounded read-only Git observation, local target hashes and operation fingerprint |
| `crates/agent/src/protection.rs` | Enrollment, local pending operations and before-edit control |
| `crates/adapters/src/change_guard.rs` | Pure version/tool-specific pre-edit parse and decision output |
| `web/src/CodeContext.tsx`, `web/src/CodeAttribution.tsx` | Brief/confirmation and owner attribution/reviewer flows |

Add migrations `0014.sql` for repository/profile/mapping registration, `0015.sql` for contributions/anchors/maintainers, and `0016.sql` for preflight state. Update migration registration/readiness to the current last migration after each task, ending at 16; update restricted grants, schema/restore assertions and deployment instructions in the tasks introducing each migration. Never edit an already-applied migration to add a later task's tables. Tests stay in the existing package layouts; agent tests are module unit tests because `wk-agent` is currently a binary crate.

Shared wire types in `wk_contracts::change_context`:

```rust
// Derive Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema;
// deny_unknown_fields on structs. Decision also derives Copy and Eq.
pub struct CodeTarget { pub path: String, pub start: Option<u32>, pub end: Option<u32> }
pub struct ChangeContextRequest {
    pub schema_version: u32,          // exactly 1
    pub repository_id: uuid::Uuid,
    pub base_revision: String,        // full 40/64 lowercase-hex Git object ID
    pub targets: Vec<CodeTarget>,
    pub operation_class: String,      // read_context, edit, write, delete, multi_edit
    pub operation_handle: Option<uuid::Uuid>, // opaque; never an edit-body digest
    pub budget_bytes: u32,            // 1024..=16384; gaps/reasons reserved first
}
pub enum Decision { ContextAvailable, NeedsReview, Denied, Unavailable }
pub struct SourceRef {
    pub kind: String, pub id: String, pub revision: i64,
    pub code_revision: Option<String>, pub assurance: String,
    pub freshness: String, pub observed_at_ms: i64,
}
pub struct ContextItem {
    pub role: String,                 // contributor, rationale_author, submitter,
                                     // maintainer, activity_reporter, impact, test
    pub user_id: Option<String>, pub display_name: Option<String>,
    pub contact_url: Option<String>, pub text: String,
    pub matched_by: String, pub sources: Vec<SourceRef>,
}
pub struct ChangeContextBrief {
    pub schema_version: u32, pub repository_id: uuid::Uuid,
    pub base_revision: String, pub targets: Vec<CodeTarget>,
    pub decision: Decision, pub reasons: Vec<String>, pub gaps: Vec<String>,
    pub coverage: String, pub observation_assurance: String,
    pub items: Vec<ContextItem>, pub truncated: bool,
    pub brief_id: Option<uuid::Uuid>, pub expires_at_ms: Option<i64>,
}
// Preserve snake_case serialized enums; response is not an instruction block.
// Implement ChangeContextRequest::validate(&self) -> Result<(), String>.
```

Knowledge interfaces introduced by the named tasks, all returning `Result<_, wk_knowledge::Error>`:

- Task 1: `configure_repository(&Actor, &str, RepositoryInput) -> RepositoryRecord`; `put_code_profile(&Actor, &str, ProfileInput) -> ProfileRecord`; `configure_code_mapping(&Actor, &str, MappingInput) -> MappingRecord`.
- Task 2: `propose_contribution(&Actor, &str, ContributionInput) -> ContributionRecord`; `confirm_contribution(&Actor, &str, Uuid, i64) -> ContributionRecord`; `review_contribution(&Actor, &str, Uuid, i64) -> ContributionRecord`; `propose_rationale_anchor(&Actor, &str, AnchorInput) -> AnchorRecord`; `confirm_rationale_anchor(&Actor, &str, Uuid, i64) -> AnchorRecord`; `review_rationale_anchor(&Actor, &str, Uuid, i64) -> AnchorRecord`; `configure_code_maintainer(&Actor, &str, MaintainerInput) -> ()`.
- Task 4: `change_context(&Actor, &str, ChangeContextRequest) -> ChangeContextBrief`.
- Task 5: `create_preflight(&Actor, &str, ProtectedIdentity, ChangeContextRequest) -> ChangeContextBrief`; `get_preflight(&Actor, &str, Uuid) -> ChangeContextBrief`; `acknowledge_preflight(&Actor, &str, Uuid) -> ()`; `consume_preflight(&Actor, &str, ProtectedIdentity, Uuid, Uuid) -> ()`.

Define provenance input/record structs in `code_provenance.rs`, not in HTTP handlers. Records derive Debug/PartialEq for assertions. Every metadata mutation increments its own record revision and returns the new record where confirmation/review chains need it. Define `ProtectedIdentity { device_id: Uuid, binding_id: Uuid, security_epoch: i64 }`, deriving Clone/Debug, in `preflight.rs`; construct it only from validated signed-device authentication plus an authorized binding lookup, never directly from a client body.

## Test setup and execution

Use `scripts/cargo`, which selects the bundled toolchain when available. All database tests require an **owned disposable** PostgreSQL database named `wk_test` and `WK_TEST_DATABASE_URL`; use the repository's existing test container only after inspecting ownership. Do not echo credentials. DB fixtures copy the `setup`/`body` pattern from `crates/knowledge/tests/team.rs`: unique organization, Alice and Bob with backend/frontend access, Reader with backend only. Add an administrator grant via the existing protected provisioning helper for registration tests. Retain RLS tests through the restricted `wk_app` runtime role, not just the migration owner.

The assertion snippets below belong inside these fixtures; variables identify fixture entities introduced in the owning step. No production card, identity, repository or conversation is needed. Each task follows red → minimal implementation → green → signed commit. Missing symbols are acceptable only as the intentional initial red result; database availability/configuration errors are not a passing red test.

### Task 1: Strict contracts and scoped repository/profile storage

**Files:** Create contract module, `migrations/server/0014.sql`, provenance module, `crates/contracts/tests/change_context.rs`, `crates/knowledge/tests/code_provenance.rs`; modify both crate `lib.rs` files, migration registration/readiness, `deploy/scripts/application-role.sql` and schema snapshots if exported schemas change.

**Inputs/outputs:** Introduces the DTOs above. `RepositoryInput { id: Uuid, services: Vec<String>, high_risk_paths: Vec<String>, contact_hosts: Vec<String>, base_revision: Option<i64> }`; `ProfileInput { display_name: Option<String>, contact_url: Option<String>, services: Vec<String>, base_revision: Option<i64> }`; `MappingInput { repository_id: Uuid, user_id: String, identity_reference: String, proof_reference: String, services: Vec<String>, base_revision: Option<i64> }`. Records include ID, revision and the corresponding fields; profile owner derives from Actor. No email/name-based mapping input. Optional display/contact supports withdrawal.

- [ ] Write request-validation regressions for unknown fields, empty/21 targets, traversal/absolute/control-character paths, leading option-like paths, inconsistent ranges, malformed object IDs and excessive budgets. Require parse/validation failure without echoing rejected data.

```rust
let mut input = serde_json::json!({
    "schema_version":1,"repository_id":uuid::Uuid::new_v4(),
    "base_revision":"a".repeat(40),"targets":[{"path":"api/retry.rs","start":1,"end":4}],
    "operation_class":"edit","operation_handle":null,"budget_bytes":16384
});
let valid: ChangeContextRequest = serde_json::from_value(input.clone()).unwrap();
assert!(valid.validate().is_ok());
input["targets"][0]["path"] = "../private.env".into();
assert!(serde_json::from_value::<ChangeContextRequest>(input.clone()).unwrap().validate().is_err());
input["raw_prompt"] = "RAW_CANARY".into();
assert!(serde_json::from_value::<ChangeContextRequest>(input).is_err());
```

- [ ] Run `scripts/cargo test -p wk-contracts --test change_context` and the ignored provenance DB suite; expect missing contract/methods before implementation.
- [ ] Implement validation and storage. Tables: `code_repositories` (org/product/id/services/revision/high_risk_paths/contact_hosts/revoked), `code_profiles` (org/product/user_id/services/revision/display_name/contact_url/revoked), `code_mappings` (org/product/id/repository_id/user_id/identity_reference/proof_reference/services/revision/reviewer/revoked). Add composite scope keys/FKs, positive revisions, bounded nonempty services, owner profile checks, exact HTTPS contact host validation without credentials, and compare-and-set updates. Read visibility requires every service in current membership; repository/mapping mutations require protected operator authority with scope ceiling. A profile never grants membership.

```sql
-- Apply this read condition to each provenance table in addition to SQL filters.
code_repositories.org = current_setting('wk.org',true)
AND EXISTS (SELECT 1 FROM memberships m
 WHERE m.org=code_repositories.org AND m.product=code_repositories.product
 AND m.user_id=current_setting('wk.user',true)
 AND m.services @> code_repositories.services)
```

- [ ] Add RLS/grants denying direct unauthorized writes/deletes; runtime gets only operations used by these modules. Test profile narrowing, invalid contact hosts, attempted other-user profile edit, cross-org access, unauthorized operator mutation and repository rebinding conflicts.
- [ ] Run targeted tests, schema checks and formatter; commit `feat: add scoped code provenance contracts` with `-s`.

### Task 2: Confirmed contribution/rationale anchors and explicit maintainers

**Files:** Create `migrations/server/0015.sql`; extend provenance module and its tests; add `crates/knowledge/tests/code_anchors.rs`; extend `crates/server/src/admin_cli.rs` and `crates/server/src/main.rs` for protected repository/mapping/maintainer configuration. Register migration 15, update readiness/grants/schema assertions, and preserve applied migration 14. No HTTP operator provisioning endpoint.

**Inputs/outputs:** `ContributionInput { repository_id: Uuid, mapping_id: Uuid, code_revision: String, targets: Vec<CodeTarget> }`; `AnchorInput { repository_id: Uuid, card_id: Uuid, card_revision: i64, code_revision: String, targets: Vec<CodeTarget>, contribution_id: Option<Uuid> }`; `MaintainerInput { repository_id: Uuid, user_id: String, targets: Vec<CodeTarget>, services: Vec<String>, base_revision: Option<i64> }`. Records have ID/revision, authenticated confirmer/time, reviewer/time and revocation state. The contribution's asserted owner comes from its reviewed mapping, and only that owner can confirm it. The rationale confirmer is the card's actual author; third-person reporting stays labelled a report.

- [ ] Write DB tests where Bob submits a candidate for Alice, Bob cannot confirm Alice's contribution/intent, Alice confirms, Reader cannot review, Bob as scoped reviewer reviews, and unpublished/withdrawn/stale cards cannot produce shared anchors. A copied author field with a valid mapping but no contribution remains unconfirmed.

```rust
let candidate = k.propose_contribution(&bob, "demo", contribution_input).await.unwrap();
assert_eq!(k.confirm_contribution(&bob, "demo", candidate.id, candidate.revision).await, Err(Error::Denied));
let confirmed = k.confirm_contribution(&alice, "demo", candidate.id, candidate.revision).await.unwrap();
k.review_contribution(&bob, "demo", confirmed.id, confirmed.revision).await.unwrap();
assert_eq!(k.review_contribution(&reader, "demo", candidate.id, candidate.revision).await, Err(Error::Denied));
```

- [ ] Run `scripts/cargo test -p wk-knowledge --test code_anchors -- --ignored`; verify red for the missing behavior.
- [ ] Implement `code_contributions`, `code_rationale_anchors`, `code_maintainers` with explicit scoped joins, source/card/mapping revisions, confirmation and review state. No heuristic backfill. Confirmation/review check the exact current revision under locks; only confirmed/reviewed anchors referencing a published, nonwithdrawn card are eligible. Identity mapping verification never verifies a new contribution automatically. Query current source state on every read so correction, supersession, withdrawal or mapping/profile removal immediately changes visibility/freshness without a background worker.

```sql
-- Eligibility also joins repo/mapping/profile and their current scope/revisions.
c.published AND NOT c.withdrawn AND c.revision = a.card_revision
AND a.confirmed_by = c.author AND a.reviewed_by IS NOT NULL
AND a.revoked = false
```

- [ ] Test multiple contributors, unknown mapping, removed contact/profile, maintained-by assignment independent of author, candidate historical anchors, and correction/republication requiring renewed statement confirmation. Add operator CLI strict-input tests following `admin_cli.rs` fixtures; never import a raw Git author list.
- [ ] Run targeted suites and commit `feat: bind confirmed rationale to code revisions` with `-s`.

### Task 3: Bounded local repository observation

**Files:** Create `crates/agent/src/repository.rs`; register the module in `main.rs`; add optional repository enrollment to `state.rs`. Keep existing config readable through serde defaults. Module tests use `tempfile` and synthetic Git commits, not a developer's real history.

**Inputs/outputs:** `RepositoryEnrollment { repository_id: Uuid, root: PathBuf, git_dirs: Vec<PathBuf>, binding_revision: i64 }`; `LocalObservation { request: ChangeContextRequest, target_digests: BTreeMap<String,String>, operation_digest: String, gaps: Vec<String> }`; `observe(&Config, &RepositoryEnrollment, &[CodeTarget], operation_class: &str, tool_input: &serde_json::Value) -> Result<LocalObservation,String>` async; `same_snapshot(&LocalObservation, &LocalObservation) -> bool`. The wire request excludes digests/tool input. Root and Git dirs are independently approved and local only.

- [ ] Write module tests for exact HEAD, staged/unstaged target changes, new/untracked/binary targets, shallow history, symlink escape, a separately unapproved worktree Git dir, invalid object IDs, replacement objects, path starting with `-`, and platform-specific case collisions. Assert serialized request has no code/prompt/email/absolute path/digest.

```rust
let original = observe(&config, &enrollment, &targets, "edit", &edit).await.unwrap();
std::fs::write(config.root.join("api/retry.rs"), "changed locally\n").unwrap();
let changed = observe(&config, &enrollment, &targets, "edit", &edit).await.unwrap();
assert!(!same_snapshot(&original, &changed));
let wire = serde_json::to_string(&changed.request).unwrap();
assert!(!wire.contains("changed locally"));
assert!(!wire.contains(&changed.operation_digest));
```

- [ ] Run `scripts/cargo test -p wk-agent repository::tests`; verify missing observer behavior.
- [ ] Implement a fixed-argument child runner with a five-second outer deadline, a bounded stdout reader and child termination on timeout. Clear Git execution-control environment; set `GIT_CONFIG_NOSYSTEM=1`, isolated global config, `GIT_NO_REPLACE_OBJECTS=1`, `GIT_TERMINAL_PROMPT=0`, and disable credential helpers/external diff/textconv. Commands are fixed read-only `rev-parse`, `ls-files`, `status`, and bounded `blame --line-porcelain` with `--` before a validated path. Never fetch, run hooks, load commit messages or expose child stderr. Check approved metadata roots before commands, and canonicalize each path/component before reads. Do not follow unapproved submodules. Ambiguity becomes an explicit local gap or denial.

```rust
let mut cmd = tokio::process::Command::new(git_executable);
cmd.current_dir(&enrollment.root).kill_on_drop(true)
    .env_remove("GIT_DIR").env_remove("GIT_WORK_TREE")
    .env_remove("GIT_INDEX_FILE").env_remove("GIT_OBJECT_DIRECTORY")
    .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES").env_remove("GIT_CONFIG_COUNT")
    .env("GIT_NO_REPLACE_OBJECTS", "1").env("GIT_TERMINAL_PROMPT", "0")
    .args(["-c", "credential.helper=", "-c", "core.fsmonitor=false", "rev-parse", "--verify", "HEAD^{commit}"]);
```

- [ ] Parse bounded provenance into local candidate mapping references only. Unmapped author fields never leave the device. Hash local operation JSON and target contents with SHA-256 in private local state; dirty overlaps require review, never an unchanged-revision claim.
- [ ] Run unit tests on available OS, rely on existing OS CI for conditional Windows cases, and commit `feat: observe approved repository snapshots locally` with `-s`.

### Task 4: Deterministic authorized change-context assembly

**Files:** Create `crates/knowledge/src/change_context.rs`, `crates/knowledge/tests/change_context.rs`; register module; complete deterministic rendering/validation in contracts module. Reuse graph/evidence storage, not the unrestricted complete graph API.

**Inputs/outputs:** `change_context` returns the shared brief contract. Internal `select_context(tx, actor, product, request)` returns a bounded source set in the same request transaction. Render function `render_change_context(&ChangeContextBrief) -> String` in contracts produces a reference-only block that cannot omit safety reasons to make room for narrative.

- [ ] Write two-identity DB tests with a published Alice rationale, confirmed contribution, Bob lookup, exact repository/revision/path, separate current maintainer, reviewed dependency and stale test evidence. Assert source-backed name/reason and incomplete coverage. Add same-path/different-repository, own published contribution, hidden edge endpoint, superseded/corrected card, profile/mapping revocation and legacy heuristic-only cases.

```rust
let brief = k.change_context(&bob, "demo", request.clone()).await.unwrap();
assert!(matches!(brief.decision, Decision::NeedsReview));
assert_eq!(brief.coverage, "reviewed records only");
assert!(brief.items.iter().any(|i| i.role == "contributor" && i.display_name.as_deref() == Some("Alice")));
assert!(brief.items.iter().any(|i| i.role == "rationale_author" && !i.sources.is_empty()));
assert!(brief.gaps.iter().any(|g| g == "dependency_coverage_incomplete"));
assert!(!serde_json::to_string(&brief).unwrap().contains("PRIVATE_CANARY"));
```

- [ ] Run `scripts/cargo test -p wk-knowledge --test change_context -- --ignored` and contract renderer tests; expect red.
- [ ] Query exact authorized anchors first; bounded same-repository historical candidates second, explicitly marked stale. Join contributions separately from submitters/maintainers. One-hop reviewed dependency/conflict records must have both endpoints and their card/repository scopes visible, and source revisions current; never bridge a hidden node. Tests must match immutable code revision to appear current. Activity uses last-reported time only; existing unbound activity paths are advisory, never factual repository provenance. V1 never promises complete dependency coverage.

```rust
brief.coverage = "reviewed records only".into();
brief.gaps.push("dependency_coverage_incomplete".into());
brief.decision = Decision::NeedsReview;
if !has_confirmed_reason { brief.reasons.push("reason_not_recorded".into()); }
if source_conflict { brief.reasons.push("conflicting_recorded_constraints".into()); }
```

- [ ] Enforce 20-source/target bounds and byte budget with reason/gap fields reserved first; aggregate explicit `truncated` flag and gap. Test giant approved rationale, repeated references and malicious display text. Database outages return unavailable errors, not empty successful briefs. Narrowing membership or any revision invalidates current joins.
- [ ] Run targeted restricted-role tests and commit `feat: assemble source-backed code change context` with `-s`.

### Task 5: Revision-bound preflight persistence and single-use acknowledgement

**Files:** Create `migrations/server/0016.sql`, `crates/knowledge/src/preflight.rs`, `crates/knowledge/tests/preflight.rs`; update migration/readiness/grants to 16. Extend retention counts/cleanup and restore invalidation in Task 9 before any deployed use.

**Inputs/outputs:** The preflight methods listed above. `ProtectedIdentity` is derived from signed authenticated device/binding state. Brief IDs and operation handles are separate random UUIDs. Server never needs the edit patch or its hash; the local adapter owns that binding.

- [ ] Write DB tests for foreign owner/device, stale HEAD/operation handle, changed card/profile/mapping/repository/maintainer/edge revisions, revoked membership/device, expired acknowledgement, duplicate consumption, concurrent attempts and unavailable storage. A read-context request cannot produce an enforceable receipt.

```rust
let brief = k.create_preflight(&bob, "demo", protected.clone(), request).await.unwrap();
let id = brief.brief_id.unwrap();
assert_eq!(k.consume_preflight(&bob,"demo",protected.clone(),id,operation).await, Err(Error::Denied));
k.acknowledge_preflight(&bob,"demo",id).await.unwrap();
k.consume_preflight(&bob,"demo",protected.clone(),id,operation).await.unwrap();
assert!(k.consume_preflight(&bob,"demo",protected,id,operation).await.is_err());
```

- [ ] Run `scripts/cargo test -p wk-knowledge --test preflight -- --ignored`; verify red.
- [ ] Persist `code_preflights` with org/product/owner, device/binding/epoch, repository and policy revisions, full source-version set, operation handle, bounded brief JSON, created/deadline/acknowledged/consumed state. Use a fixed five-minute authorization deadline and separate 24-hour physical cleanup deadline. Recompute the authorized source-version set before acknowledgement and consumption; compare exactly. Include dependent graph/source state even when an edge disappears, so removal cannot preserve a prior receipt accidentally. Lock membership/device/policy/source state using existing lock order, then lock preflight row; any lock-order conflicts must be resolved before green tests.

```sql
UPDATE code_preflights SET consumed_at=clock_timestamp()
WHERE id=$1 AND org=$2 AND product=$3 AND owner=$4
 AND device_id=$5 AND operation_handle=$6 AND consumed_at IS NULL
 AND acknowledged_at IS NOT NULL AND expires_at>clock_timestamp()
RETURNING id;
```

- [ ] Perform source/policy revalidation and update in the same transaction; zero rows means denial/conflict, never allow. RLS limits reads and mutations to the owner's current all-service scope. Shared callers cannot list another developer's pending actions. Minimal audit events omit names, code, source text and raw input.
- [ ] Run replay/concurrency/revocation regressions and commit `feat: require revision-bound preflight acknowledgement` with `-s`.

### Task 6: REST, collector, CLI and MCP transport integration

**Files:** Create `crates/server/src/change_routes.rs`, `crates/server/tests/change_context.rs`; modify server router, `device_routes.rs`, `remote_mcp.rs`, `crates/mcp/src/main.rs`, `crates/agent/src/main.rs` and network/device wrappers; extend MCP tests in their existing module/test patterns.

**Inputs/outputs:** Ordinary REST `POST /v1/products/{product}/change-context` gives an advisory brief. Collector signed operations `change-context` and `preflight-consume` give/create a bound brief and consume acknowledged permission, using the existing collector authentication/audience/nonces. Browser-only `GET .../preflights/{id}` and `POST .../preflights/{id}/acknowledge` require the owner browser session; acknowledgement never accepts bearer/collector/MCP credentials. Code profile and candidate/confirmation/review routes delegate to Task 1/2 methods, with confirmation restricted to browser identity, and review restricted to authorized reviewers. Add bounded candidate listing only within the author/reviewer visibility rule, not a general member directory.

- [ ] Write HTTP tests for auth-less/cross-product requests, cookie/bearer ambiguity, collector-audience isolation, invalid body over 64 KiB, strict schema, response over 16 KiB, unauthorized brief enumeration, missing CSRF and bearer acknowledgement. Run `scripts/cargo test -p wk-server --test change_context -- --ignored` for red.

```rust
let response = app.clone().oneshot(Request::builder()
    .method("POST").uri(format!("/v1/products/demo/preflights/{id}/acknowledge"))
    .header("authorization", "Bearer synthetic-bob")
    .header("content-type", "application/json").body(Body::from("{}"))
    .unwrap()).await.unwrap();
assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
```

- [ ] Implement thin handlers using current Actor resolution, browser-session/Origin/CSRF checks and `no-store`. Dedicated routes have a 64 KiB body limit and five-second timeout; scrub failure bodies. Signed collector operations construct `ProtectedIdentity` from the authenticated registration and current binding. They do not grant general bearer MCP access or expose human confirmation operations.

```rust
Router::new().route("/v1/products/{product}/change-context", post(context))
    .route("/v1/products/{product}/preflights/{id}", get(brief))
    .route("/v1/products/{product}/preflights/{id}/acknowledge", post(acknowledge))
    .layer(DefaultBodyLimit::max(64 * 1024));
```

- [ ] Add `knowledge.change_context` to stdio and remote MCP, using the same request schema and deterministic source-backed response. Explicitly exclude acknowledgement/consume from MCP. Existing `knowledge.context`/`knowledge.overlaps` tests must still pass. Provide `wk-agent --state-dir <private-dir> context --files <relative-paths>` using Task 3 and signed collector; it reports advisory coverage, not permission to edit. Enrollment commands bind repository ID and approved metadata dirs without inferred remote identity.
- [ ] Test local-observation serialization against code/prompt/email canaries and auth failure without credential logging; run targeted server/MCP/agent suites; commit `feat: expose scoped code context across clients` with `-s`.

### Task 7: Hosted attribution, rationale review and change brief

**Files:** Create `web/src/CodeAttribution.tsx`, `web/src/CodeContext.tsx`, `web/tests/code-context.spec.ts`, `web/tests/code-attribution.spec.ts`; modify `App.tsx`, `types.ts` and API helpers only where needed; extend existing portal identity harness with a new `scripts/portal-code-context-smoke.py`.

**Inputs/outputs:** `CodeContext({product, briefId})` displays the owner brief and browser-only acknowledgement. `CodeAttribution({product, policy})` edits one's opt-in profile and confirms one's contribution/intent candidates; reviewers see only authorized candidates. Existing application navigation/deep link accepts an opaque brief ID, never source contents or tokens in URL. Use existing layout/styles; no dashboard redesign.

- [ ] Write Playwright tests for distinct contributor/rationale/maintainer roles, missing names/reasons, historical source warnings, expired/revised briefs, oversized text, XSS-like strings, contact URL rejection, no cross-user pending list, reviewer visibility and one-time confirmation. Add logout/product-switch tests with delayed responses; a resolved old request must not enable a new acknowledgement button.

```typescript
await expect(page.getByRole('heading', {name:'Before changing this code'})).toBeVisible();
await expect(page.getByText('Dependency coverage is incomplete.')).toBeVisible();
await expect(page.getByRole('button', {name:'Acknowledge this change'})).toBeEnabled();
await page.getByRole('button', {name:'Acknowledge this change'}).click();
await expect(page.getByText('Acknowledged for this operation only.')).toBeVisible();
await expect(page.getByRole('button', {name:'Acknowledge this change'})).toBeDisabled();
```

- [ ] Run `npm test -- code-context.spec.ts code-attribution.spec.ts` in `web` for red; do not pass tests solely through a static mock snapshot.
- [ ] Implement API-backed forms with required explicit intent/contribution confirmation, current revision checks and abortable requests. Separate role/source rows, quoted confirmed rationale, unknown gaps, relevant tests and last-reported activity. Contact links are approved HTTPS and no send action exists. Acknowledgement has a user checkbox and describes the exact targets/operation plus gaps; it cannot signal file safety or alter membership. Never expose tokens in browser storage. Input strings render as escaped text.

```typescript
await request('', `${productPath(product)}/preflights/${encodeURIComponent(briefId)}/acknowledge`, {});
// request() uses existing same-origin cookie/CSRF handling, never an MCP token.
```

- [ ] Run build and targeted browser tests, then the disposable live OIDC/server harness as Alice and Bob. Inspect actual desktop and phone-width UI for readable references, keyboard focus and no horizontal overflow. Do not simulate a developer's acknowledgement on a real working edit; harness confirmations are synthetic.
- [ ] Commit `feat: show contributor rationale and change review in portal` with `-s`.

### Task 8: Separate protected pre-edit adapter and local operation binding

**Files:** Create `crates/adapters/src/change_guard.rs`, `crates/agent/src/protection.rs`; modify adapters exports, hook installation/config ownership and agent CLI dispatch. Add module tests alongside these files; existing observational hooks stay unchanged.

**Inputs/outputs:** `parse_edit(client: &str, version: &str, payload: &Value) -> Result<GuardEdit,GuardError>`; `GuardEdit { targets: Vec<CodeTarget>, operation_class: String, tool_input: Value }` local-only; `GuardError::{UnsupportedVersion,UnsupportedTool,InvalidInput,OutOfScope}`. `guard_output(client: &str, decision: GuardDecision) -> Value`, with `GuardDecision::{Stop{reason:String,brief_url:String},Proceed}`. `protection::before_edit(state: &Path, client: &str, input: Value) -> Result<Value,String>` is a separate CLI/hook path that never uses capture's success fallback. `consume_signed_preflight(config: &Config, brief_id: Uuid, operation_handle: Uuid) -> Result<(),String>` async wraps the authenticated collector operation. `PendingOperation { observation: LocalObservation, brief_id: Uuid, operation_handle: Uuid, expires_at_ms: i64 }` remains private local state. Config records approved client/version/OS/tool shapes and enrolled repository binding; status distinguishes installed/certified/advisory/uncovered.

- [ ] Test supported Edit/Write/Delete/MultiEdit input shapes, nested/outside targets, too many targets, NotebookEdit ambiguity, changed retry payload, local file change while awaiting acknowledgement, missing enrollment, server 401/403/timeout, and failure before/after consuming a receipt. Retry cannot inherit approval merely because the prior brief was shown.

```rust
let stopped = guard_output("claude-code", GuardDecision::Stop {
    reason: "Developer confirmation required".into(), brief_url: approved_brief_url,
});
assert_eq!(stopped["hookSpecificOutput"]["permissionDecision"], "deny");
assert!(stopped["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap()
    .contains("Developer confirmation required"));
```

- [ ] Run `scripts/cargo test -p wk-adapters change_guard` and `scripts/cargo test -p wk-agent protection::tests` for red.
- [ ] Implement explicit `wk-agent ... protect enroll/status/disable` and `protect-hook` commands. Merge a separately owned pre-tool configuration fragment and preserve unrelated/capture hooks. On first covered operation observe targets, store private pending input fingerprint/digests, request a signed-device preflight and return a denial with a trusted same-origin brief URL. After human browser acknowledgement, an identical retry reobserves and compares local state, consumes permission under current server authorization, rechecks local state, and only then returns no app veto so normal host permissions still apply. A mismatch restarts preflight. Failed consume/recheck never grants permission; consumed permission is not recoverable.

```rust
let stop = || guard_output("claude-code", GuardDecision::Stop {
    reason: "Snapshot changed; a fresh brief and confirmation are required".into(),
    brief_url: approved_brief_url.clone(),
});
if !same_snapshot(&pending.observation, &current) { return Ok(stop()); }
consume_signed_preflight(&config, pending.brief_id, pending.operation_handle).await?;
let immediately_before_edit = repository::observe(&config, &enrollment,
    &edit.targets, &edit.operation_class, &edit.tool_input).await?;
if !same_snapshot(&current, &immediately_before_edit) {
    return Ok(stop());
}
// No permission escalation: leave the host's ordinary permission checks in place.
```

- [ ] Make failure exit/output a blocking result on the actually tested host; unsupported client/tool shapes are denied within enrolled coverage or explicitly uncovered outside it, never silently called protected. Verify timeout/crash behavior experimentally. If a host ignores denial or fail-opens on process failure, ship advisory retrieval for that surface and record the failure; do not fake protection through prompt instructions. Show exact tools and bypasses including shell/manual edits. No automatic user confirmation.
- [ ] Add post-edit target deviation detection to this separate adapter using local before/after state, marking proposals/reported tests honestly. Confirm capture errors remain nonblocking in existing regression suites. Commit `feat: add explicit before-edit code context gate` with `-s`.

### Task 9: Retention, restore, live acceptance and operations documentation

**Files:** Modify `crates/knowledge/src/retention.rs`, retention DB tests, `deploy/scripts/restore.sh`, `scripts/hosted-restore-drill.py`, CI registration and README coverage; create `scripts/code-context-live.py`, `scripts/code-context-pilot.py`, `docs/reviews/2026-10-11-code-change-context-acceptance.md` and `docs/architecture/code-change-context.md`.

**Inputs/outputs:** Live harness consumes existing synthetic-provider/server patterns and two distinct app identities, an owned temp repository/clone, a device binding and a supported client executable. Output sanitized evidence lists versions, paths relative to synthetic repos, check results, latency and storage growth. Never report an unrun check as passed.

- [ ] Write retention/restore failures with an expired preflight, acknowledged operation in a backup and later withdrawal/revocation. Restored receipt must be unusable; logical expiry must deny before cleanup; restricted runtime cannot run retention cleanup.

```python
assert consume_restored_receipt_status in (401, 403, 404, 409)
assert unsafe_file_before_confirmation == unsafe_file_after_blocked_attempt
assert confirmed_contributor_name == 'Alice Synthetic'
assert 'PRIVATE_CANARY' not in shared_response_json
```

- [ ] Run targeted retention/restore tests for red. Add preflight cleanup to the existing bounded operator worker (max 1,000 per batch), delete short-lived bodies after the 24-hour cleanup deadline, and expire local digests with receipts. Extend restore quarantine to delete/invalidate every preflight acknowledgement in the same transaction as existing membership/session/device revocation. Handle schema 16 in compatibility/grant/readiness checks.
- [ ] Build binaries and run the disposable end-to-end harness: Alice commits and confirms a retry invariant; Bob's separate clone pulls it; a reviewed consumer depends on it; Bob sees Alice's name/reason and relevant references before a risky edit; the actual file remains unchanged until human confirmation in the synthetic UI, then the exact operation proceeds. Repeat with changed HEAD, forged Git identity, missing rationale, hidden service, withdrawn source, stale tests, network failure, different operation, client timeout/crash and receipt replay. Verify unsupported surfaces remain visibly advisory.
- [ ] Run a five-developer bounded pilot at cold/warm cache, record p50/p95, deadlines, DB relation sizes/row growth, and source limits using the new path. Do not extrapolate portal-only throughput or claim a particular deployed server capacity.
- [ ] Complete the normal final verification once changes stabilize:

```sh
scripts/cargo fmt --all -- --check
scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/cargo test --workspace --locked --no-fail-fast
scripts/cargo test --workspace --locked -- --include-ignored
python3 -m unittest discover -s scripts/tests
```

In `web`, run `npm run build` and `npm test`; run new live/pilot/restore harnesses with owned synthetic resources and validated private test configuration. Missing DB/client access is reported as unverified, not skipped success.

- [ ] Publish acceptance documentation with exact client/version/OS/tool coverage and results, human-interaction trust boundary, known race/bypass limitations, dependency incompleteness and unchanged native-capture/security release gates. Update README with the working setup/workflow and advisory-versus-certified labels. Commit `test: verify two-developer code change context flow` with `-s`; preserve test evidence outside raw-conversation artifacts. No release/deployment or automatic teammate contact.

## Plan self-review and execution boundary

Coverage: intent/names/contact → Tasks 1–2, 7; approved snapshot/provenance → Task 3; impact/freshness/scope → Task 4; confirmation/replay/revocation → Tasks 5–8; retention/performance/real client evidence → Task 9. All five Review Focus cases have explicit regression ownership. Production provider connectors, semantic framework analyzers, additional protected clients and hostile-local-agent isolation remain the approved spec's separate work, not implied completion claims.

Review this plan before execution. Native execution in this chat is recommended because these tasks share contracts and authorization invariants; it avoids dispatching implementation into multiple concurrent owners. Follow `superpowers:executing-plans` after plan approval, completing task checks and evidence before moving on. A fresh whole-branch reviewer, if required by that execution skill, follows implementation; no implementation delegation is needed for planning.
