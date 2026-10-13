# Team Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Browse authorized published contributions and explicitly shared, unexpired activity in the hosted portal, without revealing private context or an employee directory.

**Architecture:** Read existing shared cards and activity through current membership and RLS. Add a bounded chronological published feed and safe activity list; use existing card detail/evidence and overlap APIs. Author filters identify only visible published contributors; no new profile table or inferred productivity metric.

**Tech Stack:** Rust/Axum/SQLx/PostgreSQL, React/TypeScript/Playwright.

**Spec:** `docs/architecture/2026-10-10-hosted-developer-portal-plan.md` increment 3 and Team context/visibility sections. User authorized the roadmap and inline execution; this plan refines that approved scope.

## Global Constraints

- Team feed includes published, non-withdrawn cards only, even for reviewers; no personal table joins.
- Every query requires current org/product membership and all card/activity services. No unauthorized author names or counts.
- Published historical/deprecated decisions may appear with explicit lifecycle, unlike active-only lexical search.
- Pages contain at most 50 cards, with chronological `(created_at,id)` ordering and opaque UUID anchor. Filters: author, permitted service, created-since Unix milliseconds, exact safe relative file reference.
- Activity is existing opt-in reporting, not inferred work: show last reported and expiry, exclude ended/expired records, no session IDs or raw configuration.
- Preserve same-origin cookie/CSRF and existing bearer/MCP interfaces. No new dependency or external analytics.
- Product/view switches abort outstanding requests and clear former details. Render text safely; 390px layout remains usable.

## Review Focus

- Reviewer with broad privileges cannot expose unpublished/private cards through the feed (Tasks 1/2).
- Multi-service card/activity is hidden from a member who lacks any service, including names/counts/filter choices (Tasks 1/2).
- Withdrawal or correction between feed and detail does not keep stale text visible after refresh or a denied detail (Task 2).
- Expired or ended activity is not shown as current work; even live heartbeat is labeled last reported (Tasks 1/2).
- Product switch during slow feed/detail prevents old response from repopulating the new scope (Task 2).

---

### Task 1: Published feed and scoped activity routes

**Files:** Create `crates/knowledge/src/team.rs`, `crates/knowledge/tests/team.rs`, `crates/server/src/team_routes.rs`, `crates/server/tests/team.rs`; modify lib module/router registration. Reuse existing schema; no migration required initially.

**Interfaces:** `TeamQuery { before:Option<Uuid>, author:Option<String>, service:Option<String>, since_ms:Option<i64>, file:Option<String> }`, strict Deserialize/Default. `TeamEntry { card:Card, author:String, created_at_ms:i64 }`. `SharedActivity { developer:String, intent:String, files:Vec<String>, services:Vec<String>, reported_at_ms:i64, expires_at_ms:i64 }`. `Knowledge::team_context(&Actor,&str,TeamQuery)->Result<Vec<TeamEntry>,Error>` and `team_activity(&Actor,&str)->Result<Vec<SharedActivity>,Error>`. GET `/v1/products/{product}/context` accepts the query and returns `{entries,next_cursor}`; GET `.../activity` returns `{activity}` alongside existing POST/DELETE. Unknown/malformed query fields return generic422. Cap strings256, validate service IDs and file using activity::valid_path plus no drive-colon; since_ms nonnegative and at most now+5min.

- [ ] Write real disposable DB tests using owner alice, reviewer bob, backend-only reader, and cross-org identities. Submit unpublished and personal notes, publish one backend and one backend/frontend card. Assert reviewer feed contains only published permitted cards; narrow reader sees neither mixed-scope card nor its author via filters. Assert arbitrary hidden UUID anchor yields empty page. Test 51 same-time cards yield 50+1 without duplicates, author/service/date/file filters, current revision after correction/republish, withdrawal exclusion. Record active/ended/expired/mixed activity and assert safe shape/no session IDs; revoke membership and verify empty queries. Use existing runtime-role test setup/grants to exercise RLS without ownership.

