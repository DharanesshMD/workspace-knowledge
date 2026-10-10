# Owner Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a developer browse, edit and deliberately submit their own sanitized summaries in the hosted portal, with optional local collector synchronization disabled by default.

**Architecture:** Separate `personal_summaries` from shared cards. All reads/writes require the owner and current membership in every included service, including SQL RLS. Local synchronization reuses existing source validation but sends only complete screened CardSubmission bodies to a distinct owner endpoint. Explicit review submission creates a shared unpublished card atomically; no private record reaches MCP/search/graph/activity.

**Tech Stack:** Rust/Axum/SQLx/PostgreSQL, existing local collector and constrained summary contracts, React/TypeScript/Playwright.

**Spec:** `docs/architecture/2026-10-10-hosted-developer-portal-plan.md`, increment 2 and sections 4, 5, 7, 9. User approved the roadmap; execute inline, preserving all existing capture gates and worktree changes.

## Global Constraints

- No raw transcripts, local absolute paths, vault values or provider tokens in the owner endpoint.
- Owner-only synchronization defaults off. Enabling it is an explicit local action bound to the approved workspace policy hash.
- Native held/incomplete/mixed/unsupported sources remain local; do not weaken capture attribution or publication gates.
- Owner records live for 30 days from creation; edits/retries do not extend expiration. At most 2,000 active summaries per owner/product, 64 KiB body each.
- Current organization/product/every-service membership on every operation; review/admin roles confer no personal access.
- Deletion clears hosted text and retains a content-free idempotency/deletion marker; old retries never recreate it.
- Manual notes are labeled manual, proposed, and have no fabricated test/code evidence.
- Submission names the reviewer audience and uses current owner policy/revision; publication remains a separate reviewer action.
- Preserve quiet workbench design, same-origin cookie/CSRF, keyboard navigation and 390px layout.

## Review Focus

- Reviewer guessing an owner ID: return not found even with all service permissions (Tasks 1/2).
- A retry after local restart or owner deletion: same record on identical body, conflict on changed/deleted body; no TTL extension (Tasks 1/3).
- Product or service revocation after creation: list/detail/edit/submission unavailable and no count/name leakage (Tasks 1/2).
- Private edit and review submission racing: hold owner row lock, require expected revision, commit shared card atomically (Task 2).
- Partial/held source at sync time, or failed local model screening: do not upload and return only safe counts/reason codes (Task 3).

---

### Task 1: Separate private summary repository and RLS

**Files:** Create `migrations/server/0007.sql`, `crates/knowledge/src/personal.rs`, `crates/knowledge/tests/personal.rs`; modify knowledge `lib.rs` and `deploy/scripts/application-role.sql`.

**Interfaces:** `PersonalSummary { id:Uuid, body:CardSubmission, revision:i64, provenance:String, created_at_ms:i64, expires_at_ms:i64, submitted_card:Option<Uuid> }`; `PersonalInput { body:CardSubmission, provenance:PersonalProvenance }` with strict enum `manual|captured`. Methods `personal_put(actor,product,input)`, `personal_get(actor,product,id)`, `personal_list(actor,product,before:Option<Uuid>)`, `personal_edit(actor,product,id,expected:i64,body:CardSubmission)`, `personal_delete(actor,product,id,expected:i64)` returning the appropriate record or unit. List returns at most 50 records ordered by UUID descending; cursor remains opaque UUID and no total count.

- [ ] Write disposable DB regressions establishing alice, reviewer bob and cross-org alice. Capture a private summary, check owner list/get, and assert all other identities get NotFound/empty. Search/review/graph remain empty, even for the owner. Assert idempotent put preserves ID/expiry; differing body conflicts; stale edit conflicts without losing text; delete clears body and retry conflicts; service revocation hides the record; expiration denies detail. Test non-owner runtime role with the grant script and owner RLS.

