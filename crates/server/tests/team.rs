use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_knowledge::{Actor, Knowledge};
async fn get(app: &axum::Router, path: &str, token: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn team_routes_are_published_scoped_bounded_and_strict() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let a = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "alice".into(),
    };
    k.grant(&a, "demo", &["backend".into()], true)
        .await
        .unwrap();
    let card=k.submit(&a,"demo",serde_json::from_value(json!({"schema_version":1,"idempotency_key":"http","policy_revision":1,"title":"Tenant limiter","intent":"Bound work","rationale":"Shared middleware","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()).await.unwrap();
    let app =
        wk_server::configured_router(k.clone(), vec![("owner".into(), a.clone())], None, None);
    let root = "/v1/products/demo/context";
    assert_eq!(get(&app, root, "missing").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(get(&app, root, "owner").await.0, StatusCode::OK);
    assert_eq!(get(&app, root, "owner").await.1["entries"], json!([]));
    let bad = get(&app, &format!("{root}?messages=RAW_CANARY"), "owner").await;
    assert_eq!(bad.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!bad.1.to_string().contains("RAW_CANARY"));
    assert_eq!(
        get(&app, &format!("{root}?since_ms=bad"), "owner").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    k.publish(&a, "demo", card.id).await.unwrap();
    let feed = get(&app, root, "owner").await.1;
    assert_eq!(feed["entries"].as_array().unwrap().len(), 1);
    assert_eq!(feed["entries"][0]["author"], "alice");
    assert_eq!(feed["entries"][0]["card"]["published"], true);
    assert!(feed["next_cursor"].is_null());
    assert_eq!(
        get(&app, &format!("{root}?author=hidden"), "owner").await.1["entries"],
        json!([])
    );
    k.record_activity(
        &a,
        "demo",
        wk_knowledge::activity::ActivityUpdate {
            session_id: "native-private-session".into(),
            services: vec!["backend".into()],
            files: vec!["backend/lib.rs".into()],
            intent: "Review bounded work".into(),
            ended: false,
        },
    )
    .await
    .unwrap();
    let signal = get(&app, "/v1/products/demo/activity", "owner").await.1;
    assert_eq!(signal["activity"].as_array().unwrap().len(), 1);
    assert!(!signal.to_string().contains("native-private-session"));
}
