use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
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
#[derive(Clone, Default)]
struct Seen {
    embeds: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
}
async fn model() -> (Config, Seen) {
    let seen = Seen::default();
    let app = Router::new()
        .route(
            "/api/tags",
            get(|| async { Json(json!({"models":[{"name":ns().model,"digest":ns().digest}]})) }),
        )
        .route(
            "/api/embed",
            post(
                |State(s): State<Seen>, Json(body): Json<Value>| async move {
                    s.embeds.fetch_add(1, Ordering::SeqCst);
                    s.bodies.lock().unwrap().push(body);
                    Json(json!({"model":ns().model,"embeddings":[[1.,0.,0.]]}))
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
    e.embed_document("idempotency key").await.unwrap();
    e.embed_document("idempotency key").await.unwrap();
    assert_eq!(seen.embeds.load(Ordering::SeqCst), 2);
    assert_eq!(
        e.embed_query("password=PRIVATE_SYNTHETIC").await,
        Err("input_rejected")
    );
}
#[tokio::test]
async fn requests_keep_the_model_loaded_and_warm_up_embeds_once() {
    let (config, seen) = model().await;
    let e = Embeddings::new(config.clone()).unwrap();
    e.warm().await.unwrap();
    assert_eq!(seen.bodies.lock().unwrap()[0]["keep_alive"], "30m");
    let custom = Embeddings::new(config)
        .unwrap()
        .with_keep_alive("2h".into());
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
