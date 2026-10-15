//! Opt-in relevance and latency evaluation of prompt-time overlap notices against a real local
//! Nomic model. Set `WK_EVAL_EMBEDDING_URL`, `WK_EVAL_EMBEDDING_MODEL` and
//! `WK_EVAL_EMBEDDING_DIGEST` (768 dimensions) plus `WK_TEST_DATABASE_URL` to run it. The prompt
//! similarity floor is the product default unless `WK_PROMPT_SIMILARITY_FLOOR` overrides it.
#[path = "../../knowledge/tests/common/mod.rs"]
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::Row;
use std::time::Instant;
use tower::ServiceExt;
use wk_knowledge::semantic::Namespace;
use wk_server::embeddings::{parse_prompt_floor, Config, Embeddings};

fn config() -> Option<Config> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    Some(Config {
        endpoint: var("WK_EVAL_EMBEDDING_URL")?,
        namespace: Namespace {
            model: var("WK_EVAL_EMBEDDING_MODEL")?,
            digest: var("WK_EVAL_EMBEDDING_DIGEST")?,
            dimensions: 768,
        },
    })
}

async fn overlap(app: &Router, query: &str) -> (Value, f64) {
    let started = Instant::now();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/products/demo/overlaps")
                .header("content-type", "application/json")
                .header("authorization", "Bearer reader")
                .body(Body::from(json!({"query":query,"limit":8}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(status, StatusCode::OK);
    (serde_json::from_slice(&bytes).unwrap(), elapsed)
}

fn related(result: &Value) -> Vec<&Value> {
    result["notices"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "related")
        .collect()
}

/// One query outcome: 1-based rank among the first five related notices, and how it matched.
struct Outcome {
    rank: Option<usize>,
    matched_by: Value,
    mode: Value,
    ms: f64,
}

async fn run(app: &Router, cases: &[(String, String, String)]) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for (_, keywords, card) in cases {
        let (result, ms) = overlap(app, keywords).await;
        let found = related(&result);
        let rank = found.iter().take(5).position(|n| n["card_id"] == *card);
        outcomes.push(Outcome {
            rank: rank.map(|r| r + 1),
            matched_by: rank.map_or(Value::Null, |r| found[r]["matched_by"].clone()),
            mode: result["retrieval"]["mode"].clone(),
            ms,
        });
    }
    outcomes
}

fn hits(outcomes: &[Outcome]) -> usize {
    outcomes.iter().filter(|o| o.rank.is_some()).count()
}

fn mrr(outcomes: &[Outcome]) -> f64 {
    outcomes
        .iter()
        .filter_map(|o| o.rank)
        .map(|r| 1.0 / r as f64)
        .sum::<f64>()
        / outcomes.len() as f64
}

/// Nearest-rank percentile in milliseconds.
fn percentile(outcomes: &[Outcome], p: f64) -> f64 {
    let mut ms: Vec<f64> = outcomes.iter().map(|o| o.ms).collect();
    ms.sort_by(f64::total_cmp);
    let index = ((p / 100.0) * ms.len() as f64).ceil() as usize;
    ms[index.clamp(1, ms.len()) - 1]
}

fn round(ms: f64) -> f64 {
    (ms * 10.0).round() / 10.0
}

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

// Mirrors of the product's fusion constants, used only to replay fusion at other floors.
const FLOORS: [f64; 5] = [0.40, 0.45, 0.50, 0.55, 0.60];
const MEANING_CANDIDATES: usize = 20;
const MEANING_ADDED: usize = 5;
const RANK_CONSTANT: f64 = 61.0;
const LIMIT: usize = 8;

/// The text route's answer for a prompt: card IDs of non-related notices, how many notices are
/// not related, and the related card IDs in order.
struct TextNotices {
    shown: Vec<String>,
    others: usize,
    related: Vec<String>,
}

async fn text_notices(app: &Router, keywords: &str) -> TextNotices {
    let (result, _) = overlap(app, keywords).await;
    let notices = result["notices"].as_array().unwrap();
    let id = |n: &Value| n["card_id"].as_str().map(str::to_string);
    let (related, others): (Vec<&Value>, Vec<&Value>) =
        notices.iter().partition(|n| n["kind"] == "related");
    TextNotices {
        shown: others.iter().filter_map(|n| id(n)).collect(),
        others: others.len(),
        related: related.iter().filter_map(|n| id(n)).collect(),
    }
}

/// Cosine similarity of `vector` to every current card vector the reader may see, highest
/// first. Every fixture card is in the reader's `backend` service, so membership is implied.
async fn similarities(f: &common::Fixture, ns: &Namespace, vector: &[f32]) -> Vec<(String, f64)> {
    sqlx::query("SELECT c.id::text AS id, 1 - (e.embedding <=> $1::vector) AS sim FROM card_embeddings e JOIN cards c ON c.org=e.org AND c.product=e.product AND c.id=e.card_id AND c.revision=e.revision WHERE c.org=$2 AND c.product='demo' AND c.author<>$3 AND c.published AND NOT c.withdrawn AND c.body->>'lifecycle' NOT IN ('superseded','deprecated') AND e.model=$4 AND e.digest=$5 AND e.dimensions=$6 AND e.format=$7 AND e.status='ready' ORDER BY e.embedding <=> $1::vector, c.id")
        .bind(serde_json::to_string(vector).unwrap())
        .bind(&f.reader.org)
        .bind(&f.reader.user)
        .bind(&ns.model)
        .bind(&ns.digest)
        .bind(ns.dimensions as i32)
        .bind(wk_knowledge::semantic::FORMAT)
        .fetch_all(&f.pool)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.try_get("id").unwrap(), r.try_get("sim").unwrap()))
        .collect()
}

