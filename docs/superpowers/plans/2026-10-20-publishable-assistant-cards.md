# Publishable assistant-written cards Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status (2026-10-20): Tasks 1-6 and Task 7 step 1 are implemented and reviewed (fmt, clippy -D warnings and the full workspace suite including Postgres tests pass). Not done: Task 7 step 2 (eval re-run) and Task 8 (docs, changelog, review note), which edit files that are still uncommitted in the main checkout; do them once the item-2 changes are committed.**

**Goal:** A card written by the developer's assistant (`wk-agent draft` / `/wk-summarize`) and submitted after the author's review can be published by a reviewer and then reaches teammates, while rule drafts, hook partials, imports and truncated cards stay unpublishable.

**Architecture:** Add an optional `provenance` field (`assistant_written`) to `CardSubmission`; `capture_completeness` keeps its meaning (`partial` = capture not verified complete). `Knowledge::publish` accepts `complete`, or `partial` with `assistant_written`; `correct` may not change provenance. The server advertises `card_provenance` in the policy so a new collector against an old server drops the field instead of failing. Portal, decisions API and docs show the origin. Spec: [`2026-10-19-publishable-assistant-cards-design.md`](../specs/2026-10-19-publishable-assistant-cards-design.md) (accepted 2026-10-20). No SQL migration.

**Tech Stack:** Rust workspace (`./scripts/cargo`), sqlx/PostgreSQL, axum, React/TypeScript + Playwright (`web/`), Python scripts.

**Prerequisites and conventions**

- Build on a base that already contains the adoption-friction item 2 changes (assistant-written default docs, `docs/reviews/2026-10-19-summary-quality-comparison.md`, `scripts/summary-quality-eval.py`, `rules_draft.rs` fixes). At the time of writing they are uncommitted in the main checkout; commit them first or Task 8 has nothing to edit and the spec's links to the review are broken. Refer to docs by section, not line number.
- Postgres tests are `#[ignore]` and need a disposable database named `wk_test`: `export WK_TEST_DATABASE_URL=postgres://USER@127.0.0.1:PORT/wk_test` and add `-- --ignored`. Never point it at anything else.
- Run all cargo commands as `./scripts/cargo ...` from the repository root. Commit after every task; end each commit message with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

## File structure

| File | Change |
|---|---|
| `crates/contracts/src/lib.rs` | `Provenance` enum, `CardSubmission.provenance`, validation |
| `crates/contracts/tests/provenance.rs` | new: contract tests |
| `docs/architecture/schemas/card-submission-v1.json`, `docs/architecture/schemas/effective-policy-v1.json`, `docs/architecture/schemas/README.md` | regenerated snapshots, one README line |
| `crates/summarize/src/lib.rs`, `crates/agent/src/assistant_draft.rs` | add `provenance` to struct literals (assistant sets it) |
| `crates/knowledge/src/lib.rs` | publish gate |
| `crates/knowledge/src/lifecycle.rs` | `Policy.features`; `correct` keeps provenance |
| `crates/knowledge/tests/provenance.rs` | new: Postgres tests |
| `crates/server/src/decisions.rs` | expose `provenance` |
| `crates/server/tests/http.rs` | policy advertises feature; publish over HTTP |
| `crates/agent/src/assistant_draft.rs`, `crates/agent/src/process.rs` | v2 digest, old-server fallback, tests |
| `web/src/types.ts`, `web/src/App.tsx`, `web/tests/assistant-cards.spec.ts` | portal |
| `docs/adapters/setup.md`, `README.md`, `CHANGELOG.md`, review doc | wording |

---

### Task 1: Contract field

**Files:**
- Modify: `crates/contracts/src/lib.rs` (after `Completeness`, in `CardSubmission`, in `validate`)
- Create: `crates/contracts/tests/provenance.rs`
- Modify: `crates/summarize/src/lib.rs:355-376`, `crates/agent/src/assistant_draft.rs:78-94` (struct literals)

- [ ] **Step 1: Write the failing tests**

`crates/contracts/tests/provenance.rs`:

```rust
use serde_json::{json, Value};
use wk_contracts::{validate_card, CardSubmission, Provenance};

fn card() -> Value {
    json!({"schema_version":1,"idempotency_key":"k1","policy_revision":1,"title":"Cap retries","intent":"Avoid repeat charges","rationale":"The gateway charges twice on duplicate calls.","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"partial","base_revision":null})
}

#[test]
fn provenance_is_absent_and_omitted_by_default() {
    let c: CardSubmission = serde_json::from_value(card()).unwrap();
    assert!(c.provenance.is_none());
    // Old servers use deny_unknown_fields: an ordinary card must serialize without the key.
    assert!(serde_json::to_value(&c).unwrap().get("provenance").is_none());
}

#[test]
fn accepts_assistant_written_on_a_partial_card() {
    let mut v = card();
    v["provenance"] = json!("assistant_written");
    assert!(validate_card(&v).is_ok());
    let c: CardSubmission = serde_json::from_value(v).unwrap();
    assert_eq!(c.provenance, Some(Provenance::AssistantWritten));
}

#[test]
fn rejects_unknown_provenance() {
    let mut v = card();
    v["provenance"] = json!("hand_waved");
    assert!(validate_card(&v).is_err());
}

#[test]
fn rejects_provenance_on_a_truncated_card() {
    let mut v = card();
    v["provenance"] = json!("assistant_written");
    v["capture_completeness"] = json!("truncated");
    assert!(validate_card(&v).is_err());
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `./scripts/cargo test -p wk-contracts --test provenance`
Expected: compile error, `Provenance` not found.

- [ ] **Step 3: Implement**

In `crates/contracts/src/lib.rs`, after the `Completeness` enum:

```rust
/// Who wrote a card, when that (and not capture coverage) is what makes it publishable.
/// Self-declared by the submitter, like `capture_completeness`; the reviewer is the check.
#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    AssistantWritten,
}
```

In `CardSubmission`, after `observed_at_ms`:

```rust
    /// Set only for a card the developer's assistant wrote and the developer reviewed before
    /// submitting. Omitted from the wire when absent so older servers still accept ordinary cards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
```

In `validate`, before `Ok(())`:

```rust
        if self.provenance.is_some() && self.capture_completeness == Completeness::Truncated {
            return Err("provenance cannot be set on a truncated card".into());
        }
```

Add `provenance: None,` after `observed_at_ms: None,` in `crates/summarize/src/lib.rs` (the `CardSubmission` literal in `card`). In `crates/agent/src/assistant_draft.rs` add `provenance: Some(wk_contracts::Provenance::AssistantWritten),` after `observed_at_ms: None,` (the assistant card is the one that carries it).

- [ ] **Step 4: Regenerate the schema snapshot**

Run:

```bash
./scripts/cargo run -q -p wk-contracts --example schema > docs/architecture/schemas/card-submission-v1.json
```

In `docs/architecture/schemas/README.md`, extend the `card-submission-v1.json` bullet with: ` Optional \`provenance\` (\`assistant_written\`) marks a card the developer's assistant wrote and the developer reviewed; it is additive and omitted when absent.`

- [ ] **Step 5: Run the tests**

Run: `./scripts/cargo test -p wk-contracts && ./scripts/cargo build --workspace`
Expected: all pass, workspace compiles (any other `CardSubmission { .. }` literal that fails to compile needs `provenance: None`; only the two above exist today).

- [ ] **Step 6: Commit**

```bash
git add crates/contracts crates/summarize crates/agent/src/assistant_draft.rs docs/architecture/schemas
git commit -m "feat: add an optional provenance field to card submissions"
```

---

### Task 2: Publish gate and correct immutability

**Files:**
- Modify: `crates/knowledge/src/lib.rs:306` (publish query)
- Modify: `crates/knowledge/src/lifecycle.rs` (`correct`, after the `old` revision check)
- Create: `crates/knowledge/tests/provenance.rs`

- [ ] **Step 1: Write the failing tests** (copy the `setup` helper from `crates/knowledge/tests/access.rs`: actor `a` with `can_review`, actor `b` without, product `product`, services backend/frontend)

`crates/knowledge/tests/provenance.rs`:

```rust
use wk_contracts::CardSubmission;
use wk_knowledge::{Actor, Error, Knowledge};

fn card(key: &str, completeness: &str, provenance: Option<&str>) -> CardSubmission {
    let mut v = serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Cap retries","intent":"Avoid repeat charges","rationale":"The gateway charges twice on duplicate calls.","decisions":["cap at three"],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":completeness,"base_revision":null});
    if let Some(p) = provenance {
        v["provenance"] = serde_json::json!(p);
    }
    serde_json::from_value(v).unwrap()
}
async fn setup() -> (Knowledge, Actor, Actor, String) {
    let url = std::env::var("WK_TEST_DATABASE_URL")
        .expect("set WK_TEST_DATABASE_URL to a disposable /wk_test database");
    assert!(url.split('?').next().unwrap().ends_with("/wk_test"), "refusing non-test database");
    let k = Knowledge::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let a = Actor { org: org.clone(), user: "a".into() };
    let b = Actor { org, user: "b".into() };
    let p = "product".to_string();
    k.grant(&a, &p, &["backend".into()], true).await.unwrap();
    k.grant(&b, &p, &["backend".into()], false).await.unwrap();
    (k, a, b, p)
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn reviewer_publishes_an_assistant_written_partial_card() {
    let (k, a, b, p) = setup().await;
    let c = k.submit(&a, &p, card("1", "partial", Some("assistant_written"))).await.unwrap();
    assert_eq!(k.publish(&b, &p, c.id).await.unwrap_err(), Error::Denied, "non-reviewer");
    k.publish(&a, &p, c.id).await.unwrap();
    assert_eq!(k.search(&b, &p, "retries", 10).await.unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn other_partial_cards_stay_unpublishable() {
    let (k, a, _b, p) = setup().await;
    let plain = k.submit(&a, &p, card("2", "partial", None)).await.unwrap();
    assert_eq!(k.publish(&a, &p, plain.id).await.unwrap_err(), Error::Denied);
    // `validate` already refuses truncated + provenance, so submit is the guard.
    assert_eq!(
        k.submit(&a, &p, card("3", "truncated", Some("assistant_written"))).await.unwrap_err(),
        Error::Invalid
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn correct_keeps_provenance_and_cannot_change_it() {
    let (k, a, _b, p) = setup().await;
    let c = k.submit(&a, &p, card("4", "partial", Some("assistant_written"))).await.unwrap();
    k.publish(&a, &p, c.id).await.unwrap();
    let mut next = card("4", "partial", Some("assistant_written"));
    next.base_revision = Some(1);
    next.intent = "Revised scope".into();
    let updated = k.correct(&a, &p, c.id, next).await.unwrap();
    assert!(!updated.published);
    assert!(updated.body.provenance.is_some());
    // Adding provenance to a plain partial card, or removing it, is refused.
    let plain = k.submit(&a, &p, card("5", "partial", None)).await.unwrap();
    let mut add = card("5", "partial", Some("assistant_written"));
    add.base_revision = Some(1);
    assert_eq!(k.correct(&a, &p, plain.id, add).await.unwrap_err(), Error::Invalid);
    let mut drop = card("4", "partial", None);
    drop.base_revision = Some(2);
    assert_eq!(k.correct(&a, &p, c.id, drop).await.unwrap_err(), Error::Invalid);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn publish_rescreens_the_stored_text() {
    let (k, a, _b, p) = setup().await;
    let c = k.submit(&a, &p, card("6", "partial", Some("assistant_written"))).await.unwrap();
    // Simulate a row edited after submit: the screen at publish must still refuse it.
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("UPDATE cards SET body=jsonb_set(body,'{rationale}','\"password = CANARY\"') WHERE id=$1")
        .bind(c.id).execute(&pool).await.unwrap();
    assert_eq!(k.publish(&a, &p, c.id).await.unwrap_err(), Error::Invalid);
}
```

If `sqlx` is not a dev-dependency of `wk-knowledge`, replace the last test's raw update with whatever the existing `retention`/`admin` tests use to alter a row, or drop that test and cover re-screening in the server test of Task 4 (`sensitive_correction_error_does_not_echo_canary` already covers the correct path); do not add a dependency for it. Check with `grep -n "sqlx" crates/knowledge/Cargo.toml`.

- [ ] **Step 2: Run to confirm failure**

Run: `./scripts/cargo test -p wk-knowledge --test provenance -- --ignored`
Expected: `reviewer_publishes_an_assistant_written_partial_card` FAILS with `Denied`; `correct_keeps_provenance...` FAILS (the add/remove calls succeed).

- [ ] **Step 3: Implement the gate**

In `crates/knowledge/src/lib.rs`, in `publish`, replace the predicate

