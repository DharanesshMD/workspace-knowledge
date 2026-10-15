//! Opt-in relevance and latency evaluation of prompt-time overlap notices against a real local
//! Nomic model. Set `WK_EVAL_EMBEDDING_URL`, `WK_EVAL_EMBEDDING_MODEL` and
//! `WK_EVAL_EMBEDDING_DIGEST` (768 dimensions) plus `WK_TEST_DATABASE_URL` to run it.
#[path = "../../knowledge/tests/common/mod.rs"]
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::time::Instant;
use tower::ServiceExt;
use wk_knowledge::semantic::Namespace;
use wk_server::embeddings::{Config, Embeddings};

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

#[tokio::test]
#[ignore = "requires pgvector PostgreSQL and a real local Nomic model"]
async fn prompt_context_relevance_and_latency_with_a_real_model() {
    let Some(config) = config() else {
        eprintln!("skipped: set WK_EVAL_EMBEDDING_URL, WK_EVAL_EMBEDDING_MODEL and WK_EVAL_EMBEDDING_DIGEST to run the evaluation");
        return;
    };
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/semantic-search.json"
    ))
    .unwrap();
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();

    // Seed: alice submits each record, bob publishes it; the reader queries.
    let mut cases = Vec::new();
    for record in fixture["records"].as_array().unwrap() {
        let id = record["id"].as_str().unwrap();
        let submission: wk_contracts::CardSubmission = serde_json::from_value(json!({"schema_version":1,"idempotency_key":format!("eval-{id}"),"policy_revision":1,"title":record["title"],"intent":record["intent"],"rationale":record["rationale"],"decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap();
        let card = f.k.submit(&f.alice, "demo", submission).await.unwrap();
        f.k.publish(&f.bob, "demo", card.id).await.unwrap();
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
    let hybrid = router(Some(Embeddings::new(config).unwrap()));
    let text = router(None);

    let cold = run(&hybrid, &cases).await;
    let warm = run(&hybrid, &cases).await;
    let lexical = run(&text, &cases).await;

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
    let report = json!({
        "corpus": fixture["name"],
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
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    drop((hybrid, text));
    common::cleanup_runtime(&f, pool, role).await;

    assert!(hits(&warm) >= 11, "hybrid top-5 hits below 11 of 12");
    assert!(report["unanswerable_leaks"] == 0, "unanswerable leaks");
    assert!(
        percentile(&warm, 95.0) < 100.0,
        "warm p95 is not below 100 ms"
    );
}
