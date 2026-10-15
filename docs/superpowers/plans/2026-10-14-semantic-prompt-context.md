# Meaning-based Prompt Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The prompt-time overlap lookup also returns teammates' published summaries that match the prompt keywords by meaning, fast (warm p95 < 100 ms) and relevant (≥ 11/12 paraphrases in the top five, no unrelated matches).

**Architecture:** A shared server function `overlaps_with` serves HTTP, MCP and device-signed lookups. It gates on policy and index readiness, embeds the keywords concurrently with the existing text lookup (cached, bounded to 1.5 s), fetches meaning candidates through a new knowledge query, and fuses them with text-related notices by reciprocal rank. The embedding client gains a query cache, configurable `keep_alive` and a warm-up call used at startup.

**Tech Stack:** Rust (axum, tokio, sqlx/PostgreSQL + pgvector), local Ollama Nomic embeddings.

**Spec:** `docs/superpowers/specs/2026-10-14-semantic-prompt-context-design.md`. Plan-time refinements (Task 6 updates the spec): related notices use the existing `matched_by` field with new values `meaning` and `both`; the text-only retrieval mode is labelled `lexical`, matching `/v1/search`; startup warm-up uses a separate short-lived client in a background task so startup never waits.

**Commands:** always `./scripts/cargo`. Database tests need `WK_TEST_DATABASE_URL` for a disposable pgvector `wk_test` database and run with `-- --include-ignored`. Gate before each commit: `./scripts/cargo fmt --all -- --check && ./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`. Commit with `git commit -s` plus the `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` trailer.

**Note for implementers:** code blocks are written against the real APIs read at plan time but were not compiled. Adapt minimally to compile, keep the tested behaviour, and report deviations.

---

## File structure

| File | Responsibility |
|---|---|
| Modify `crates/adapters/src/team_context.rs` | Keyword extraction stop list |
| Modify `crates/server/src/embeddings.rs` | Query cache, `keep_alive`, `embed_query_within`, `warm` |
| Create `crates/server/tests/embeddings.rs` | Cache/keep-alive tests against a fake loopback model (no database) |
| Modify `crates/knowledge/src/semantic.rs` | `related_by_meaning` candidate query |
| Modify `crates/knowledge/src/activity.rs` | `card_notice` visible to the crate |
| Modify `crates/knowledge/tests/hybrid_search.rs` | Candidate query database tests |
| Create `crates/server/src/prompt_context.rs` | `overlaps_with`, gate, `fuse_related` |
| Modify `crates/server/src/lib.rs`, `device_routes.rs` | Route HTTP/MCP/device lookups through `overlaps_with` |
| Modify `crates/server/src/main.rs` | Background warm-up and `WK_EMBEDDING_KEEP_ALIVE` |
| Modify `crates/server/tests/semantic.rs` | Overlap hybrid/fallback/cache database tests |
| Create `crates/server/tests/prompt_context_eval.rs` | Opt-in real-model relevance and latency evaluation |
| Modify docs | `overlap-notices.md`, `semantic-search.md`, README, CHANGELOG, spec |

---

### Task 1: Keyword stop list

**Files:** Modify `crates/adapters/src/team_context.rs` (the `keywords` function and its tests).

- [ ] **Step 1: Failing test.** Add to the module's tests:

```rust
#[test]
fn keywords_drop_question_words_and_keep_identifiers() {
    assert_eq!(
        keywords("Why must retries avoid repeating a payment for PAY-142 in RateLimiter?").as_deref(),
        Some("retries avoid repeating payment pay 142 ratelimiter")
    );
    assert_eq!(keywords("How does it work, what should we do?"), None);
}
```

- [ ] **Step 2:** Run `./scripts/cargo test -p wk-adapters --locked keywords_drop` — expect FAIL (`why`, `must`, `how` kept).
- [ ] **Step 3: Implement.** Extend `STOP` with: `"why","how","what","when","where","which","does","did","should","could","would","must","there","here","about","just","like","some","any","work","we","our","your","they","them","was","were","has","had","been","being","into","onto","over","under","more","most","also","very","only","then","than","there","these","those","who","whom","whose"`. Keep the remaining logic unchanged. (`142` stays because it has 3 characters; `pay` stays.)
- [ ] **Step 4:** Run `./scripts/cargo test -p wk-adapters --locked` — all pass. Existing keyword tests may need expectations updated only where they contain new stop words; report each change.
- [ ] **Step 5:** Gate and commit: `feat: focus prompt keywords on subject words`.