```
AND c.body->>'capture_completeness'='complete'
```

with

```
AND (c.body->>'capture_completeness'='complete' OR (c.body->>'capture_completeness'='partial' AND c.body->>'provenance'='assistant_written'))
```

(absent provenance is `NULL`, so every existing row behaves as before).

- [ ] **Step 4: Implement immutability**

In `crates/knowledge/src/lifecycle.rs`, in `correct`, right after

```rust
        if old.revision != base {
            return Err(Error::Conflict);
        }
```

add:

```rust
        // The origin label is part of what made the card publishable; a correction keeps it as is.
        if old.body.provenance != body.provenance {
            return Err(Error::Invalid);
        }
```

- [ ] **Step 5: Run**

Run: `./scripts/cargo test -p wk-knowledge -- --ignored`
Expected: new tests PASS; existing `access`, `admin`, `team` tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/knowledge
git commit -m "feat: let reviewers publish assistant-written cards and keep their origin on correction"
```

---

### Task 3: Policy advertises the feature

**Files:**
- Modify: `crates/knowledge/src/lifecycle.rs` (`Policy` struct and `policy()`)
- Modify: `docs/architecture/schemas/effective-policy-v1.json` (regenerated)
- Test: `crates/server/tests/http.rs`

- [ ] **Step 1: Write the failing test** (append to `crates/server/tests/http.rs`, same style as `sensitive_correction_error_does_not_echo_canary`, Postgres, `#[ignore]`)

```rust
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn policy_advertises_card_provenance() {
    use http_body_util::BodyExt;
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = wk_knowledge::Knowledge::connect(&url).await.unwrap();
    let actor = wk_knowledge::Actor { org: uuid::Uuid::new_v4().to_string(), user: "owner".into() };
    k.grant(&actor, "demo", &["backend".into()], true).await.unwrap();
    let r = wk_server::router(k, vec![("synthetic-token".into(), actor)])
        .oneshot(
            Request::builder()
                .uri("/v1/policy?product=demo")
                .header("authorization", "Bearer synthetic-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v["policy"]["features"], serde_json::json!(["card_provenance"]));
}
```

(Confirm the imports at the top of `http.rs` already include `Request`, `Body`, `StatusCode`, `ServiceExt`; they do for the existing tests.)

- [ ] **Step 2: Run to confirm failure**

Run: `./scripts/cargo test -p wk-server --test http policy_advertises -- --ignored`
Expected: FAIL, `features` is null.

- [ ] **Step 3: Implement**

In `lifecycle.rs`, add to `Policy` after `restrictions`:

```rust
    /// Server capabilities a collector may rely on. `card_provenance`: accepts `provenance` on
    /// card submissions and lets a reviewer publish an `assistant_written` partial card.
    pub features: Vec<String>,
```

and in `policy()` add `features: vec!["card_provenance".into()],` to the returned `Policy`.

- [ ] **Step 4: Regenerate the policy schema**

Run: `./scripts/cargo run -q -p wk-knowledge --example schema > docs/architecture/schemas/effective-policy-v1.json`

- [ ] **Step 5: Run**

Run: `./scripts/cargo test -p wk-knowledge --test schema_snapshot && ./scripts/cargo test -p wk-server --test http -- --ignored`
Expected: PASS. Then `./scripts/cargo build --workspace` (any other `Policy { .. }` constructor, for example in device or remote MCP code, needs `features`; compiler errors name them).

- [ ] **Step 6: Commit**

```bash
git add crates docs/architecture/schemas/effective-policy-v1.json
git commit -m "feat: advertise card provenance support in the policy"
```

---

### Task 4: Decisions API shows provenance; publish over HTTP

**Files:**
- Modify: `crates/server/src/decisions.rs` (`card_state`)
- Test: `crates/server/src/decisions.rs` (unit test), `crates/server/tests/http.rs`

- [ ] **Step 1: Write the failing unit test** (append to `crates/server/src/decisions.rs`; `mod tests` already exists at about line 303, so add the test inside it)

