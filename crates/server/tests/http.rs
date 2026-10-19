use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;
fn app() -> axum::Router {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1/wk_test")
        .unwrap();
    wk_server::router(wk_knowledge::Knowledge::from_pool(pool), vec![])
}
#[tokio::test]
async fn missing_or_invalid_identity_is_unauthorized() {
    for auth in [None, Some("Bearer invalid")] {
        let mut b = Request::builder()
            .method("POST")
            .uri("/v1/search")
            .header("content-type", "application/json");
        if let Some(v) = auth {
            b = b.header("authorization", v);
        }
        let r = app()
            .oneshot(
                b.body(Body::from(
                    r#"{"product":"demo","query":"limiter","limit":5}"#,
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
}
#[tokio::test]
async fn development_mode_is_explicit_in_health() {
    let r = app()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    use http_body_util::BodyExt;
    let body = r.into_body().collect().await.unwrap().to_bytes();
    assert!(std::str::from_utf8(&body).unwrap().contains("development"));
}
#[tokio::test]
async fn invalid_body_does_not_echo_private_input() {
    use http_body_util::BodyExt;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1/wk_test")
        .unwrap();
    let router = wk_server::router(
        wk_knowledge::Knowledge::from_pool(pool),
        vec![(
            "test-token".into(),
            wk_knowledge::Actor {
                org: "o".into(),
                user: "u".into(),
            },
        )],
    );
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/cards")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"PRIVATE_CANARY":"secret"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE_CANARY"));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn sensitive_correction_error_does_not_echo_canary() {
    use http_body_util::BodyExt;
    use serde_json::json;
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = wk_knowledge::Knowledge::connect(&url).await.unwrap();
    let actor = wk_knowledge::Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "owner".into(),
    };
    k.grant(&actor, "demo", &["backend".into()], true)
        .await
        .unwrap();
    let mut body = json!({"schema_version":1,"idempotency_key":"privacy-http","policy_revision":1,"title":"Retries","intent":"Bound retries","rationale":"Use idempotency","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null});
    let saved = k
        .submit(
            &actor,
            "demo",
            serde_json::from_value(body.clone()).unwrap(),
        )
        .await
        .unwrap();
    body["base_revision"] = json!(saved.revision);
    body["rationale"] = json!("password = HTTP-PRIVATE-CANARY");
    let r = wk_server::router(k, vec![("synthetic-token".into(), actor)])
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/cards/{}/correct", saved.id))
                .header("authorization", "Bearer synthetic-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"product":"demo","card":body}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains("HTTP-PRIVATE-CANARY"));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn policy_advertises_card_provenance() {
    use http_body_util::BodyExt;
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = wk_knowledge::Knowledge::connect(&url).await.unwrap();
    let actor = wk_knowledge::Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "owner".into(),
    };
    k.grant(&actor, "demo", &["backend".into()], true)
        .await
        .unwrap();
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
    assert_eq!(
        v["policy"]["features"],
        serde_json::json!(["card_provenance"])
    );
}