```rust
let note=k.personal_put(&alice,"demo",input("one", "captured")).await.unwrap();
assert_eq!(k.personal_get(&bob,"demo",note.id).await.unwrap_err(),Error::NotFound);
assert!(k.personal_list(&bob,"demo",None).await.unwrap().is_empty());
assert!(k.search(&alice,"demo","limiter",10).await.unwrap().is_empty());
k.personal_delete(&alice,"demo",note.id,note.revision).await.unwrap();
assert_eq!(k.personal_put(&alice,"demo",input("one","captured")).await.unwrap_err(),Error::Conflict);
```

- [ ] Run `scripts/cargo test -p wk-knowledge --test personal -- --ignored` against disposable wk_test; expected missing repository methods.
- [ ] Register migration7. Table stores UUID/org/product/owner, bounded idempotency key, services, nullable body, provenance, revision, submitted_card, created_at, fixed expires_at and deleted flag; unique owner/product/key. Check body size and provenance; index owner/product/id for keyset list and expiration. RLS enforces explicit current owner and all service membership, never `can_review`. Content-free deletion markers cannot be read as notes. Implement transaction-scoped membership locks, strict contract validation and privacy screening, complete captured summaries only, manual notes proposed without evidence, exact retries, 2,000 active cap under per-owner advisory lock, revision-checked edits, body-clearing delete and no expiry reset. Do not place personal summaries in shared tables.
- [ ] Run DB regressions and existing knowledge tests; expected PASS. Commit repository/migration/grants.

### Task 2: Owner HTTP routes and atomic review submission

**Files:** Create `crates/server/src/personal_routes.rs`, `crates/server/tests/personal.rs`; modify server router, `crates/knowledge/src/personal.rs` and extract the existing card insertion transaction helper in knowledge `lib.rs`.

**Interfaces:** GET/POST `/v1/products/{product}/personal`; GET/PATCH/DELETE `/v1/products/{product}/personal/{id}`; POST `/v1/products/{product}/personal/{id}/submit`. Write bodies deny unknown fields: put `{body,provenance}`, edit `{expected_revision,body}`, delete/submit `{expected_revision}`. Responses `{summary}` or `{summaries,next_cursor}`; no transcript fields. `personal_submit(actor,product,id,expected)->Card` holds the owner row and membership locks and calls reusable `submit_in_transaction` before updating its submitted_card link.

- [ ] Write HTTP tests for missing identity, unknown transcript fields, wrong-owner detail, guessed ID and CSRF mutation denial, safe validation errors, stale edit409, private-to-review transition and denied private MCP retrieval. Use real disposable PostgreSQL with development bearer identities for owner/reviewer and existing same-origin cookie protection tests.

```rust
assert_eq!(call(&app,"GET",owner_path,None,"reviewer").await.status(),StatusCode::NOT_FOUND);
assert_eq!(call(&app,"POST",owner_submit,Some(json!({"expected_revision":1})),"owner").await.status(),StatusCode::OK);
assert_eq!(k.review_queue(&reviewer,"demo").await.unwrap().len(),1);
assert!(k.search(&reviewer,"demo","limiter",10).await.unwrap().is_empty());
```

- [ ] Run the HTTP test; expected missing route404. Extract existing submit transaction internals unchanged into a helper taking `&mut Transaction`, actor/product/body. Both public submit and personal submit validate the strict card and current policy. Private submit requires expected revision, generates `personal:{id}:{revision}` idempotency key, uses the current policy revision and atomically creates the unpublished shared card. An edited personal note clears its link; old shared card remains a separate reviewed workflow. Repeat submit of the same revision returns the same card, and never publishes. Add strict routes and bounded JSON parsing through existing helpers; errors never echo input.
- [ ] Run personal HTTP and all existing server/knowledge tests; expected PASS. Commit routes/atomic transition.

### Task 3: Explicit local opt-in and eligible summary sync

**Files:** Create `crates/agent/src/personal_sync.rs`; modify agent `main.rs` and `process.rs`; add tests to sync module and existing process tests; document local commands.

**Interfaces:** `wk-agent --state-dir PATH owner-sync status|enable|disable [--yes]`; `personal_sync::status(dir)->Value`, `configure(dir,action,yes)->Value`, `run(dir)->Result<Value,String>`. Local choice is stored through `state::save` as `{enabled,policy_hash}`. No token or path is included in remote summary body. Extend `process::submit` with a private shared `deliver(dir,id,owner_only)`; owner mode sends strict personal input and never acknowledges/deletes the local draft. Existing reviewed submit uses shared cards exactly as before.

