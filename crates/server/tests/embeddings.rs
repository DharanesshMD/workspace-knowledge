use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use wk_knowledge::semantic::Namespace;
use wk_server::embeddings::{parse_keep_alive, Config, Embeddings};

fn ns() -> Namespace {
    Namespace {
        model: "nomic-embed-text:v1.5".into(),
        digest: "a".repeat(64),
        dimensions: 3,
    }
}
#[derive(Clone)]
struct Seen {
    embeds: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
    digest: Arc<Mutex<String>>,
    failing: Arc<AtomicBool>,
}
async fn model() -> (Config, Seen) {
    let seen = Seen {
        embeds: Arc::default(),
        bodies: Arc::default(),
        digest: Arc::new(Mutex::new(ns().digest)),
        failing: Arc::default(),
    };
    let app = Router::new()
        .route(
            "/api/tags",
            get(|State(s): State<Seen>| async move {
                let digest = s.digest.lock().unwrap().clone();
                Json(json!({"models":[{"name":ns().model,"digest":digest}]}))
            }),
        )
        .route(
            "/api/embed",
            post(
                |State(s): State<Seen>, Json(body): Json<Value>| async move {
                    s.embeds.fetch_add(1, Ordering::SeqCst);
                    s.bodies.lock().unwrap().push(body);
                    if s.failing.load(Ordering::SeqCst) {
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                    Json(json!({"model":ns().model,"embeddings":[[1.,0.,0.]]})).into_response()
                },
            ),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/api/embed", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        Config {
            endpoint,
            namespace: ns(),
        },
        seen,
    )
}
#[tokio::test]
async fn repeated_queries_are_served_from_the_cache() {
    let (config, seen) = model().await;
    let e = Embeddings::new(config).unwrap();
    let a = e
        .embed_query_within(
            "retries avoid repeating payment",
            Duration::from_millis(1500),
        )
        .await
        .unwrap();
    let b = e
        .embed_query_within(
            "retries  avoid repeating payment",
            Duration::from_millis(1500),
        )
        .await
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(
        seen.embeds.load(Ordering::SeqCst),
        1,
        "whitespace-normalized repeat is cached"
    );
    e.embed_query("different words").await.unwrap();
    assert_eq!(seen.embeds.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn documents_and_rejected_input_bypass_the_cache() {
    let (config, seen) = model().await;
    let e = Embeddings::new(config).unwrap();
    e.embed_query("idempotency key").await.unwrap();
    e.embed_document("idempotency key").await.unwrap();
    e.embed_document("idempotency key").await.unwrap();
    assert_eq!(seen.embeds.load(Ordering::SeqCst), 3);
    assert_eq!(
        e.embed_query("password=PRIVATE_SYNTHETIC").await,
        Err("input_rejected")
    );
    assert_eq!(
        seen.embeds.load(Ordering::SeqCst),
        3,
        "rejected input makes no request"
    );
}
#[tokio::test]
async fn requests_keep_the_model_loaded_and_warm_up_embeds_once() {
    let (config, seen) = model().await;
    let e = Embeddings::new(config.clone()).unwrap();
    e.warm().await.unwrap();
    assert_eq!(seen.embeds.load(Ordering::SeqCst), 1);
    assert_eq!(seen.bodies.lock().unwrap()[0]["keep_alive"], "30m");
    e.embed_query("warm up").await.unwrap();
    assert_eq!(
        seen.embeds.load(Ordering::SeqCst),
        2,
        "warm-up is not cached"
    );
    let custom = Embeddings::new(config)
        .unwrap()
        .with_keep_alive("2h".into());
    custom.embed_query("rate limiter").await.unwrap();
    assert_eq!(seen.bodies.lock().unwrap()[2]["keep_alive"], "2h");
}
#[tokio::test]
async fn a_detected_model_change_clears_the_query_cache() {
    let (config, seen) = model().await;
    let e = Embeddings::new(config).unwrap();
    e.embed_query("payment retries").await.unwrap();
    *seen.digest.lock().unwrap() = "b".repeat(64);
    assert_eq!(
        e.embed_query("different words").await,
        Err("embedding_model_changed")
    );
    assert_eq!(
        e.embed_query("payment retries").await,
        Err("embedding_model_changed"),
        "cached vector from the old model is no longer served"
    );
}
#[tokio::test]
async fn failed_embeddings_are_not_cached() {
    let (config, seen) = model().await;
    let e = Embeddings::new(config).unwrap();
    seen.failing.store(true, Ordering::SeqCst);
    assert!(e.embed_query("rate limiter").await.is_err());
    assert_eq!(seen.embeds.load(Ordering::SeqCst), 1);
    seen.failing.store(false, Ordering::SeqCst);
    e.embed_query("rate limiter").await.unwrap();
    assert_eq!(
        seen.embeds.load(Ordering::SeqCst),
        2,
        "recovered model is reached"
    );
    e.embed_query("rate limiter").await.unwrap();
    assert_eq!(seen.embeds.load(Ordering::SeqCst), 2, "success is cached");
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