---

### Task 2: Embedding cache, keep-alive and warm-up

**Files:** Modify `crates/server/src/embeddings.rs`; create `crates/server/tests/embeddings.rs`.

- [ ] **Step 1: Failing tests** in `crates/server/tests/embeddings.rs` (no database; runs in normal CI):

```rust
use axum::{extract::State, routing::{get, post}, Json, Router};
use serde_json::{json, Value};
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex};
use std::time::Duration;
use wk_knowledge::semantic::Namespace;
use wk_server::embeddings::{parse_keep_alive, Config, Embeddings};

fn ns() -> Namespace {
    Namespace { model: "nomic-embed-text:v1.5".into(), digest: "a".repeat(64), dimensions: 3 }
}
#[derive(Clone, Default)]
struct Seen { embeds: Arc<AtomicUsize>, bodies: Arc<Mutex<Vec<Value>>> }
async fn model() -> (Config, Seen) {
    let seen = Seen::default();
    let app = Router::new()
        .route("/api/tags", get(|| async { Json(json!({"models":[{"name":ns().model,"digest":ns().digest}]})) }))
        .route("/api/embed", post(|State(s): State<Seen>, Json(body): Json<Value>| async move {
            s.embeds.fetch_add(1, Ordering::SeqCst);
            s.bodies.lock().unwrap().push(body);
            Json(json!({"model":ns().model,"embeddings":[[1.,0.,0.]]}))
        }))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/api/embed", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (Config { endpoint, namespace: ns() }, seen)
}
#[tokio::test]
async fn repeated_queries_are_served_from_the_cache() {
    let (config, seen) = model().await;
    let e = Embeddings::new(config).unwrap();
    let a = e.embed_query_within("retries avoid repeating payment", Duration::from_millis(1500)).await.unwrap();
    let b = e.embed_query_within("retries  avoid repeating payment", Duration::from_millis(1500)).await.unwrap();
    assert_eq!(a, b);
    assert_eq!(seen.embeds.load(Ordering::SeqCst), 1, "whitespace-normalized repeat is cached");
    e.embed_query("different words").await.unwrap();
    assert_eq!(seen.embeds.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn documents_and_rejected_input_bypass_the_cache() {
    let (config, seen) = model().await;
    let e = Embeddings::new(config).unwrap();
    e.embed_document("idempotency key").await.unwrap();
    e.embed_document("idempotency key").await.unwrap();
    assert_eq!(seen.embeds.load(Ordering::SeqCst), 2);
    assert_eq!(e.embed_query("password=PRIVATE_SYNTHETIC").await, Err("input_rejected"));
}
#[tokio::test]
async fn requests_keep_the_model_loaded_and_warm_up_embeds_once() {
    let (config, seen) = model().await;
    let e = Embeddings::new(config.clone()).unwrap();
    e.warm().await.unwrap();
    assert_eq!(seen.bodies.lock().unwrap()[0]["keep_alive"], "30m");
    let custom = Embeddings::new(config).unwrap().with_keep_alive("2h".into());
    custom.embed_query("rate limiter").await.unwrap();
    assert_eq!(seen.bodies.lock().unwrap()[1]["keep_alive"], "2h");
}
#[test]
fn keep_alive_values_are_bounded() {
    assert_eq!(parse_keep_alive("30m").as_deref(), Ok("30m"));
    assert_eq!(parse_keep_alive("24h").as_deref(), Ok("24h"));
    assert_eq!(parse_keep_alive("0s").as_deref(), Ok("0s"));
    for bad in ["25h", "1d", "-1m", "m", "30", "1500m", ""] {
        assert!(parse_keep_alive(bad).is_err(), "{bad}");
    }
}
```

