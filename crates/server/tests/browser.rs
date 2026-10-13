use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
fn app() -> axum::Router {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1/wk_test")
        .unwrap();
    wk_server::configured_router(
        wk_knowledge::Knowledge::from_pool(pool),
        vec![],
        None,
        Some("http://127.0.0.1:4318".into()),
    )
}
#[tokio::test]
async fn browser_configuration_and_unavailable_login_are_explicit() {
    let r = app()
        .oneshot(
            Request::builder()
                .uri("/auth/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["browser_login"], false);
    assert_eq!(v["development"], true);
    let r = app()
        .oneshot(
            Request::builder()
                .uri("/auth/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
}
#[tokio::test]
async fn me_requires_identity_and_cookie_cannot_authenticate_mcp() {
    for uri in ["/v1/me", "/mcp/demo"] {
        let r = app()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("cookie", "wk_dev_session=synthetic-cookie")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
}
#[tokio::test]
async fn mixed_credentials_and_cross_origin_cookie_mutations_are_rejected() {
    let r = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/cards")
                .header("cookie", "wk_dev_session=synthetic-cookie")
                .header("origin", "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert_eq!(r.headers()["referrer-policy"], "no-referrer");
    let r = app()
        .oneshot(
            Request::builder()
                .uri("/v1/me")
                .header("cookie", "wk_dev_session=synthetic-cookie")
                .header("authorization", "Bearer another")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(r.headers()["cache-control"], "no-store");
}