/// Replays `fuse_related` on card IDs with meaning candidates at or above `floor`: the 1-based
/// rank of `target` among the first five visible related notices, and whether meaning found it.
fn simulate(
    target: &str,
    text: &TextNotices,
    sims: &[(String, f64)],
    floor: f64,
) -> (Option<usize>, bool) {
    let meaning: Vec<&String> = sims
        .iter()
        .filter(|(id, s)| *s >= floor && !text.shown.contains(id))
        .take(MEANING_CANDIDATES)
        .map(|(id, _)| id)
        .collect();
    let score = |rank: usize| 1.0 / (RANK_CONSTANT + rank as f64);
    let mut related: Vec<(&String, f64)> = text
        .related
        .iter()
        .enumerate()
        .map(|(rank, id)| (id, score(rank)))
        .collect();
    let mut added = 0;
    for (rank, &id) in meaning.iter().enumerate() {
        if let Some((_, s)) = related.iter_mut().find(|(t, _)| *t == id) {
            *s += score(rank);
        } else if added < MEANING_ADDED {
            added += 1;
            related.push((id, score(rank)));
        }
    }
    related.sort_by(|(a, x), (b, y)| y.total_cmp(x).then_with(|| a.cmp(b)));
    let visible = LIMIT.saturating_sub(text.others).min(5);
    let rank = related
        .iter()
        .take(visible)
        .position(|(id, _)| id.as_str() == target);
    (
        rank.map(|r| r + 1),
        meaning.iter().any(|id| id.as_str() == target),
    )
}

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../scripts/fixtures/semantic-search.json"
    ))
    .unwrap()
}