- [ ] **Step 2:** Run `./scripts/cargo test -p wk-server --locked --test embeddings` — expect compile errors (`embed_query_within`, `warm`, `with_keep_alive`, `parse_keep_alive`).
- [ ] **Step 3: Implement** in `crates/server/src/embeddings.rs`:
  - Update the module doc: "Explicit loopback-only Ollama embeddings; query vectors are cached in memory only."
  - Add constants and the cache:

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
const QUERY_CACHE: usize = 1024;
const QUERY_TTL: Duration = Duration::from_secs(600);
#[derive(Default)]
struct QueryCache { entries: HashMap<String, (Vec<f32>, Instant, u64)>, tick: u64 }
impl QueryCache {
    fn get(&mut self, key: &str, now: Instant) -> Option<Vec<f32>> {
        self.tick += 1;
        let tick = self.tick;
        match self.entries.get_mut(key) {
            Some((v, at, used)) if now.duration_since(*at) < QUERY_TTL => { *used = tick; Some(v.clone()) }
            Some(_) => { self.entries.remove(key); None }
            None => None,
        }
    }
    fn put(&mut self, key: String, vector: Vec<f32>, now: Instant) {
        self.tick += 1;
        if self.entries.len() >= QUERY_CACHE && !self.entries.contains_key(&key) {
            if let Some(oldest) = self.entries.iter().min_by_key(|(_, (_, _, used))| *used).map(|(k, _)| k.clone()) {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (vector, now, self.tick));
    }
}
/// Ollama `keep_alive`: an integer with `s`, `m` or `h`, at most 24 hours.
pub fn parse_keep_alive(value: &str) -> Result<String, String> {
    let (number, unit) = value.split_at(value.len().saturating_sub(1));
    let seconds = match (number.parse::<u64>(), unit) {
        (Ok(n), "s") if !number.is_empty() => n,
        (Ok(n), "m") if !number.is_empty() => n.saturating_mul(60),
        (Ok(n), "h") if !number.is_empty() => n.saturating_mul(3600),
        _ => return Err("invalid embedding keep_alive".into()),
    };
    if seconds > 86_400 { return Err("embedding keep_alive exceeds 24h".into()); }
    Ok(value.to_string())
}
```

  - `Embeddings` gains `keep_alive: String` (default `"30m"` in `new`) and `cache: Mutex<QueryCache>`; add `pub fn with_keep_alive(mut self, value: String) -> Self { self.keep_alive = value; self }`.
  - `from_env` applies `WK_EMBEDDING_KEEP_ALIVE` through `parse_keep_alive` when set.
  - Query methods:

```rust
pub async fn embed_query(&self, text: &str) -> Result<Vec<f32>, &'static str> {
    self.embed_query_within(text, Duration::from_secs(3)).await
}
/// Cached query embedding within `deadline`. The cache holds vectors only, in memory.
pub async fn embed_query_within(&self, text: &str, deadline: Duration) -> Result<Vec<f32>, &'static str> {
    let key = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(v) = self.cache.lock().map_err(|_| "embedding_unavailable")?.get(&key, Instant::now()) {
        return Ok(v);
    }
    let vector = self.bounded_embed(text, true, deadline).await?;
    self.cache.lock().map_err(|_| "embedding_unavailable")?.put(key, vector.clone(), Instant::now());
    Ok(vector)
}
/// Load the model before the first prompt; never cached.
pub async fn warm(&self) -> Result<(), &'static str> {
    self.bounded_embed("warm up", true, Duration::from_secs(30)).await.map(|_| ())
}
```

  - In `embed`, replace `"keep_alive":"5m"` with `"keep_alive":self.keep_alive`.
- [ ] **Step 4:** Run `./scripts/cargo test -p wk-server --locked --test embeddings` — 4 passed. Run `./scripts/cargo test -p wk-server --locked` — existing tests pass.
- [ ] **Step 5:** Gate and commit: `feat: cache query embeddings and keep the model loaded`.

---

### Task 3: Meaning candidates for overlap notices

**Files:** Modify `crates/knowledge/src/semantic.rs`, `crates/knowledge/src/activity.rs`; test in `crates/knowledge/tests/hybrid_search.rs` (read its existing fixture first and reuse it).

- [ ] **Step 1: Failing database test** (add to `hybrid_search.rs`, `#[ignore = "requires disposable pgvector PostgreSQL"]`). Using that file's existing setup helpers: migrate the semantic schema; alice submits card A (vector `[1,0,0]`) and card B (vector `[0,1,0]`), bob publishes both; store embeddings with `semantic_store`. Then assert:
  - `k.related_by_meaning(&reader, "demo", &ns, &[1.,0.,0.], &[], 20)` returns exactly A, with `kind == "related"`, `matched_by == "meaning"`, `developer == "alice"` (B is below the 0.6 cosine floor).
  - Called as alice (the author) it returns nothing.
  - With `exclude = [A.id]` it returns nothing.
  - After withdrawing A it returns nothing; a product the reader is not a member of yields `Err(NotFound)`.
