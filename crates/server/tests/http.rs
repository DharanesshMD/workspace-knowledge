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
