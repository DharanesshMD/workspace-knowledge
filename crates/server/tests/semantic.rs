#[path = "../../knowledge/tests/common/mod.rs"]
mod common;
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;
use wk_knowledge::semantic::Namespace;
use wk_server::embeddings::{Config, Embeddings};
fn ns() -> Namespace {
    Namespace {
        model: "nomic-embed-text:v1.5".into(),
        digest: "a".repeat(64),
        dimensions: 3,
    }
}
fn card() -> wk_contracts::CardSubmission {
    serde_json::from_value(json!({"schema_version":1,"idempotency_key":"synthetic-index","policy_revision":1,"title":"Idempotency key","intent":"Bound retries","rationale":"Protect payment operations","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()
}
#[derive(Clone)]
struct Model {
    requests: Arc<AtomicUsize>,
    revocation: Option<(wk_knowledge::Knowledge, wk_knowledge::Actor)>,
}
async fn model(
    revocation: Option<(wk_knowledge::Knowledge, wk_knowledge::Actor)>,
) -> (Config, tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
    let requests = Arc::new(AtomicUsize::new(0));
    let state = Model {
        requests: requests.clone(),
        revocation,
    };
    let app = Router::new()
        .route(
            "/api/tags",
            get(|State(s): State<Model>| async move {
                s.requests.fetch_add(1, Ordering::SeqCst);
                Json(json!({"models":[{"name":ns().model,"digest":ns().digest}]}))
            }),
        )
        .route(
            "/api/embed",
            post(
                |State(s): State<Model>, Json(input): Json<Value>| async move {
                    s.requests.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(input["truncate"], false);
                    if let Some((k, a)) = s.revocation {
                        k.revoke(&a, "demo").await.unwrap();
                    }
                    Json(json!({"model":ns().model,"embeddings":[[1.,0.,0.]]}))
                },
            ),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/api/embed", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        Config {
            endpoint,
            namespace: ns(),
        },
        task,
        requests,
    )
}
async fn send(app: &Router, query: &str, token: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/search")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    json!({"product":"demo","query":query,"limit":5}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn http_hybrid_search_and_provider_outage_preserve_authorization_and_explicit_diagnostics() {
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();
    let c = f.k.submit(&f.alice, "demo", card()).await.unwrap();
    f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    let (config, task, requests) = model(None).await;
    let worker = Embeddings::new(config.clone()).unwrap();
    let result = wk_server::semantic::index_once(&f.k, &worker, &f.alice.org, "demo")
        .await
        .unwrap();
    assert_eq!(result["indexed"], 1);
    let (runtime, pool, role) = common::runtime(&f).await;
    sqlx::raw_sql(
        &include_str!("../../../deploy/scripts/semantic-role.sql").replace("wk_app", &role),
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let app = wk_server::portal_router_with_embeddings(
        runtime,
        vec![("reader".into(), f.reader.clone())],
        None,
        None,
        None,
        None,
        Some(Embeddings::new(config).unwrap()),
    )
    .unwrap();
    let prior = requests.load(Ordering::SeqCst);
    assert_eq!(
        send(&app, "password=PRIVATE_SYNTHETIC", "reader").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        send(&app, "idempotency", "invalid").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(requests.load(Ordering::SeqCst), prior);
    let (status, result) = send(&app, "duplicate money", "reader").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["cards"][0]["id"], c.id.to_string());
    assert_eq!(result["retrieval"]["mode"], "hybrid");
    for (id, name, args) in [
        (
            1,
            "knowledge.search",
            json!({"query":"duplicate money","limit":5}),
        ),
        (
            2,
            "knowledge.context",
            json!({"query":"duplicate money","budget_tokens":2000}),
        ),
    ] {
        let response=app.clone().oneshot(Request::builder().method("POST").uri("http://localhost/mcp/demo").header("authorization","Bearer reader").header("content-type","application/json").header("accept","application/json, text/event-stream").header("mcp-protocol-version","2025-06-18").body(Body::from(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let result: Value = serde_json::from_slice(&bytes).unwrap();
        let data: Value =
            serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(data["cards"][0]["id"], c.id.to_string());
        assert_eq!(data["retrieval"]["mode"], "hybrid");
    }
    task.abort();
    let (_, result) = send(&app, "idempotency", "reader").await;
    assert_eq!(result["cards"][0]["id"], c.id.to_string());
    assert_eq!(result["retrieval"]["mode"], "lexical");
    assert_eq!(result["retrieval"]["reason"], "embedding_unavailable");
    common::cleanup_runtime(&f, pool, role).await;
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn revocation_during_embedding_does_not_return_previously_authorized_context() {
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();
    let c = f.k.submit(&f.alice, "demo", card()).await.unwrap();
    f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    let source =
        f.k.semantic_pending(&f.alice.org, "demo", &ns())
            .await
            .unwrap()
            .remove(0);
    f.k.semantic_store(&f.alice.org, "demo", &ns(), &source, &[1., 0., 0.])
        .await
        .unwrap();
    let (config, task, _) = model(Some((f.k.clone(), f.reader.clone()))).await;
    let app = wk_server::portal_router_with_embeddings(
        f.k.clone(),
        vec![("reader".into(), f.reader.clone())],
        None,
        None,
        None,
        None,
        Some(Embeddings::new(config).unwrap()),
    )
    .unwrap();
    let (status, result) = send(&app, "duplicate money", "reader").await;
    assert!(matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::FORBIDDEN
    ));
    assert!(result["cards"].is_null());
    task.abort();
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn missing_current_index_falls_back_without_sending_query_to_provider() {
    let f = common::setup().await;
    let c = f.k.submit(&f.alice, "demo", card()).await.unwrap();
    f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    let (config, task, requests) = model(None).await;
    let app = wk_server::portal_router_with_embeddings(
        f.k.clone(),
        vec![("reader".into(), f.reader.clone())],
        None,
        None,
        None,
        None,
        Some(Embeddings::new(config).unwrap()),
    )
    .unwrap();
    let (_, result) = send(&app, "idempotency", "reader").await;
    assert_eq!(result["cards"][0]["id"], c.id.to_string());
    assert_eq!(result["retrieval"]["reason"], "index_not_ready");
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    task.abort();
}