/// `(record id, prompt keywords)` for each `meaning_only` entry, checked to share no token with
/// the record's title, intent or rationale, so text matching alone cannot find the record.
fn meaning_only(fixture: &Value) -> Vec<(String, String)> {
    let entries = fixture["meaning_only"].as_array().unwrap();
    assert!(entries.len() >= 10);
    entries
        .iter()
        .map(|entry| {
            let id = entry["id"].as_str().unwrap();
            let record = fixture["records"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == id)
                .unwrap_or_else(|| panic!("unknown record {id}"));
            let text = ["title", "intent", "rationale"]
                .map(|field| record[field].as_str().unwrap().to_ascii_lowercase())
                .join(" ");
            let tokens: std::collections::BTreeSet<&str> = text
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|t| !t.is_empty())
                .collect();
            let keywords = wk_adapters::team_context::keywords(entry["query"].as_str().unwrap())
                .expect("keywords");
            let shared: Vec<&str> = keywords.split(' ').filter(|k| tokens.contains(k)).collect();
            assert!(shared.is_empty(), "{id} shares {shared:?} with its record");
            (id.to_string(), keywords)
        })
        .collect()
}

#[test]
fn meaning_only_queries_share_no_words_with_their_record() {
    let fixture = fixture();
    let cases = meaning_only(&fixture);
    let records: std::collections::BTreeSet<&String> = cases.iter().map(|(id, _)| id).collect();
    assert!(records.len() >= 8);
}

