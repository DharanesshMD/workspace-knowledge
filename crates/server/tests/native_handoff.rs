use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_knowledge::{Actor, Knowledge};
async fn post(app: &axum::Router, token: &str, path: &str, body: Value) -> (u16, Value) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status().as_u16();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn reviewed_native_card_reaches_only_authorized_teammates_with_revision() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let a = Actor {
        org: org.clone(),
        user: "owner".into(),
    };
    let r = Actor {
        org: org.clone(),
        user: "reviewer".into(),
    };
    let b = Actor {
        org: org.clone(),
        user: "teammate".into(),
    };
    let u = Actor {
        org: org.clone(),
        user: "denied".into(),
    };
    let x = Actor {
        org: format!("other-{org}"),
        user: "other".into(),
    };
    for (actor, services, review) in [
        (&a, vec!["backend".into()], false),
        (&r, vec!["backend".into()], true),
        (&b, vec!["backend".into()], false),
        (&u, vec!["frontend".into()], false),
        (&x, vec!["backend".into()], true),
    ] {
        k.grant(actor, "demo", &services, review).await.unwrap();
    }
    let app = wk_server::router(
        k.clone(),
        vec![
            ("a".into(), a.clone()),
            ("r".into(), r),
            ("b".into(), b.clone()),
            ("u".into(), u),
            ("x".into(), x),
        ],
    );
    let card = if let Ok(path) = std::env::var("WK_NATIVE_CARD_FILE") {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    } else {
        json!({"schema_version":1,"idempotency_key":"native-fixture","policy_revision":1,"title":"Retry idempotency","intent":"Bound retries","rationale":"Prevent duplicate operations","decisions":["Retries require idempotency"],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[{"kind":"file_observation","reference":"backend/retry.ts"}],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})
    };
    let (status, saved) = post(
        &app,
        "a",
        "/v1/cards",
        json!({"product":"demo","card":card}),
    )
    .await;
    assert_eq!(status, 200);
    let id = saved["card"]["id"].as_str().unwrap();
    let q = json!({"product":"demo","query":"idempotency","limit":10});
    assert_eq!(
        post(&app, "b", "/v1/search", q.clone()).await.1["cards"],
        json!([])
    );
    assert_eq!(
        post(
            &app,
            "a",
            &format!("/v1/cards/{id}/publish"),
            json!({"product":"demo"})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        post(
            &app,
            "r",
            &format!("/v1/cards/{id}/publish"),
            json!({"product":"demo"})
        )
        .await
        .0,
        200
    );
    let shared = post(&app, "b", "/v1/search", q.clone()).await.1;
    assert_eq!(shared["cards"][0]["id"], id);
    assert!(!shared.to_string().contains("OWNER-RAW-CANARY"));
    let (_, overlap) = post(
        &app,
        "b",
        "/v1/products/demo/overlaps",
        json!({"files":["backend/retry.ts"],"limit":8}),
    )
    .await;
    assert_eq!(overlap["notices"][0]["card_id"], id);
    assert_eq!(overlap["notices"][0]["revision"], 1);
    for token in ["u", "x"] {
        assert_eq!(
            post(&app, token, "/v1/search", q.clone()).await.1["cards"],
            json!([])
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/products/demo/cards/{id}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(matches!(response.status().as_u16(), 403 | 404));
    }
    let revisions = k
        .revisions(&b, "demo", uuid::Uuid::parse_str(id).unwrap())
        .await
        .unwrap();
    assert!(!serde_json::to_string(&revisions)
        .unwrap()
        .contains("OWNER-RAW-CANARY"));
}
