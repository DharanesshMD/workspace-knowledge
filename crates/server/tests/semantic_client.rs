use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use wk_knowledge::semantic::Namespace;
use wk_server::embeddings::{Config, Embeddings};
const MODEL: &str = "nomic-embed-text:v1.5";
#[derive(Clone)]
struct Model {
    mode: &'static str,
    embeds: Arc<AtomicUsize>,
}
fn config(url: String) -> Config {
    Config {
        endpoint: url,
        namespace: Namespace {
            model: MODEL.into(),
            digest: "a".repeat(64),
            dimensions: 3,
        },
    }
}
async fn fixture(
    mode: &'static str,
) -> (Embeddings, tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
    let model = Model {
        mode,
        embeds: Arc::new(AtomicUsize::new(0)),
    };
    let calls = model.embeds.clone();
    let app=Router::new().route("/api/tags",get(|State(m):State<Model>|async move {let digest=if m.mode=="changed" && m.embeds.load(Ordering::SeqCst)>0 {"b"}else{"a"};Json(json!({"models":[{"name":MODEL,"digest":digest.repeat(64)}]}))}))
        .route("/api/embed",post(|State(m):State<Model>,headers:HeaderMap,Json(body):Json<Value>|async move {
            assert!(!headers.contains_key("authorization"));assert_eq!(body["truncate"],false);assert_eq!(body["model"],MODEL);
            assert!(body["input"].as_str().unwrap().starts_with("search_query: ") || body["input"].as_str().unwrap().starts_with("search_document: "));
            m.embeds.fetch_add(1,Ordering::SeqCst);
            if m.mode=="slow" {tokio::time::sleep(std::time::Duration::from_secs(4)).await;}
            match m.mode {
                "redirect"=>(StatusCode::TEMPORARY_REDIRECT,[("location","http://127.0.0.1:1/api/embed")]).into_response(),
                "oversized"=>Json(json!({"model":MODEL,"embeddings":[[1.,0.,0.]],"padding":"x".repeat(1024*1024)})).into_response(),
                "wrong_model"=>Json(json!({"model":"other","embeddings":[[1.,0.,0.]]})).into_response(),
                "wrong_dimensions"=>Json(json!({"model":MODEL,"embeddings":[[1.,0.]]})).into_response(),
                "zero"=>Json(json!({"model":MODEL,"embeddings":[[0.,0.,0.]]})).into_response(),
                "overflow"=>Json(json!({"model":MODEL,"embeddings":[[f32::MAX,1.,1.]]})).into_response(),
                "extra"=>Json(json!({"model":MODEL,"embeddings":[[1.,0.,0.],[1.,0.,0.]]})).into_response(),
                _=>Json(json!({"model":MODEL,"embeddings":[[1.,0.,0.]]})).into_response()
            }
        })).with_state(model);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        Embeddings::new(config(format!("http://{addr}/api/embed"))).unwrap(),
        task,
        calls,
    )
}
#[test]
fn embedding_configuration_requires_literal_loopback_and_complete_model_pin() {
    for url in [
        "https://example.org/api/embed",
        "http://localhost:11434/api/embed",
        "http://127.0.0.1:11434/api/embed?x=1",
        "http://u:p@127.0.0.1/api/embed",
        "http://127.0.0.1/api/embed#x",
        "http://127.0.0.1/api/embeddings",
    ] {
        assert!(Embeddings::new(config(url.into())).is_err());
    }
    assert!(Config::parse(|_| None).unwrap().is_none());
    assert!(Config::parse(|name| if name == "WK_EMBEDDING_URL" {
        Some("http://127.0.0.1/api/embed".into())
    } else {
        None
    })
    .is_err());
}
#[tokio::test]
async fn valid_pinned_model_embeds_only_bounded_query_or_document_inputs() {
    let (client, task, calls) = fixture("valid").await;
    assert_eq!(
        client
            .embed_query("org", "demo", "idempotency")
            .await
            .unwrap(),
        vec![1., 0., 0.]
    );
    assert_eq!(
        client
            .embed_document("Retries require an idempotency key")
            .await
            .unwrap(),
        vec![1., 0., 0.]
    );
    assert!(client
        .embed_query("org", "demo", &"x".repeat(1025))
        .await
        .is_err());
    assert!(client
        .embed_query("org", "demo", "password=PRIVATE_SYNTHETIC")
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    task.abort();
}
#[tokio::test]
async fn untrusted_model_changes_and_invalid_embedding_responses_are_rejected() {
    for mode in [
        "changed",
        "redirect",
        "oversized",
        "wrong_model",
        "wrong_dimensions",
        "zero",
        "overflow",
        "extra",
    ] {
        let (client, task, _) = fixture(mode).await;
        assert!(
            client
                .embed_query("org", "demo", "idempotency")
                .await
                .is_err(),
            "accepted {mode}"
        );
        task.abort();
    }
}
#[tokio::test]
async fn stalled_embedding_does_not_block_keyword_fallback_deadline() {
    let (client, task, _) = fixture("slow").await;
    let now = std::time::Instant::now();
    assert!(client
        .embed_query("org", "demo", "idempotency")
        .await
        .is_err());
    assert!(now.elapsed() < std::time::Duration::from_millis(3800));
    task.abort();
}