```rust
    #[test]
    fn card_state_reports_provenance() {
        let body = serde_json::json!({"schema_version":1,"idempotency_key":"k","policy_revision":1,"title":"t","intent":"i","rationale":"r","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"partial","base_revision":null,"provenance":"assistant_written"});
        let card = wk_knowledge::Card {
            id: uuid::Uuid::nil(),
            body: serde_json::from_value(body).unwrap(),
            revision: 1,
            published: true,
        };
        assert_eq!(card_state(&card)["provenance"], "assistant_written");
        assert_eq!(card_state(&card)["coverage"], "partial");
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `./scripts/cargo test -p wk-server --lib card_state_reports_provenance`
Expected: FAIL (`provenance` is null).

- [ ] **Step 3: Implement**

In `card_state`, add `"provenance":c.body.provenance` after `"coverage":c.body.capture_completeness` in the `json!` object.

- [ ] **Step 4: Add the HTTP publish test** (append to `http.rs`, `#[ignore]`, same setup as Task 3): submit via `k.submit` a `partial` card with provenance `assistant_written`, POST `/v1/cards/{id}/publish` with body `{"product":"demo"}` and the bearer token, expect `StatusCode::OK`; repeat for a `partial` card without provenance and expect a non-2xx status (use `assert!(!r.status().is_success())`, because the exact mapping of `Denied` is already covered elsewhere).

- [ ] **Step 5: Run**

