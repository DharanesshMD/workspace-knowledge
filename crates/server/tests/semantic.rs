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
    embeds: Arc<AtomicUsize>,
    delay: std::time::Duration,
    revocation: Option<(wk_knowledge::Knowledge, wk_knowledge::Actor)>,
}
async fn model(
    revocation: Option<(wk_knowledge::Knowledge, wk_knowledge::Actor)>,
) -> (Config, tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
    let (config, task, requests, _) = counted_model(revocation).await;
    (config, task, requests)
}
/// Also returns the `/api/embed` call count alone (excluding `/api/tags` digest checks).
async fn counted_model(
    revocation: Option<(wk_knowledge::Knowledge, wk_knowledge::Actor)>,
) -> (
    Config,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    delayed_model(revocation, std::time::Duration::ZERO).await
}
/// `/api/embed` answers only after `delay`.
async fn delayed_model(
    revocation: Option<(wk_knowledge::Knowledge, wk_knowledge::Actor)>,
    delay: std::time::Duration,
) -> (
    Config,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let requests = Arc::new(AtomicUsize::new(0));
    let embeds = Arc::new(AtomicUsize::new(0));
    let state = Model {
        requests: requests.clone(),
        embeds: embeds.clone(),
        delay,
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
                    s.embeds.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(s.delay).await;
                    assert_eq!(input["truncate"], false);
                    if let Some((k, a)) = s.revocation {
                        if a.user == "admin" {
                            k.admin_policy_put(
                                &a,
                                "demo",
                                wk_knowledge::admin::ProductPolicy {
                                    allow_loopback: false,
                                    ..Default::default()
                                },
                            )
                            .await
                            .unwrap();
                        } else {
                            k.revoke(&a, "demo").await.unwrap();
                        }
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
        embeds,
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

#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn disabled_inference_policy_prevents_search_and_index_provider_calls() {
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
    f.k.admin_policy_put(
        &f.admin,
        "demo",
        wk_knowledge::admin::ProductPolicy {
            allow_loopback: false,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (config, task, requests) = model(None).await;
    let app = wk_server::portal_router_with_embeddings(
        f.k.clone(),
        vec![("reader".into(), f.reader.clone())],
        None,
        None,
        None,
        None,
        Some(Embeddings::new(config.clone()).unwrap()),
    )
    .unwrap();
    let (_, result) = send(&app, "idempotency", "reader").await;
    assert_eq!(result["retrieval"]["reason"], "embedding_policy_disabled");
    assert_eq!(result["cards"][0]["id"], c.id.to_string());
    let response=app.clone().oneshot(Request::builder().method("POST").uri("http://localhost/mcp/demo").header("authorization","Bearer reader").header("content-type","application/json").header("accept","application/json, text/event-stream").header("mcp-protocol-version","2025-06-18").body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge.context","arguments":{"query":"idempotency","budget_tokens":2000}}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let wire: Value = serde_json::from_slice(&bytes).unwrap();
    let context: Value =
        serde_json::from_str(wire["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(context["retrieval"]["reason"], "embedding_policy_disabled");
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert!(wk_server::semantic::index_once(
        &f.k,
        &Embeddings::new(config).unwrap(),
        &f.alice.org,
        "demo"
    )
    .await
    .is_err());
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    task.abort();
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn inference_policy_change_discards_results_and_stops_index_batch() {
    for indexing in [true, false] {
        let f = common::setup().await;
        f.k.semantic_migrate().await.unwrap();
        for i in 0..2 {
            let mut body = card();
            body.idempotency_key = format!("policy-card-{i}");
            let c = f.k.submit(&f.alice, "demo", body).await.unwrap();
            f.k.publish(&f.bob, "demo", c.id).await.unwrap();
        }
        if !indexing {
            for source in
                f.k.semantic_pending(&f.alice.org, "demo", &ns())
                    .await
                    .unwrap()
            {
                f.k.semantic_store(&f.alice.org, "demo", &ns(), &source, &[1., 0., 0.])
                    .await
                    .unwrap();
            }
        }
        let (config, task, requests) = model(Some((f.k.clone(), f.admin.clone()))).await;
        if indexing {
            assert!(wk_server::semantic::index_once(
                &f.k,
                &Embeddings::new(config).unwrap(),
                &f.alice.org,
                "demo"
            )
            .await
            .is_err());
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM card_embeddings WHERE org=$1 AND status='ready'",
            )
            .bind(&f.alice.org)
            .fetch_one(&f.pool)
            .await
            .unwrap();
            assert_eq!(count, 0);
        } else {
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
            assert_eq!(result["retrieval"]["mode"], "lexical");
            assert_eq!(result["retrieval"]["reason"], "embedding_policy_disabled");
        }
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        task.abort();
    }
}
async fn overlap(app: &Router, token: &str, body: Value) -> (StatusCode, Value) {
    overlap_in(app, token, "demo", body).await
}
async fn overlap_in(app: &Router, token: &str, product: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/products/{product}/overlaps"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn mcp_tool(app: &Router, name: &str, args: Value) -> Value {
    let response=app.clone().oneshot(Request::builder().method("POST").uri("http://localhost/mcp/demo").header("authorization","Bearer reader").header("content-type","application/json").header("accept","application/json, text/event-stream").header("mcp-protocol-version","2025-06-18").body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let wire: Value = serde_json::from_slice(&bytes).unwrap();
    serde_json::from_str(wire["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
const COLLECTOR_ORIGIN: &str = "http://127.0.0.1:4318";
async fn signed(
    app: &Router,
    path: &str,
    key: &ed25519_dalek::SigningKey,
    id: uuid::Uuid,
    body: Value,
) -> (StatusCode, Value) {
    use ed25519_dalek::Signer;
    let (raw, nonce) = (serde_json::to_vec(&body).unwrap(), uuid::Uuid::new_v4());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let signature = key.sign(&wk_contracts::device::signing_bytes(
        COLLECTOR_ORIGIN,
        "POST",
        path,
        now,
        nonce,
        id,
        "collector",
        &raw,
    ));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .header("authorization", "Bearer collector")
                .header("x-wk-device", id.to_string())
                .header("x-wk-time", now.to_string())
                .header("x-wk-nonce", nonce.to_string())
                .header("x-wk-signature", hex::encode(signature.to_bytes()))
                .body(Body::from(raw))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn related<'a>(result: &'a Value, id: &str) -> Vec<&'a Value> {
    result["notices"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["card_id"] == id)
        .collect()
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn prompt_overlaps_add_meaning_matches_through_http_mcp_and_device_routes() {
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();
    let c = f.k.submit(&f.alice, "demo", card()).await.unwrap();
    f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    let id = c.id.to_string();
    let (config, task, requests, embeds) = counted_model(None).await;
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
        Some(COLLECTOR_ORIGIN.into()),
        None,
        Some(wk_server::device_routes::CollectorAuth {
            origin: COLLECTOR_ORIGIN.into(),
            oidc: None,
            tokens: vec![("collector".into(), f.reader.clone())],
            client_id: None,
        }),
        Some(Embeddings::new(config).unwrap()),
    )
    .unwrap();

    // Meaning only: no shared words with the card.
    let (status, result) =
        overlap(&app, "reader", json!({"query":"duplicate money","limit":8})).await;
    assert_eq!(status, StatusCode::OK);
    let found = related(&result, &id);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["kind"], "related");
    assert_eq!(found[0]["matched_by"], "meaning");
    assert_eq!(result["retrieval"], json!({"mode":"hybrid","reason":null}));
    assert_eq!(result["query_ignored"], false);
    assert!(result["text"].as_str().unwrap().contains("Idempotency key"));

    // Found by text and meaning: listed once as `both`.
    let (_, result) = overlap(&app, "reader", json!({"query":"retries payment","limit":8})).await;
    let found = related(&result, &id);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["matched_by"], "both");
    assert_eq!(result["retrieval"]["mode"], "hybrid");

    // A repeated prompt is served from the query cache.
    let embedded = embeds.load(Ordering::SeqCst);
    let (_, result) = overlap(
        &app,
        "reader",
        json!({"query":"retries  payment","limit":8}),
    )
    .await;
    assert_eq!(related(&result, &id)[0]["matched_by"], "both");
    assert_eq!(embeds.load(Ordering::SeqCst), embedded);

    // No query text, or sensitive query text: never sent to the provider.
    let prior = requests.load(Ordering::SeqCst);
    let (status, result) =
        overlap(&app, "reader", json!({"files":["backend/x.rs"],"limit":8})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["retrieval"], json!({"mode":"lexical","reason":null}));
    let (status, result) = overlap(
        &app,
        "reader",
        json!({"query":"password=PRIVATE_SYNTHETIC"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["query_ignored"], true);
    assert_eq!(result["retrieval"], json!({"mode":"lexical","reason":null}));
    let (status, result) = overlap(
        &app,
        "reader",
        json!({"files":["backend/x.rs"],"query":"password=PRIVATE_SYNTHETIC"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["query_ignored"], true);
    assert!(!result.to_string().contains("PRIVATE_SYNTHETIC"));
    assert_eq!(requests.load(Ordering::SeqCst), prior);

    // Unauthenticated callers never reach the provider.
    assert_eq!(
        overlap(&app, "invalid", json!({"query":"duplicate funds"}))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(requests.load(Ordering::SeqCst), prior);

    // MCP shares the lookup.
    let data = mcp_tool(
        &app,
        "knowledge.overlaps",
        json!({"query":"duplicate money"}),
    )
    .await;
    assert_eq!(related(&data, &id)[0]["matched_by"], "meaning");
    assert_eq!(data["retrieval"]["mode"], "hybrid");

    // The device-signed collector operation shares the lookup.
    let key = ed25519_dalek::SigningKey::from_bytes(&[42; 32]);
    let device = uuid::Uuid::new_v4();
    let approval = "22".repeat(32);
    let (status, _) = signed(
        &app,
        "/collector/register",
        &key,
        device,
        json!({"id":device,"public_key":hex::encode(key.verifying_key().to_bytes()),"product":"demo","services":["backend"],"approval_hash":approval}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, data) = signed(
        &app,
        "/collector/overlaps",
        &key,
        device,
        json!({"product":"demo","approval_hash":approval,"request":{"query":"duplicate money","limit":8}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(related(&data, &id)[0]["matched_by"], "meaning");
    assert_eq!(data["retrieval"]["mode"], "hybrid");

    // Provider outage: text matches still arrive, with a fixed reason. The query differs from
    // earlier ones so the cache cannot answer it.
    task.abort();
    let _ = task.await;
    let (status, result) =
        overlap(&app, "reader", json!({"query":"payment retries","limit":8})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(related(&result, &id)[0]["matched_by"], "text");
    assert_eq!(
        result["retrieval"],
        json!({"mode":"lexical","reason":"embedding_unavailable"})
    );
    common::cleanup_runtime(&f, pool, role).await;
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn prompt_overlaps_fall_back_to_text_without_policy_or_index_and_skip_the_provider() {
    for disabled in [true, false] {
        let f = common::setup().await;
        f.k.semantic_migrate().await.unwrap();
        let c = f.k.submit(&f.alice, "demo", card()).await.unwrap();
        f.k.publish(&f.bob, "demo", c.id).await.unwrap();
        if disabled {
            let source =
                f.k.semantic_pending(&f.alice.org, "demo", &ns())
                    .await
                    .unwrap()
                    .remove(0);
            f.k.semantic_store(&f.alice.org, "demo", &ns(), &source, &[1., 0., 0.])
                .await
                .unwrap();
            f.k.admin_policy_put(
                &f.admin,
                "demo",
                wk_knowledge::admin::ProductPolicy {
                    allow_loopback: false,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
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
        let (status, result) =
            overlap(&app, "reader", json!({"query":"retries payment","limit":8})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(related(&result, &c.id.to_string())[0]["matched_by"], "text");
        let reason = if disabled {
            "embedding_policy_disabled"
        } else {
            "index_not_ready"
        };
        assert_eq!(
            result["retrieval"],
            json!({"mode":"lexical","reason":reason})
        );
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        task.abort();
    }
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn prompt_overlaps_without_a_model_report_semantic_disabled() {
    let f = common::setup().await;
    let c = f.k.submit(&f.alice, "demo", card()).await.unwrap();
    f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    let app = wk_server::portal_router_with_embeddings(
        f.k.clone(),
        vec![("reader".into(), f.reader.clone())],
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, result) = overlap(&app, "reader", json!({"query":"retries payment"})).await;
    assert_eq!(related(&result, &c.id.to_string())[0]["matched_by"], "text");
    assert_eq!(
        result["retrieval"],
        json!({"mode":"lexical","reason":"semantic_disabled"})
    );
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn invalid_prompt_overlaps_are_rejected_before_the_provider() {
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
    let (config, task, _, embeds) = counted_model(None).await;
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
    for body in [
        json!({"files":["/etc/passwd"],"query":"duplicate money","limit":8}),
        json!({"query":"duplicate money","limit":0}),
        json!({"query":"duplicate money","limit":21}),
    ] {
        let (status, result) = overlap(&app, "reader", body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(result, json!({"error":"invalid_request"}));
    }
    assert_eq!(embeds.load(Ordering::SeqCst), 0);
    // The same request with valid fields does reach the model.
    let (status, result) =
        overlap(&app, "reader", json!({"query":"duplicate money","limit":8})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["retrieval"]["mode"], "hybrid");
    assert_eq!(embeds.load(Ordering::SeqCst), 1);
    task.abort();
}

/// Publishes `body` (authored by alice) and stores its embedding as `[1,0,0]`.
async fn indexed(f: &common::Fixture, body: wk_contracts::CardSubmission) -> String {
    let c = f.k.submit(&f.alice, "demo", body).await.unwrap();
    f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    for source in
        f.k.semantic_pending(&f.alice.org, "demo", &ns())
            .await
            .unwrap()
    {
        f.k.semantic_store(&f.alice.org, "demo", &ns(), &source, &[1., 0., 0.])
            .await
            .unwrap();
    }
    c.id.to_string()
}
fn app_for(
    f: &common::Fixture,
    tokens: Vec<(String, wk_knowledge::Actor)>,
    collector: Option<wk_server::device_routes::CollectorAuth>,
    config: Config,
) -> Router {
    wk_server::portal_router_with_embeddings(
        f.k.clone(),
        tokens,
        None,
        Some(COLLECTOR_ORIGIN.into()),
        None,
        collector,
        Some(Embeddings::new(config).unwrap()),
    )
    .unwrap()
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn policy_disabled_during_inference_keeps_text_notices_only() {
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();
    let id = indexed(&f, card()).await;
    let (config, task, _, embeds) = counted_model(Some((f.k.clone(), f.admin.clone()))).await;
    let app = app_for(&f, vec![("reader".into(), f.reader.clone())], None, config);
    let (status, result) =
        overlap(&app, "reader", json!({"query":"retries payment","limit":8})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        embeds.load(Ordering::SeqCst),
        1,
        "policy flipped mid-inference"
    );
    assert_eq!(
        result["retrieval"],
        json!({"mode":"lexical","reason":"embedding_policy_disabled"})
    );
    let found = related(&result, &id);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["matched_by"], "text");
    assert!(!result.to_string().contains("\"meaning\""));
    assert!(!result.to_string().contains("\"both\""));
    task.abort();
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn non_members_and_other_products_never_reach_the_provider() {
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();
    indexed(&f, card()).await;
    let (config, task, _, embeds) = counted_model(None).await;
    let stranger = wk_knowledge::Actor {
        org: f.alice.org.clone(),
        user: "stranger".into(),
    };
    let app = app_for(
        &f,
        vec![
            ("reader".into(), f.reader.clone()),
            ("stranger".into(), stranger),
        ],
        None,
        config,
    );
    let body = json!({"query":"duplicate money","limit":8});
    assert_eq!(
        overlap(&app, "stranger", body.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        overlap_in(&app, "reader", "other", body).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(embeds.load(Ordering::SeqCst), 0);
    task.abort();
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn a_device_never_sees_meaning_matches_outside_its_services() {
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();
    let backend = indexed(&f, card()).await;
    let mut body = card();
    body.idempotency_key = "synthetic-frontend".into();
    body.service_ids = vec!["frontend".into()];
    let frontend = indexed(&f, body).await;
    let (config, task, _, _) = counted_model(None).await;
    let app = app_for(
        &f,
        vec![],
        Some(wk_server::device_routes::CollectorAuth {
            origin: COLLECTOR_ORIGIN.into(),
            oidc: None,
            tokens: vec![("collector".into(), f.bob.clone())],
            client_id: None,
        }),
        config,
    );
    let key = ed25519_dalek::SigningKey::from_bytes(&[44; 32]);
    let device = uuid::Uuid::new_v4();
    let approval = "33".repeat(32);
    let (status, _) = signed(
        &app,
        "/collector/register",
        &key,
        device,
        json!({"id":device,"public_key":hex::encode(key.verifying_key().to_bytes()),"product":"demo","services":["frontend"],"approval_hash":approval}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for query in ["duplicate money", "retries payment"] {
        let (status, data) = signed(
            &app,
            "/collector/overlaps",
            &key,
            device,
            json!({"product":"demo","approval_hash":approval,"request":{"query":query,"limit":8}}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(data["retrieval"]["mode"], "hybrid");
        assert!(related(&data, &backend).is_empty(), "{query}: {data}");
        assert_eq!(related(&data, &frontend).len(), 1);
    }
    task.abort();
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn a_changed_card_is_not_repeated_as_a_meaning_match() {
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();
    let mut body = card();
    body.evidence =
        vec![
            serde_json::from_value(json!({"kind":"file_observation","reference":"backend/x.rs"}))
                .unwrap(),
        ];
    let id = indexed(&f, body).await;
    let (config, task, _, _) = counted_model(None).await;
    let app = app_for(&f, vec![("reader".into(), f.reader.clone())], None, config);
    let (status, result) = overlap(
        &app,
        "reader",
        json!({"files":["backend/x.rs"],"query":"duplicate money","limit":8}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["retrieval"]["mode"], "hybrid");
    let found = related(&result, &id);
    assert_eq!(found.len(), 1, "{result}");
    assert_eq!(found[0]["kind"], "changed");
    task.abort();
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn a_slow_model_falls_back_to_text_within_the_hook_timeout() {
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();
    let id = indexed(&f, card()).await;
    let (config, task, _, embeds) = delayed_model(None, std::time::Duration::from_secs(5)).await;
    let app = app_for(&f, vec![("reader".into(), f.reader.clone())], None, config);
    let started = std::time::Instant::now();
    let (status, result) =
        overlap(&app, "reader", json!({"query":"retries payment","limit":8})).await;
    let elapsed = started.elapsed();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(embeds.load(Ordering::SeqCst), 1);
    assert_eq!(
        result["retrieval"],
        json!({"mode":"lexical","reason":"embedding_timeout"})
    );
    assert_eq!(related(&result, &id)[0]["matched_by"], "text");
    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "took {elapsed:?}; the hook client gives up at 2 s"
    );
    task.abort();
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn prompt_overlaps_use_the_prompt_floor_and_search_keeps_its_own() {
    let f = common::setup().await;
    f.k.semantic_migrate().await.unwrap();
    let c = f.k.submit(&f.alice, "demo", card()).await.unwrap();
    f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    let id = c.id.to_string();
    let source =
        f.k.semantic_pending(&f.alice.org, "demo", &ns())
            .await
            .unwrap()
            .remove(0);
    // Cosine similarity 0.57 to the fake model's [1,0,0]: above 0.55, below search's 0.6.
    let y = (1f32 - 0.57 * 0.57).sqrt();
    f.k.semantic_store(&f.alice.org, "demo", &ns(), &source, &[0.57, y, 0.])
        .await
        .unwrap();
    let (config, task, _) = model(None).await;
    let app = |client: Embeddings| {
        wk_server::portal_router_with_embeddings(
            f.k.clone(),
            vec![("reader".into(), f.reader.clone())],
            None,
            None,
            None,
            None,
            Some(client),
        )
        .unwrap()
    };
    let default = app(Embeddings::new(config.clone()).unwrap());
    let (_, result) = overlap(
        &default,
        "reader",
        json!({"query":"duplicate money","limit":8}),
    )
    .await;
    assert_eq!(result["retrieval"]["mode"], "hybrid");
    assert_eq!(related(&result, &id)[0]["matched_by"], "meaning");
    let (status, result) = send(&default, "duplicate money", "reader").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["retrieval"]["mode"], "hybrid");
    assert_eq!(result["cards"], json!([]), "search keeps its 0.6 floor");
    let strict = app(Embeddings::new(config)
        .unwrap()
        .with_prompt_floor(wk_server::embeddings::parse_prompt_floor("0.6").unwrap()));
    let (_, result) = overlap(
        &strict,
        "reader",
        json!({"query":"duplicate money","limit":8}),
    )
    .await;
    assert_eq!(result["retrieval"]["mode"], "hybrid");
    assert!(related(&result, &id).is_empty(), "configured floor is used");
    task.abort();
}