- [ ] **Step 2:** Run `./scripts/cargo test -p wk-knowledge --locked --test hybrid_search -- --include-ignored related_by_meaning` — expect compile error.
- [ ] **Step 3: Implement.** In `activity.rs` change `fn card_notice` to `pub(crate) fn card_notice`. In `semantic.rs` add to `impl Knowledge`:

```rust
/// Published summaries by other developers that match `vector` by meaning,
/// within the caller's services, excluding `exclude`. Cosine similarity >= 0.6.
pub async fn related_by_meaning(
    &self,
    a: &Actor,
    p: &str,
    ns: &Namespace,
    vector: &[f32],
    exclude: &[uuid::Uuid],
    limit: u32,
) -> Result<Vec<crate::activity::Notice>, Error> {
    if !(1..=20).contains(&limit) || exclude.len() > 100 {
        return Err(Error::Invalid);
    }
    let encoded = ns.vector(vector)?;
    let mut tx = self.code_tx(a, p).await?;
    let services = self.semantic_services(&mut tx, a, p).await?;
    let now_ms: i64 = sqlx::query_scalar("SELECT (extract(epoch from now())*1000)::bigint")
        .fetch_one(&mut *tx).await.map_err(db)?;
    let rows = sqlx::query("SELECT c.id,c.revision,c.author,c.body,(extract(epoch from c.created_at)*1000)::bigint AS at FROM card_embeddings e JOIN cards c ON c.org=e.org AND c.product=e.product AND c.id=e.card_id AND c.revision=e.revision JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.services @> c.services AND $4::text[] @> c.services AND c.author<>$1 AND NOT (c.id = ANY($11::uuid[])) AND c.published AND NOT c.withdrawn AND c.body->>'lifecycle' NOT IN ('superseded','deprecated') AND e.model=$5 AND e.digest=$6 AND e.dimensions=$7 AND e.format=$8 AND e.status='ready' AND (e.embedding <=> $9::vector)<=0.4 ORDER BY e.embedding <=> $9::vector,c.id LIMIT $10")
        .bind(&a.user).bind(&a.org).bind(p).bind(&services).bind(&ns.model).bind(&ns.digest)
        .bind(ns.dimensions as i32).bind(FORMAT).bind(&encoded).bind(i64::from(limit)).bind(exclude)
        .fetch_all(&mut *tx).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    rows.into_iter()
        .map(|r| {
            let body: serde_json::Value = r.try_get("body").map_err(db)?;
            Ok(crate::activity::card_notice(
                "related", "meaning", r.try_get("author").map_err(db)?, r.try_get("id").map_err(db)?,
                r.try_get("revision").map_err(db)?, &body, vec![], r.try_get("at").map_err(db)?, now_ms,
            ))
        })
        .collect()
}
```

- [ ] **Step 4:** Run the new test and `./scripts/cargo test -p wk-knowledge --locked -- --include-ignored` — all pass.
- [ ] **Step 5:** Gate and commit: `feat: find related summaries by meaning for overlap notices`.

---

### Task 4: Shared overlap lookup with meaning, fusion and diagnostics

**Files:** Create `crates/server/src/prompt_context.rs`; modify `crates/server/src/lib.rs` (`overlaps_for`, module declaration), `crates/server/src/device_routes.rs` (`"overlaps"` arm), `crates/server/src/main.rs` (warm-up); tests in `crates/server/tests/semantic.rs` and unit tests in `prompt_context.rs`.