- [ ] Test absent choice returns disabled without loading credentials; enable without --yes denied; changed binding hash disables sync; partial CardSubmission denied before delivery. Existing native/managed validators remain unchanged and are tested against held/digest/policy/expiry changes.

```rust
assert_eq!(status(temp.path()).unwrap()["enabled"],false);
assert!(!eligible(&json!({"card":{"capture_completeness":"partial"}})));
```

- [ ] Run targeted agent tests; expected missing module/functions. Implement local opt-in only after binding/current policy validation. After processing, sync at most 50 live drafts while opted-in, skip acknowledged sync markers, reuse current native/managed source checks and final screening, and send only `{body:card,provenance:"captured"}` to the owner endpoint. Native validation guards remain held through upload. Mark success locally with original expiry bounded by the draft; failed uploads remain retryable and never extend raw TTL. Return uploaded/held counts; no source text or credential errors. Changing consent/policy invalidates opt-in. Disable stops future synchronization; it does not silently delete already hosted notes.
- [ ] Run agent suite and synthetic loopback transport tests for success/retry/held exclusion/default-off; expected PASS. Commit local sync.

### Task 4: My context UI

**Files:** Create `web/src/MyContext.tsx`, `web/tests/personal.spec.ts`; modify App/Session/types/style. No unrelated workbench redesign.

**Interfaces:** Pass selected product policy/services to hosted workbench. Add My context as default hosted view; retain existing development view. MyContext consumes personal routes and policy, holds only current page/detail/form state, aborts on product switch/unmount, and uses existing request cookie/CSRF helper.

- [ ] Write Playwright checks for owner-only label and no conversation viewer, empty state/manual note, allowed service options, partial captured state shown safely, revision409 retaining text, explicit reviewer-audience explanation before submit, delete confirmation clearing detail, product switch clearing old notes and 390px no overflow.

```ts
await expect(page.getByRole('heading',{name:'My context',exact:true})).toBeVisible();
await expect(page.getByText('Only you can read these summaries.')).toBeVisible();
await expect(page.getByRole('button',{name:'Submit for review'})).toBeVisible();
```

- [ ] Run tests; expected missing view. Implement paginated owner list/detail with manual/captured provenance, creation/expiry and submission link, service filter, safe text summaries and structured title/intent/rationale/decisions/constraints/open questions editor. Manual notes use crypto.randomUUID key, current policy, proposed lifecycle, complete manually authored content and no evidence. Explain review visibility explicitly on submit; keep published lifecycle separate. Edit uses expected revision and retains text on409. Deletion asks an in-page confirmation naming this summary, then clears it on success. Show local sync disabled/unknown guidance until a reporting device exists; do not invent remote collector state.
- [ ] Run build/all browser tests and inspect desktop/mobile screenshots; expected PASS. Commit UI.

### Task 5: End-to-end privacy acceptance and review

**Files:** Add owner context scenarios to a separate synthetic integration script, `docs/reviews/2026-10-10-owner-context-acceptance.md`; update console/backend/completion docs.

- [ ] Run owner/reviewer/restricted/cross-org identities through private creation/read/edit/delete/retry/expiry/revoke/submit; assert private text is absent from shared search/review/graph/MCP before explicit submission. Use a synthetic secret canary rejected by screening and verify generic responses contain no canary. Verify local default-off and held source sync without touching real agent settings. Record current capabilities and blocked native version coverage honestly.
- [ ] Run formatting, relevant clippy/tests/DB regressions, web build/browser suite, whitespace checks and inspect screenshots. Document deletion markers/30-day logical expiry and physical cleanup/restore integration still belonging to admin/operations increment; no rollout claim until that gate.
- [ ] Perform one independent final review, fix Important/Critical issues in one RED→GREEN pass, commit. Continue Team context and device/admin increments from their own approved roadmap plans.