#[tokio::test]
#[ignore = "requires pgvector PostgreSQL and a real local Nomic model"]
async fn prompt_context_relevance_and_latency_with_a_real_model() {
    let Some(config) = config() else {
        eprintln!("skipped: set WK_EVAL_EMBEDDING_URL, WK_EVAL_EMBEDDING_MODEL and WK_EVAL_EMBEDDING_DIGEST to run the evaluation");
        return;
    };
    let fixture = fixture();
    let meaning_only = meaning_only(&fixture);
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();

    // Seed: alice submits each record, bob publishes it; the reader queries.
    let (mut cases, mut cards) = (Vec::new(), std::collections::BTreeMap::new());
    for record in fixture["records"].as_array().unwrap() {
        let id = record["id"].as_str().unwrap();
        let submission: wk_contracts::CardSubmission = serde_json::from_value(json!({"schema_version":1,"idempotency_key":format!("eval-{id}"),"policy_revision":1,"title":record["title"],"intent":record["intent"],"rationale":record["rationale"],"decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap();
        let card = f.k.submit(&f.alice, "demo", submission).await.unwrap();
        f.k.publish(&f.bob, "demo", card.id).await.unwrap();
        cards.insert(id.to_string(), card.id.to_string());
        if let Some(query) = record["query"].as_str() {
            let keywords = wk_adapters::team_context::keywords(query).expect("keywords");
            cases.push((id.to_string(), keywords, card.id.to_string()));
        }
    }
    let records = fixture["records"].as_array().unwrap().len();
    let worker = Embeddings::new(config.clone()).unwrap();
    let indexed = wk_server::semantic::index_once(&f.k, &worker, &f.alice.org, "demo")
        .await
        .unwrap();
    assert_eq!(indexed["indexed"], records, "{indexed}");

    let (runtime, pool, role) = common::runtime(&f).await;
    sqlx::raw_sql(
        &include_str!("../../../deploy/scripts/semantic-role.sql").replace("wk_app", &role),
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let router = |embeddings: Option<Embeddings>| {
        wk_server::portal_router_with_embeddings(
            runtime.clone(),
            vec![("reader".into(), f.reader.clone())],
            None,
            None,
            None,
            None,
            embeddings,
        )
        .unwrap()
    };
    let mut client = Embeddings::new(config).unwrap();
    if let Ok(value) = std::env::var("WK_PROMPT_SIMILARITY_FLOOR") {
        client = client.with_prompt_floor(parse_prompt_floor(&value).unwrap());
    }
    // Rounded for the report: the f32 floor widens to 0.550000011920929 in JSON.
    let floor = r3(f64::from(client.prompt_floor()));
    let hybrid = router(Some(client));
    let text = router(None);

    let cold = run(&hybrid, &cases).await;
    let warm = run(&hybrid, &cases).await;
    let lexical = run(&text, &cases).await;

    // Meaning-only prompts: no shared words, so only meaning can find the record. The first
    // hybrid pass fills the query cache; the second is measured warm.
    let meaning_cases: Vec<(String, String, String)> = meaning_only
        .into_iter()
        .map(|(id, keywords)| {
            let card = cards[&id].clone();
            (id, keywords, card)
        })
        .collect();
    run(&hybrid, &meaning_cases).await;
    let meaning_hybrid = run(&hybrid, &meaning_cases).await;
    let meaning_text = run(&text, &meaning_cases).await;
    let meaning_queries: Vec<Value> = meaning_cases
        .iter()
        .enumerate()
        .map(|(i, (id, keywords, _))| {
            json!({
                "id": id,
                "keywords": keywords,
                "hybrid_rank": meaning_hybrid[i].rank,
                "hybrid_matched_by": meaning_hybrid[i].matched_by,
                "hybrid_mode": meaning_hybrid[i].mode,
                "text_rank": meaning_text[i].rank,
                "warm_ms": round(meaning_hybrid[i].ms),
            })
        })
        .collect();

    // Leaks are meaning matches; text matches are counted for context only.
    let (mut leaks, mut text_related) = (Vec::new(), 0);
    for item in fixture["unanswerable"].as_array().unwrap() {
        let keywords =
            wk_adapters::team_context::keywords(item["query"].as_str().unwrap()).expect("keywords");
        let (result, _) = overlap(&hybrid, &keywords).await;
        for notice in related(&result) {
            if notice["matched_by"] == "meaning" || notice["matched_by"] == "both" {
                leaks.push(json!({"id":item["id"],"keywords":keywords,"card_id":notice["card_id"],"title":notice["title"],"matched_by":notice["matched_by"]}));
            } else {
                text_related += 1;
            }
        }
    }

    let queries: Vec<Value> = cases
        .iter()
        .enumerate()
        .map(|(i, (id, keywords, _))| {
            json!({
                "id": id,
                "keywords": keywords,
                "hybrid_rank": warm[i].rank,
                "hybrid_matched_by": warm[i].matched_by,
                "hybrid_mode": warm[i].mode,
                "text_rank": lexical[i].rank,
                "cold_ms": round(cold[i].ms),
                "warm_ms": round(warm[i].ms),
            })
        })
        .collect();
    // Similarity floor calibration (report only). Keywords are embedded with the indexing client
    // and compared with the stored card vectors in SQL; the sweep replays fusion (20 candidates,
    // at most 5 meaning-only additions, limit 8) against the text route's real notices.
    let names: std::collections::BTreeMap<&String, &String> =
        cards.iter().map(|(id, card)| (card, id)).collect();
    let unanswerable_cases: Vec<(String, String, String)> = fixture["unanswerable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let keywords = wk_adapters::team_context::keywords(item["query"].as_str().unwrap())
                .expect("keywords");
            (
                item["id"].as_str().unwrap().to_string(),
                keywords,
                String::new(),
            )
        })
        .collect();
    let mut probes = Vec::new();
    for (set, list) in [
        ("paraphrase", &cases),
        ("meaning_only", &meaning_cases),
        ("unanswerable", &unanswerable_cases),
    ] {
        for (id, keywords, card) in list {
            let vector = worker.embed_query(keywords).await.unwrap();
            let sims = similarities(&f, &worker.namespace, &vector).await;
            probes.push((set, id, card, text_notices(&text, keywords).await, sims));
        }
    }
    let targets: Vec<Value> = probes
        .iter()
        .filter(|(set, ..)| *set != "unanswerable")
        .map(|(set, id, card, _, sims)| {
            let rank = sims.iter().position(|(c, _)| c == *card).unwrap();
            let best_other = sims.iter().find(|(c, _)| c != *card).unwrap();
            json!({"set": set, "id": id, "similarity": r3(sims[rank].1), "rank": rank + 1, "best_other": r3(best_other.1), "best_other_id": names[&best_other.0]})
        })
        .collect();
    let closest: Vec<Value> = probes
        .iter()
        .filter(|(set, ..)| *set == "unanswerable")
        .map(|(_, id, _, _, sims)| json!({"id": id, "max_similarity": r3(sims[0].1), "closest_id": names[&sims[0].0]}))
        .collect();
    let sweep: Vec<Value> = FLOORS
        .iter()
        .map(|&floor| {
            let count = |set: &str, by_meaning: bool| {
                probes
                    .iter()
                    .filter(|(s, ..)| *s == set)
                    .filter(|(_, _, card, text, sims)| {
                        let (rank, meaning) = simulate(card, text, sims, floor);
                        rank.is_some() && (meaning || !by_meaning)
                    })
                    .count()
            };
            let leaks = probes
                .iter()
                .filter(|(s, _, _, text, sims)| {
                    *s == "unanswerable"
                        && sims.iter().any(|(c, x)| *x >= floor && !text.shown.contains(c))
                })
                .count();
            json!({"floor": floor, "meaning_only_hits5": count("meaning_only", true), "paraphrase_meaning_hits5": count("paraphrase", true), "paraphrase_hits5": count("paraphrase", false), "unanswerable_leaks": leaks})
        })
        .collect();
    let by_meaning =
        |o: &&Outcome| o.rank.is_some() && (o.matched_by == "meaning" || o.matched_by == "both");
    let calibration = json!({
        "method": "fusion replayed on card IDs: text-route notices plus meaning candidates with similarity >= floor (<= 20, excluding non-related notices), reciprocal rank k=61, <= 5 meaning-only added, limit 8, hit = target among first 5 related; unanswerable leak = any visible card with similarity >= floor",
        "router_observed": {
            "floor": floor,
            "meaning_only_hits5": meaning_hybrid.iter().filter(by_meaning).count(),
            "paraphrase_meaning_hits5": warm.iter().filter(by_meaning).count(),
        },
        "sweep": sweep,
        "targets": targets,
        "unanswerable": closest,
    });

    let mut report = json!({
        "corpus": fixture["name"],
        "prompt_floor": floor,
        "records": records,
        "paraphrases": cases.len(),
        "top5_hits": {"hybrid": hits(&warm), "hybrid_cold": hits(&cold), "text": hits(&lexical)},
        "mrr_at_5": {"hybrid": (mrr(&warm) * 1000.0).round() / 1000.0, "text": (mrr(&lexical) * 1000.0).round() / 1000.0},
        "unanswerable": fixture["unanswerable"].as_array().unwrap().len(),
        "unanswerable_leaks": leaks.len(),
        "leaks": leaks,
        "unanswerable_text_related": text_related,
        "latency_ms": {
            "cold": {"p50": round(percentile(&cold, 50.0)), "p95": round(percentile(&cold, 95.0))},
            "warm": {"p50": round(percentile(&warm, 50.0)), "p95": round(percentile(&warm, 95.0))},
            "text": {"p50": round(percentile(&lexical, 50.0)), "p95": round(percentile(&lexical, 95.0))},
        },
        "queries": queries,
        "meaning_only": {
            "queries": meaning_cases.len(),
            "hybrid_top5": hits(&meaning_hybrid),
            "text_top5": hits(&meaning_text),
            "hybrid_mrr_at_5": (mrr(&meaning_hybrid) * 1000.0).round() / 1000.0,
            "per_query": meaning_queries,
        },
    });
    report["calibration"] = calibration;
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    drop((hybrid, text));
    common::cleanup_runtime(&f, pool, role).await;

    assert!(hits(&warm) >= 11, "hybrid top-5 hits below 11 of 12");
    assert!(
        hits(&meaning_hybrid) >= 9,
        "meaning-only hybrid top-5 hits below 9 of {} at floor {floor}",
        meaning_cases.len()
    );
    assert!(
        report["unanswerable_leaks"] == 0,
        "unanswerable leaks at floor {floor}"
    );
    assert!(
        percentile(&warm, 95.0) < 100.0,
        "warm p95 is not below 100 ms"
    );
}
