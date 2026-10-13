#[path = "../../knowledge/tests/common/mod.rs"]
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn ordinary_context_is_authenticated_bounded_advisory_and_strictly_scoped() {
    let f = common::setup().await;
    let (repo, m) = common::code::repository(&f).await;
    common::code::confirmed(&f, repo, &m).await;
    let (k, pool, role) = common::runtime(&f).await;
    let app = wk_server::router(k.clone(), vec![("synthetic-bob".into(), f.bob.clone())]);
    let path = "/v1/products/demo/change-context";
    let r = serde_json::to_value(common::code::request(repo)).unwrap();
    let send = |path: &str, token: Option<&str>, body: String| {
        let mut req = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json");
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        app.clone().oneshot(req.body(Body::from(body)).unwrap())
    };
    assert_eq!(
        send(path, None, r.to_string()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let out = send(path, Some("synthetic-bob"), r.to_string())
        .await
        .unwrap();
    assert_eq!(out.status(), StatusCode::OK);
    assert_eq!(out.headers()["cache-control"], "no-store");
    let bytes = out.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.len() <= 16384);
    let brief: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(brief["decision"], "needs_review");
    assert!(brief["brief_id"].is_null());
    assert!(brief.to_string().contains("Alice Synthetic"));
    assert!(matches!(
        send(
            "/v1/products/other/change-context",
            Some("synthetic-bob"),
            r.to_string()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
    ));
    let mut bad = r.clone();
    bad["raw_prompt"] = "PRIVATE_CANARY".into();
    let out = send(path, Some("synthetic-bob"), bad.to_string())
        .await
        .unwrap();
    assert_eq!(out.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        !String::from_utf8(out.into_body().collect().await.unwrap().to_bytes().to_vec())
            .unwrap()
            .contains("PRIVATE_CANARY")
    );
    let huge = serde_json::json!({"padding":"x".repeat(65536)});
    assert_eq!(
        send(path, Some("synthetic-bob"), huge.to_string())
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    drop(app);
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}
#[tokio::test]
async fn browser_confirmation_and_pending_briefs_never_accept_bearer_credentials() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1/wk_test")
        .unwrap();
    let a = wk_knowledge::Actor {
        org: "synthetic".into(),
        user: "bob".into(),
    };
    let app = wk_server::configured_router(
        wk_knowledge::Knowledge::from_pool(pool),
        vec![("synthetic-bob".into(), a)],
        None,
        Some("http://127.0.0.1:4318".into()),
    );
    for (method, path) in [
        (
            "GET",
            "/v1/products/demo/preflights/10000000-0000-0000-0000-000000000001",
        ),
        (
            "POST",
            "/v1/products/demo/preflights/10000000-0000-0000-0000-000000000001/acknowledge",
        ),
        (
            "POST",
            "/v1/products/demo/code-contributions/10000000-0000-0000-0000-000000000001/confirm",
        ),
        (
            "POST",
            "/v1/products/demo/code-anchors/10000000-0000-0000-0000-000000000001/confirm",
        ),
    ] {
        let out = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", "Bearer synthetic-bob")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(out.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    let out = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(
                    "/v1/products/demo/preflights/10000000-0000-0000-0000-000000000001/acknowledge",
                )
                .header("cookie", format!("wk_dev_session={}", "a".repeat(64)))
                .header("origin", "http://127.0.0.1:4318")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(out.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn remote_mcp_lists_only_advisory_change_context_and_never_human_receipts() {
    let f = common::setup().await;
    let app = wk_server::router(f.k, vec![("synthetic-bob".into(), f.bob)]);
    let initialized = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("http://localhost/mcp/demo")
                .header("authorization", "Bearer synthetic-bob")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2025-06-18")
                .body(Body::from(
                    serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"synthetic-test","version":"1"}}})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let session = initialized.headers().get("mcp-session-id").cloned();
    let init_status = initialized.status();
    let init_body = String::from_utf8(
        initialized
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(init_status, StatusCode::OK, "{init_body}");
    let mut request = Request::builder()
        .method("POST")
        .uri("http://localhost/mcp/demo")
        .header("authorization", "Bearer synthetic-bob")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-06-18");
    if let Some(value) = session {
        request = request.header("mcp-session-id", value);
    }
    let r = app
        .oneshot(
            request
                .body(Body::from(
                    serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let names = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"knowledge.change_context"));
    assert!(names.contains(&"knowledge.context"));
    assert!(names.contains(&"knowledge.overlaps"));
    assert!(!names
        .iter()
        .any(|n| n.contains("acknowledge") || n.contains("consume")));
}