- [ ] **Step 1: Failing unit tests** for fusion in `prompt_context.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn notice(kind: &'static str, by: &'static str, id: u128) -> Notice {
        Notice { kind, matched_by: by, developer: "dev".into(), summary: format!("s{id}"),
            matched_files: vec![], card_id: Some(uuid::Uuid::from_u128(id)), revision: Some(1),
            lifecycle: Some("proposed".into()), open_questions: vec![], when_ms: 0, text: format!("t{id}") }
    }
    #[test]
    fn fusion_keeps_other_notices_first_marks_both_and_caps_meaning() {
        let base = vec![notice("active", "file", 1), notice("related", "text", 2), notice("related", "text", 3)];
        let meaning = (2..=9).map(|i| notice("related", "meaning", i)).collect();
        let fused = fuse_related(base, meaning, 20);
        assert_eq!(fused[0].kind, "active");
        let related: Vec<_> = fused[1..].iter().map(|n| (n.card_id.unwrap().as_u128(), n.matched_by)).collect();
        assert_eq!(related[0], (2, "both"));
        assert_eq!(related.iter().filter(|(_, by)| *by == "meaning").count(), 5, "at most 5 meaning-only");
        assert!(related.contains(&(3, "both")));
    }
    #[test]
    fn fusion_respects_the_limit_and_keeps_text_only_results_without_meaning() {
        let base = vec![notice("related", "text", 2)];
        assert_eq!(fuse_related(base.clone(), vec![], 8)[0].matched_by, "text");
        let many = (1..=10).map(|i| notice("related", "meaning", i)).collect();
        assert_eq!(fuse_related(vec![], many, 3).len(), 3);
    }
}
```

- [ ] **Step 2: Failing database tests** in `crates/server/tests/semantic.rs` (ignored, pgvector). Add a helper `overlap(app, token, body) -> (StatusCode, Value)` posting to `/v1/products/demo/overlaps`. Reuse the existing fixture pattern (alice submits the payments card, bob publishes, `index_once` with the fake model returning `[1,0,0]`, runtime role grants, router with `reader` token). Assert:
  1. `{"query":"duplicate money","limit":8}` as reader → the card appears with `matched_by == "meaning"`, `retrieval.mode == "hybrid"`, and `text` contains its title.
  2. `{"query":"retries payment","limit":8}` → the card appears once with `matched_by == "both"`.
  3. The same query again does not increase the fake model's embed count beyond the first lookup's (cache) — count `/api/embed` calls only (track them separately from `/api/tags`).
  4. `{"files":["backend/x.rs"],"limit":8}` (no query) → no provider call; `retrieval == {"mode":"lexical","reason":null}`.
  5. `{"query":"password=PRIVATE_SYNTHETIC"}` → `query_ignored == true`, no provider call.
  6. Model task aborted → card still found by text for `retries payment` with `matched_by == "text"`, `retrieval.reason == "embedding_unavailable"`.
  7. Product policy `allow_loopback=false` → no provider call, `retrieval.reason == "embedding_policy_disabled"`.
  8. Without `semantic_store` for the card (fresh fixture) → `retrieval.reason == "index_not_ready"`, no provider call.
  9. MCP `knowledge.overlaps` with `{"query":"duplicate money"}` returns the same `meaning` notice and `retrieval.mode == "hybrid"`.
  10. Device route: if `crates/server/tests` already has a signed-collector harness (search for `"/collector/"`), add one `overlaps` request asserting `retrieval.mode == "hybrid"`; otherwise rely on the shared function and note it in the report.
- [ ] **Step 3:** Run both test sets — expect compile errors / failures.
- [ ] **Step 4: Implement** `crates/server/src/prompt_context.rs`:

```rust
//! Prompt-time overlap lookup: existing file/text notices plus meaning matches.
use crate::*;
use std::collections::HashMap;
use std::time::Duration;
use wk_knowledge::activity::{Notice, OverlapQuery};
const PROMPT_BUDGET: Duration = Duration::from_millis(1500);
const MEANING_CANDIDATES: u32 = 20;
const MEANING_ADDED: usize = 5;

async fn gate(k: &Knowledge, a: &Actor, p: &str, client: &embeddings::Embeddings) -> Result<Option<&'static str>, Failure> {
    if !k.policy(a, p).await?.restrictions.allow_loopback {
        return Ok(Some("embedding_policy_disabled"));
    }
    match k.semantic_ready(a, p, &client.namespace).await {
        Ok(true) => Ok(None),
        Ok(false) => Ok(Some("index_not_ready")),
        Err(Error::Storage) => Ok(Some("semantic_storage_unavailable")),
        Err(e) => Err(e.into()),
    }
}

pub(crate) async fn overlaps_with(api: &Api, k: &Knowledge, a: &Actor, product: &str, mut q: OverlapQuery) -> Result<Value, Failure> {
    // Possibly sensitive request text is dropped, not sent to search; file matching still runs.
    let query_ignored = q.query.as_deref().is_some_and(wk_summarize::sensitive);
    if query_ignored {
        q.query = None;
        if q.files.is_empty() {
            return Ok(json!({"notices":[],"text":"","query_ignored":true,"retrieval":{"mode":"lexical","reason":null}}));
        }
    }
    let limit = q.limit as usize;
    let query = q.query.clone().filter(|s| !s.trim().is_empty());
    let (notices, embedded) = match (query, &api.embeddings) {
        (None, _) => (k.overlaps(a, product, q).await?, Err(None)),
        (Some(_), None) => (k.overlaps(a, product, q).await?, Err(Some("semantic_disabled"))),
        (Some(text), Some(client)) => match gate(k, a, product, client).await? {
            Some(reason) => (k.overlaps(a, product, q).await?, Err(Some(reason))),
            None => {
                let (notices, vector) = tokio::join!(k.overlaps(a, product, q), client.embed_query_within(&text, PROMPT_BUDGET));
                (notices?, vector.map_err(Some))
            }
        },
    };
    let (fused, retrieval) = match embedded {
        Err(reason) => (notices, json!({"mode":"lexical","reason":reason})),
        Ok(vector) => {
            let client = api.embeddings.as_ref().expect("checked above");
            if !k.policy(a, product).await?.restrictions.allow_loopback {
                (notices, json!({"mode":"lexical","reason":"embedding_policy_disabled"}))
            } else {
                let exclude: Vec<uuid::Uuid> = notices.iter().filter(|n| n.kind != "related").filter_map(|n| n.card_id).collect();
                match k.related_by_meaning(a, product, &client.namespace, &vector, &exclude, MEANING_CANDIDATES).await {
                    Ok(meaning) => (fuse_related(notices, meaning, limit), json!({"mode":"hybrid","reason":null})),
                    Err(Error::Storage) => (notices, json!({"mode":"lexical","reason":"semantic_storage_unavailable"})),
                    Err(e) => return Err(e.into()),
                }
            }
        }
    };
    Ok(json!({"text":wk_knowledge::activity::render(&fused),"notices":fused,"query_ignored":query_ignored,"retrieval":retrieval}))
}

/// Non-related notices keep their order; related notices are merged by reciprocal rank
/// (constant 60, stable ID ties). Cards found both ways become `both`; at most
/// `MEANING_ADDED` meaning-only cards are added.
pub(crate) fn fuse_related(notices: Vec<Notice>, meaning: Vec<Notice>, limit: usize) -> Vec<Notice> {
    let (mut out, text): (Vec<Notice>, Vec<Notice>) = notices.into_iter().partition(|n| n.kind != "related");
    let in_text: std::collections::HashSet<uuid::Uuid> = text.iter().filter_map(|n| n.card_id).collect();
    let mut scored: HashMap<uuid::Uuid, (Notice, f64)> = HashMap::new();
    for (rank, n) in text.into_iter().enumerate() {
        if let Some(id) = n.card_id { scored.insert(id, (n, 1. / (61. + rank as f64))); }
    }
    let mut added = 0;
    for (rank, n) in meaning.into_iter().enumerate() {
        let Some(id) = n.card_id else { continue };
        let score = 1. / (61. + rank as f64);
        if in_text.contains(&id) {
            if let Some((existing, s)) = scored.get_mut(&id) { existing.matched_by = "both"; *s += score; }
        } else if added < MEANING_ADDED {
            added += 1;
            scored.insert(id, (n, score));
        }
    }
    let mut related: Vec<(Notice, f64)> = scored.into_values().collect();
    related.sort_by(|(a, x), (b, y)| y.total_cmp(x).then_with(|| a.card_id.cmp(&b.card_id)));
    out.extend(related.into_iter().map(|(n, _)| n));
    out.truncate(limit);
    out
}
```

  - In `lib.rs`: add `mod prompt_context;`; make `overlaps_for` delegate: `prompt_context::overlaps_with(api, &api.knowledge, a, product, q).await`.
  - In `device_routes.rs` `"overlaps"` arm: replace the body with `crate::prompt_context::overlaps_with(&api, &k, &who, &e.product, serde_json::from_value(e.request).map_err(|_| invalid())?).await?` (convert the error type the same way neighbouring arms do).
  - In `main.rs`, where the router is built with `Embeddings::from_env()`, before it: if the embedding configuration parses, build a second client from the same `Config` and `tokio::spawn(async move { if let Err(reason) = warm.warm().await { eprintln!("embedding warm-up: {reason}"); } });`. Startup never waits on it.