Run: `./scripts/cargo test -p wk-server -- --ignored && ./scripts/cargo test -p wk-server --lib`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/server
git commit -m "feat: report card provenance in decisions and cover publish over HTTP"
```

---

### Task 5: Collector sets provenance, v2 key, old-server fallback

**Files:**
- Modify: `crates/agent/src/assistant_draft.rs` (digest tag, new `adapt_to_server`, tests)
- Modify: `crates/agent/src/process.rs` (`deliver` assistant branch; `v` must become `mut`)

- [ ] **Step 1: Write the failing tests** in `assistant_draft.rs` `mod tests`:

```rust
    #[test]
    fn draft_is_assistant_written_with_a_v2_key() {
        let (key, draft) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        assert_eq!(draft["card"]["provenance"], "assistant_written");
        assert_eq!(draft["card"]["capture_completeness"], "partial");
        // The digest tag moved from "assistant-v1" so a resubmission after an upgrade is a new
        // card, not a body conflict with an older provenance-less row. Pin the tag change.
        assert!(include_str!("assistant_draft.rs").contains("\"assistant-v2\""));
        assert!(key.starts_with("draft:") && key.len() == 70);
    }

    #[test]
    fn provenance_is_dropped_for_a_server_that_does_not_advertise_it() {
        let (_, mut draft) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        assert!(adapt_to_server(&mut draft, &json!({"policy_revision":7})).is_some());
        assert!(draft["card"].get("provenance").is_none());
        let (_, mut draft) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        assert!(adapt_to_server(&mut draft, &json!({"features":["card_provenance"]})).is_none());
        assert_eq!(draft["card"]["provenance"], "assistant_written");
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `./scripts/cargo test -p wk-agent assistant_draft`
Expected: compile error, `adapt_to_server` not found.

- [ ] **Step 3: Implement**

In `assistant_draft.rs`, change the tag `"assistant-v1"` to `"assistant-v2"` in `build`'s digest tuple, and add:

```rust
/// Server capability that accepts `provenance` and publishes `assistant_written` cards.
const FEATURE: &str = "card_provenance";

/// Before submit: a server that does not advertise `card_provenance` (older than this change)
/// rejects the unknown field, so send the card as the plain `partial` draft it was before.
/// Returns the note to show the developer when the field was dropped.
pub(crate) fn adapt_to_server(draft: &mut Value, policy: &Value) -> Option<&'static str> {
    let supported = policy["features"]
        .as_array()
        .is_some_and(|f| f.iter().any(|x| x == FEATURE));
    if supported {
        return None;
    }
    draft["card"].as_object_mut()?.remove("provenance")?;
    Some("this server cannot publish assistant-written cards yet; the card was sent as a partial draft and cannot be published until the server is upgraded")
}
```

In `process.rs` `deliver`, make the draft mutable (`let mut v: Value = ...`), and in the `v.get("assistant")` branch, after the `match settle_assistant(...)` block and before `post_draft`:

```rust
        let note = crate::assistant_draft::adapt_to_server(&mut v, &p);
        let mut response = post_draft(dir, &c, &b, &t, &v, owner_only).await?;
        if let Some(note) = note {
            response["note"] = json!(note);
        }
```

(replace the existing `let response = post_draft(...)` line; the `if !owner_only { s.ack(id)?; }` that follows is unchanged). `p` is the policy value already fetched at the top of `deliver`.

- [ ] **Step 4: Pin that nothing else carries provenance**

In `process.rs` test around line 917 (`assert_eq!(draft["card"]["capture_completeness"], "partial");` in the rules-mode test), add below it: `assert!(draft["card"].get("provenance").is_none(), "rule drafts never carry provenance");`. In `memory_import.rs` near line 1414 (partial memory draft assertion) add the same line for `draft["card"]`. In `personal_sync.rs` tests, add:

```rust
    #[test]
    fn assistant_written_cards_are_never_synchronized() {
        let mut card = json!({"schema_version":1,"idempotency_key":"a","policy_revision":1,"title":"T","intent":"I","rationale":"R","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"partial","base_revision":null});
        card["provenance"] = json!("assistant_written");
        assert!(payload(&card).is_err());
    }
```

- [ ] **Step 5: Run**

Run: `./scripts/cargo test -p wk-agent`
Expected: all PASS (about 230 tests; two earlier assertions of `capture_completeness == "partial"` still hold).

- [ ] **Step 6: Commit**

```bash
git add crates/agent
git commit -m "feat: mark assistant drafts as assistant-written and fall back for older servers"
```

---

### Task 6: Portal

**Files:**
- Modify: `web/src/types.ts` (add `provenance?:string` to `Body`)
- Modify: `web/src/App.tsx` (four edits in `Workspace` results and `Detail`)
- Create: `web/tests/assistant-cards.spec.ts`

- [ ] **Step 1: Write the failing test**

`web/tests/assistant-cards.spec.ts`:

```ts
import {test,expect} from '@playwright/test';
test.beforeEach(async({page})=>{await page.route('**/auth/config',r=>r.fulfill({json:{browser_login:false,development:true}}));});
const body=(over:object)=>({schema_version:1,idempotency_key:'k',policy_revision:1,title:'Cap retries',intent:'Avoid repeat charges',rationale:'Gateway charges twice',decisions:[],constraints:[],open_questions:[],service_ids:['backend'],evidence:[],lifecycle:'proposed',capture_completeness:'partial',base_revision:null,...over});
const queue=[
 {id:'00000000-0000-0000-0000-000000000001',revision:1,published:false,body:body({title:'Assistant card',provenance:'assistant_written'})},
 {id:'00000000-0000-0000-0000-000000000002',revision:1,published:false,body:body({title:'[auto-drafted] Rule card'})},
 {id:'00000000-0000-0000-0000-000000000003',revision:1,published:false,body:body({title:'Truncated card',capture_completeness:'truncated'})},
];
async function openQueue(page:import('@playwright/test').Page){
 await page.route('**/v1/policy?*',r=>r.fulfill({json:{policy:{can_review:true}}}));
 await page.route('**/v1/products/demo/review',r=>r.fulfill({json:{cards:queue}}));
 await page.goto('/');await page.getByLabel('Product ID').fill('demo');await page.getByLabel('Access token').fill('SYNTHETIC');
 await page.getByRole('button',{name:'Connect',exact:true}).click();await page.getByRole('button',{name:'Review queue',exact:true}).click();
}
test('an assistant-written card can be published and says who wrote it',async({page})=>{
 await openQueue(page);
 await page.getByRole('button').filter({hasText:'Assistant card'}).click();
 await expect(page.getByText('Written by the author’s assistant',{exact:false})).toBeVisible();
 await expect(page.getByRole('note')).toContainText('Publishing makes it searchable');
 await expect(page.getByRole('button',{name:'Publish summary'})).toBeEnabled();
});
test('rule drafts and truncated cards still cannot be published',async({page})=>{
 await openQueue(page);
 for(const title of ['Rule card','Truncated card']){
  await page.getByRole('button').filter({hasText:title}).click();
  await expect(page.getByRole('button',{name:'Publish summary'})).toBeDisabled();
  await expect(page.getByRole('note')).toHaveCount(0);
 }
});
```

(If the page shows "Review queue" without a `policy` mock in non-hosted mode, drop the policy route; the console spec shows `**/v1/policy?*` is what the existing tests mock. The apostrophe in the notice and Source text is the typographic `’` used elsewhere in `App.tsx`; keep the test and component text identical.)

- [ ] **Step 2: Run to confirm failure**

Run: `cd web && npx playwright test assistant-cards`
Expected: first test FAILS (`Publish summary` disabled, no Source text).

- [ ] **Step 3: Implement**

`types.ts`: in `Body` add `provenance?:string;` after `capture_completeness:string;`.

`App.tsx`, four exact edits (the file is a few very long lines; use string replacement, not line numbers):

1. In `Detail`, after `const lists:[string,string[]][]=[...];` add:
   `const assistant=card.body.provenance==='assistant_written';const publishable=card.body.capture_completeness==='complete'||(card.body.capture_completeness==='partial'&&assistant);`
2. Detail facts row: replace
   `<dt>Capture coverage</dt><dd className={card.body.capture_completeness==='complete'?'':'warning'}>{readable(card.body.capture_completeness)}</dd>`
   with
   `{assistant?<><dt>Source</dt><dd>Written by the author’s assistant; submitted after the author’s review</dd></>:<><dt>Capture coverage</dt><dd className={card.body.capture_completeness==='complete'?'':'warning'}>{readable(card.body.capture_completeness)}</dd></>}`
3. Publish button: replace `disabled={busy||card.body.capture_completeness!=='complete'}` with `disabled={busy||!publishable}`, and immediately before `<div className="actions"><button disabled={busy} onClick={()=>setEditing(true)}>Correct summary</button>` insert
   `{!card.published&&assistant&&<p className="muted" role="note">Written by an assistant and not checked against the conversation. Read it for accuracy and for anything the team should not see. Publishing makes it searchable by everyone with access to these services and can appear in teammates’ assistant context.</p>}`
4. Result list: replace `{card.body.capture_completeness!=='complete'&&<span className="warning">{readable(card.body.capture_completeness)} capture</span>}` with `{card.body.provenance==='assistant_written'?<span className="muted">Assistant-written</span>:card.body.capture_completeness!=='complete'&&<span className="warning">{readable(card.body.capture_completeness)} capture</span>}`.

- [ ] **Step 4: Run**

Run: `cd web && npm run build && npx playwright test`
Expected: type-check and build pass; the new spec and all existing specs PASS (`console.spec.ts` complete-card behaviour is unchanged).

- [ ] **Step 5: Commit**

```bash
git add web
git commit -m "feat: show assistant-written cards in the portal and let reviewers publish them"
```

---

### Task 7: End-to-end proof against a real server

**Files:** none (verification); results go into the review doc in Task 8.

- [ ] **Step 1: Prove the default flow reaches a teammate**

Append to `crates/knowledge/tests/provenance.rs`:

```rust
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn a_published_assistant_card_reaches_a_teammate_and_a_plain_partial_one_does_not() {
    use wk_knowledge::activity::OverlapQuery;
    let (k, a, b, p) = setup().await;
    let q = || OverlapQuery { files: vec!["backend/retry.rs".into()], query: Some("cap retries".into()), limit: 8 };
    let plain = k.submit(&a, &p, card("7", "partial", None)).await.unwrap();
    assert!(k.publish(&a, &p, plain.id).await.is_err());
    assert!(k.overlaps(&b, &p, q()).await.unwrap().iter().all(|n| n.card_id != Some(plain.id)));
    let c = k.submit(&a, &p, card("8", "partial", Some("assistant_written"))).await.unwrap();
    k.publish(&a, &p, c.id).await.unwrap();
    let notices = k.overlaps(&b, &p, q()).await.unwrap();
    assert!(notices.iter().any(|n| n.card_id == Some(c.id)), "{notices:?}");
    assert!(k.overlaps(&a, &p, q()).await.unwrap().iter().all(|n| n.card_id != Some(c.id)), "own card is not a notice");
}
```

Run: `./scripts/cargo test -p wk-knowledge --test provenance -- --ignored`
Expected: PASS. This is the first end-to-end proof that the recommended flow's card produces a teammate notice. If the `related` text match does not fire for this title, adjust the query text to share a word with the card's title or intent (the matcher is full-text on those fields); do not weaken the assertion.

- [ ] **Step 2: Re-run the quality comparison**

Run the two commands in the review doc's "Reproduce" section. The script's `as-shipped` rows now publish the assistant cards for real (their drafts carry provenance, completeness stays `partial`) and still publish zero rule drafts.
Expected: `assistant` as-shipped prompt-time recall/precision about 0.81 / 0.40, `rules` as-shipped `published: 0`. If the assistant rows publish 0, the script is dropping `provenance` somewhere in `card_lines` (it copies `draft[mode]['card']` with `dict(card, ...)`, which keeps it) or the dump test predates Task 1; regenerate the drafts file.

- [ ] **Step 3: Commit nothing yet** (numbers are recorded in Task 8).

---

### Task 8: Docs and changelog

**Files:**
- Modify: `docs/adapters/setup.md`, `README.md`, `CHANGELOG.md`, `docs/reviews/2026-10-19-summary-quality-comparison.md`, the adoption plan status line, `docs/superpowers/specs/2026-10-19-publishable-assistant-cards-design.md` (status)
- Test: `scripts/tests/test_release_docs.py`

- [ ] **Step 1: Edit wording** (find each statement by its text)

- `setup.md`, "Summarize with your assistant": replace "Assistant drafts are partial, so owner synchronization … never uploads them; they need your explicit `submit`" so it keeps the owner-sync sentence and adds: "Assistant-written cards are `partial` (they were not captured) but carry an assistant-written origin, so a reviewer can publish them in the portal. Publishing makes the card searchable by everyone with access to its services and can surface it in teammates' prompts, so read it again for anything the team should not see; screening is pattern-based and is not proof of perfect privacy."
- `setup.md`, same file, the sentence "Reviewers publish complete approved summaries in the console. Partial hooks stay labeled partial and cannot be automatically published.": change to "Reviewers publish complete approved summaries, and assistant-written summaries, in the console. Partial hooks and rule drafts stay labeled partial and cannot be published."
- `setup.md`, "Set up without a local model": keep the rule-draft statements (still unpublishable); add "To get a card the team can see, use `/wk-summarize`."
- `README.md`: remove "cannot be published" from the assistant flow caveat in quick-start step 5 and the rule-draft bullet; keep it for rule drafts.
- `CHANGELOG.md` under Unreleased: "Assistant-written cards (`wk-agent draft`, `/wk-summarize`) can now be published by a reviewer. New optional `provenance` field on card submissions; the policy advertises `card_provenance`; upgrade the server before collectors. Rule drafts, hook partials, imports and truncated cards remain unpublishable."
- Review doc: replace "Finding: partial cards cannot be published" and follow-up 1 with a short note linking the spec, and record the Task 7 numbers.
- Spec: change status line to "Implemented" once Tasks 1-7 are merged.

- [ ] **Step 2: Run the doc checks**

Run: `python3 -m unittest discover scripts/tests` (or the repository's usual invocation) and `./scripts/cargo fmt --check && ./scripts/cargo clippy --workspace --all-targets -- -D warnings`
Expected: all PASS; fix any link or phrase assertion in `test_release_docs.py` that the new wording breaks.

- [ ] **Step 3: Commit**

```bash
git add docs README.md CHANGELOG.md
git commit -m "docs: assistant-written cards can be published by a reviewer"
```

---

## Self-review against the spec

- **Contract field, validation, schema snapshot, README line:** Task 1.
- **Publish gate; truncated never; rule drafts/hook/imports unchanged; correct immutability; re-screen at publish:** Task 2 (re-screen already exists; test pins it).
- **Policy `features`, old-server fallback, v2 digest, withdraw-and-resubmit path:** Tasks 3 and 5 (the withdraw-and-resubmit behaviour follows from the new key; documented in setup wording only if the owner wants it, not a separate task).
- **Decisions API provenance:** Task 4.
- **Portal Source row, notice, button, list label; rule drafts still disabled:** Task 6.
- **Owner sync unchanged and pinned; policy-auto never eligible (no code today):** Task 5 test; policy-auto note stays in the spec.
- **Privacy notice and docs wording; changelog; upgrade order:** Task 8.
- **Eval and end-to-end proof:** Task 7.
- **Not covered, deliberately:** file evidence for assistant cards, reviewer rewrite of rule drafts, operator switch, policy-auto (spec out-of-scope list).

Type names used consistently: `Provenance::AssistantWritten`, field `provenance`, feature string `card_provenance`, helper `adapt_to_server`, wire value `assistant_written`.
