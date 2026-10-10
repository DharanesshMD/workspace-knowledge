use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_knowledge::{Actor, Knowledge};
#[allow(clippy::too_many_arguments)]
async fn call(
    app: &axum::Router,
    path: &str,
    token: &str,
    key: &SigningKey,
    id: uuid::Uuid,
    nonce: uuid::Uuid,
    body: Value,
    tamper: bool,
) -> (StatusCode, Value) {
    let raw = serde_json::to_vec(&body).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let signature = key.sign(&wk_contracts::device::signing_bytes(
        "http://127.0.0.1:4318",
        "POST",
        path,
        now,
        nonce,
        id,
        token,
        &raw,
    ));
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-wk-device", id.to_string())
        .header("x-wk-time", now.to_string())
        .header("x-wk-nonce", nonce.to_string())
        .header("x-wk-signature", hex::encode(signature.to_bytes()));
    if tamper {
        req = req.header("cookie", "wk_session=invalid")
    }
    let r = app
        .clone()
        .oneshot(req.body(Body::from(raw)).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn collector_signature_replay_revocation_and_api_separation() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let a = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "owner".into(),
    };
    k.grant(&a, "demo", &["backend".into()], false)
        .await
        .unwrap();
    let app = wk_server::portal_router_with_collector(
        k.clone(),
        vec![("browser-token".into(), a.clone())],
        None,
        Some("http://127.0.0.1:4318".into()),
        None,
        Some(wk_server::device_routes::CollectorAuth {
            origin: "http://127.0.0.1:4318".into(),
            oidc: None,
            tokens: vec![("collector-token".into(), a.clone())],
            client_id: None,
        }),
    )
    .unwrap();
    let key = SigningKey::from_bytes(&[42; 32]);
    let id = uuid::Uuid::new_v4();
    let registration = json!({"id":id,"public_key":hex::encode(key.verifying_key().to_bytes()),"product":"demo","services":["backend"],"approval_hash":"22".repeat(32)});
    assert_eq!(
        call(
            &app,
            "/collector/register",
            "browser-token",
            &key,
            id,
            uuid::Uuid::new_v4(),
            registration.clone(),
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let nonce = uuid::Uuid::new_v4();
    assert_eq!(
        call(
            &app,
            "/collector/register",
            "collector-token",
            &key,
            id,
            nonce,
            registration.clone(),
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "/collector/register",
            "collector-token",
            &key,
            id,
            nonce,
            registration.clone(),
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let scope = json!({"product":"demo","approval_hash":"22".repeat(32),"request":{}});
    assert_eq!(
        call(
            &app,
            "/collector/policy",
            "collector-token",
            &key,
            id,
            uuid::Uuid::new_v4(),
            scope.clone(),
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "/collector/policy",
            "collector-token",
            &SigningKey::from_bytes(&[43; 32]),
            id,
            uuid::Uuid::new_v4(),
            scope.clone(),
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "/collector/policy",
            "collector-token",
            &key,
            id,
            uuid::Uuid::new_v4(),
            scope.clone(),
            true
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let unknown = json!({"product":"demo","approval_hash":"22".repeat(32),"request":{"raw_messages":"RAW_CANARY"}});
    let bad = call(
        &app,
        "/collector/policy",
        "collector-token",
        &key,
        id,
        uuid::Uuid::new_v4(),
        unknown,
        false,
    )
    .await;
    assert_eq!(bad.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!bad.1.to_string().contains("RAW_CANARY"));
    for path in ["/v1/policy?product=demo", "/mcp/demo"] {
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", "Bearer collector-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED)
    }
    let card = json!({"schema_version":1,"idempotency_key":"signed-test","policy_revision":1,"title":"Bound work","intent":"Limit work","rationale":"","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null});
    let shared = json!({"product":"demo","approval_hash":"22".repeat(32),"request":{"product":"demo","card":card}});
    assert_eq!(
        call(
            &app,
            "/collector/cards",
            "collector-token",
            &key,
            id,
            uuid::Uuid::new_v4(),
            shared.clone(),
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    let personal = json!({"product":"demo","approval_hash":"22".repeat(32),"request":{"body":card,"provenance":"captured"}});
    assert_eq!(
        call(
            &app,
            "/collector/personal",
            "collector-token",
            &key,
            id,
            uuid::Uuid::new_v4(),
            personal.clone(),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    k.device_change(
        &a,
        "demo",
        id,
        1,
        wk_contracts::device::DeviceSettings {
            owner_sync: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        call(
            &app,
            "/collector/personal",
            "collector-token",
            &key,
            id,
            uuid::Uuid::new_v4(),
            personal,
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    // The signature covers exact body bytes, route and freshness, independently of JSON parsing.
    for (proof_path, sent_path, age, suffix) in [
        ("/collector/policy", "/collector/cards", 0, ""),
        ("/collector/policy", "/collector/policy", 0, " "),
        ("/collector/policy", "/collector/policy", 61, ""),
    ] {
        let raw = serde_json::to_vec(&scope).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - age;
        let nonce = uuid::Uuid::new_v4();
        let sig = key.sign(&wk_contracts::device::signing_bytes(
            "http://127.0.0.1:4318",
            "POST",
            proof_path,
            now,
            nonce,
            id,
            "collector-token",
            &raw,
        ));
        let mut sent = raw;
        sent.extend_from_slice(suffix.as_bytes());
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(sent_path)
                    .header("authorization", "Bearer collector-token")
                    .header("x-wk-device", id.to_string())
                    .header("x-wk-time", now.to_string())
                    .header("x-wk-nonce", nonce.to_string())
                    .header("x-wk-signature", hex::encode(sig.to_bytes()))
                    .body(Body::from(sent))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
    k.device_revoke(&a, "demo", id, 2).await.unwrap();
    assert_eq!(
        call(
            &app,
            "/collector/cards",
            "collector-token",
            &key,
            id,
            uuid::Uuid::new_v4(),
            shared,
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "/collector/policy",
            "collector-token",
            &key,
            id,
            uuid::Uuid::new_v4(),
            scope,
            false
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            "/collector/register",
            "collector-token",
            &key,
            id,
            uuid::Uuid::new_v4(),
            registration,
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