```rust
assert!(k.team_context(&bob,"demo",TeamQuery::default()).await.unwrap().is_empty());
k.publish(&bob,"demo",card.id).await.unwrap();
assert_eq!(k.team_context(&reader,"demo",TeamQuery::default()).await.unwrap().len(),1);
```

- [ ] Run `scripts/cargo test -p wk-knowledge --test team -- --ignored`; expected missing module/method compilation failure. Implement explicit scoped queries through actor_tx, published/nonwithdrawn predicate, validated filter binds and visible published anchor subquery (same authorization predicate); decode shared Card plus author/time. Activity SELECT joins current membership, services containment, active status and expiry, limit50 ordered last_seen; output strictly allowlisted fields. Validate stored file paths and intent screening when decoding; reject unsafe legacy records rather than exposing them.

```rust
let row = sqlx::query("SELECT c.id,c.body,c.revision,c.published,c.author,(extract(epoch from c.created_at)*1000)::bigint AS created_at_ms FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.services @> c.services AND c.published AND NOT c.withdrawn ORDER BY c.created_at DESC,c.id DESC LIMIT 50");
```

- [ ] Write HTTP regression with missing identity401, reviewer published-only, unknown query422 without input echo, hidden author filter empty, activity safe shape. Run `scripts/cargo test -p wk-server --test team -- --ignored`; expected route404. Implement a focused team_routes module and GET merge on existing activity path with per-request actor. Run both tests and existing server/knowledge suite including disposable DB; expected PASS. Commit.

### Task 2: Hosted Team context browsing

**Files:** Create `web/src/TeamContext.tsx`, `web/tests/team.spec.ts`; modify `web/src/App.tsx`/types/style. Export existing `Detail` inspector for reuse from App; preserve development navigation.

**Interfaces:** TeamContext `{product:string,policy:Membership}` requests `{entries:{card:Card,author:string,created_at_ms:number}[],next_cursor:string|null}` and `{activity:SharedActivity[]}`. It opens existing authorized detail route and passes the existing inspector/actions/evidence components. Navigation adds Team context beside My context. Filters are server-side and reset cursor/list/detail; author is text input without a hidden member directory. File exact-match filter says workspace-relative reference. Optional activity section loads only on explicit request, explains separate opt-in and last-report expiry. Overlap button for selected published file references calls existing overlaps with limit8 and renders safe notice text.

- [ ] Write browser tests for honest empty state, published card/author/service/lifecycle/date, filter request and pagination, malicious HTML rendered as text, expired report labels, denied detail clears old selection, product switch aborts a delayed response, 390px no overflow. Run `npm --prefix web test -- team.spec.ts`; expected missing view. Implement abortable per-scope data fetching, details cleared before request, server-filtered pages and dedup append, allowlisted request query builder, existing Detail reuse. Review controls keep server authorization and do not imply lifecycle verification. Activity displays provided last-report/expiry and never hours worked. Empty/error messages reveal no hidden directory.

```ts
await page.getByRole('button',{name:'Team context',exact:true}).click();
await expect(page.getByText('Published summaries visible in your service scopes.')).toBeVisible();
```

- [ ] Run build/all browser tests and inspect desktop/mobile screenshots; expected PASS. Commit UI.

### Task 3: Live two-identity acceptance and final review

**Files:** Add `scripts/portal-team-smoke.py` using local provider test harness; add `docs/reviews/2026-10-10-team-context-acceptance.md`; update console/completion docs.

- [ ] Run real local-provider cookie identities alice/bob through published feed, private exclusion, narrower membership, explicit activity opt-in/end/expiry and withdrawal. Check real browser Team context against the live server with no API mocks. Reuse disposable wk_test only. Record bounded native coverage unchanged, no production/provider/load claims.
- [ ] Run Rust relevant suites/format/clippy, web build/browser tests and whitespace checks; expected PASS. Commit acceptance. Perform one fresh independent review of this increment, fix Important/Critical issues with RED→GREEN regressions, and continue Devices/Admin roadmap. No deployment or push.