- [ ] **Step 5:** Run the unit tests and `./scripts/cargo test -p wk-server --locked -- --include-ignored` — all pass, including existing overlap and device tests.
- [ ] **Step 6:** Gate and commit: `feat: add meaning matches to prompt-time overlap notices`.

---

### Task 5: Relevance and latency evaluation

**Files:** Create `crates/server/tests/prompt_context_eval.rs`; add `wk-adapters` to `[dev-dependencies]` of `crates/server/Cargo.toml` if not present.

- [ ] **Step 1: Write the opt-in evaluation test** (`#[ignore = "requires pgvector PostgreSQL and a real local Nomic model"]`), skipped unless `WK_EVAL_EMBEDDING_URL`, `WK_EVAL_EMBEDDING_MODEL`, `WK_EVAL_EMBEDDING_DIGEST` are set (dimensions 768). It:
  1. Loads `scripts/fixtures/semantic-search.json`. For each record, alice submits a card (title, intent, rationale from the record; `service_ids: ["backend"]`), bob publishes it.
  2. Migrates and indexes with a real `Embeddings` via `wk_server::semantic::index_once`, then builds the router with the reader token (same runtime-role setup as `semantic.rs`).
  3. For each record with a `query`: `kw = wk_adapters::team_context::keywords(query)`; POST overlaps `{"query":kw,"limit":8}` as reader; record whether the record's card id is in the first five related notices, its `matched_by`, and the request latency. Run the whole set twice; the first pass is "cold" (empty cache), the second "warm".
  4. For each `unanswerable` query: assert no related notice has `matched_by` `meaning` or `both`.
  5. Repeat step 3 against a router built without embeddings to record text-only results.
  6. Print one JSON report: top-5 hits (hybrid/text), MRR@5, unanswerable leaks, cold and warm p50/p95 ms.
  7. Assert: hybrid top-5 hits ≥ 11 of 12; unanswerable leaks == 0; warm p95 < 100 ms.
- [ ] **Step 2:** Confirm it is skipped without the environment: `./scripts/cargo test -p wk-server --locked --test prompt_context_eval -- --include-ignored` passes quickly with a "skipped" message.
- [ ] **Step 3:** Gate and commit: `test: evaluate prompt context relevance and latency with a real model`.
- [ ] **Step 4 (controller):** run it against an isolated local Ollama with the pinned Nomic model and record the report in `docs/reviews/2026-10-14-semantic-prompt-context-acceptance.md`. If a gate fails, stop and report; do not tune thresholds silently.

---

### Task 6: Documentation

**Files:** `docs/architecture/overlap-notices.md`, `docs/architecture/semantic-search.md`, `README.md`, `CHANGELOG.md`, the spec.

- [ ] **Step 1:** `overlap-notices.md`: in "Lookup", related notices may match by `text`, `meaning` or `both`; meaning matches need optional semantic search and product local-inference policy; at most 5 meaning-only additions; response `retrieval` field and reasons; only keywords leave the developer machine.
- [ ] **Step 2:** `semantic-search.md`: query cache (1,024 vectors, 10 minutes, memory only), `WK_EMBEDDING_KEEP_ALIVE` (default 30m, max 24h), background warm-up, prompt-time budget 1.5 s; state that the exact scan should be re-evaluated (HNSW with iterative filtered scans) when current summaries per product exceed about 50,000 or warm p95 exceeds 100 ms.
- [ ] **Step 3:** README "Coding-agent integration": one sentence that prompt-time team notices include meaning-based matches when semantic search is enabled. CHANGELOG: one bullet under the topmost Unreleased section.
- [ ] **Step 4:** Spec: replace `match: "text" | "meaning" | "both"` with the `matched_by` values, `mode: "text"` with `lexical`, and describe the background warm-up client.
- [ ] **Step 5:** Check relative links; commit: `docs: document meaning-based prompt context`.

---

### Task 7: Full verification

- [ ] fmt, workspace clippy `-D warnings`, `cargo test --workspace --locked -- --include-ignored` against a disposable pgvector database, build, Python tests, demo, plain-PostgreSQL acceptance, integration script, console build and tests, `cargo audit`.
- [ ] The Task 5 evaluation report with a real model, recorded honestly.
