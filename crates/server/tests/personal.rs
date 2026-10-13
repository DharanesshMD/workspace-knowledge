use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_knowledge::{Actor, Knowledge};
async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    token: &str,
) -> (StatusCode, Value) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn body() -> Value {
    json!({"schema_version":1,"idempotency_key":"owner-http","policy_revision":1,"title":"Tenant rate limiter","intent":"Prevent overload","rationale":"Shared middleware","decisions":["tenant limits"],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn private_routes_reject_other_identities_and_submit_only_to_review() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let owner = Actor {
        org: org.clone(),
        user: "owner".into(),
    };
    let reviewer = Actor {
        org,
        user: "reviewer".into(),
    };
    for who in [&owner, &reviewer] {
        k.grant(who, "demo", &["backend".into()], true)
            .await
            .unwrap();
    }
    let app = wk_server::configured_router(
        k.clone(),
        vec![
            ("owner".into(), owner.clone()),
            ("reviewer".into(), reviewer.clone()),
        ],
        None,
        None,
    );
    let root = "/v1/products/demo/personal";
    assert_eq!(
        call(&app, "GET", root, None, "missing").await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, note) = call(
        &app,
        "POST",
        root,
        Some(json!({"body":body(),"provenance":"captured"})),
        "owner",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = note["summary"]["id"].as_str().unwrap();
    let path = format!("{root}/{id}");
    assert_eq!(
        call(&app, "GET", &path, None, "reviewer").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "GET",
            &format!("{root}/{}", uuid::Uuid::new_v4()),
            None,
            "owner"
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", root, None, "reviewer").await.1["summaries"],
        json!([])
    );
    let mut unknown = json!({"body":body(),"provenance":"captured"});
    unknown["messages"] = json!(["RAW_CANARY_NEVER_UPLOAD"]);
    let r = call(&app, "POST", root, Some(unknown), "owner").await;
    assert_eq!(r.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!r.1.to_string().contains("RAW_CANARY"));
    let mut sensitive = body();
    sensitive["idempotency_key"] = json!("secret-test");
    sensitive["intent"] = json!("sk-proj-SYNTHETIC_PRIVATE_CANARY_012345678901234567890");
    let r = call(
        &app,
        "POST",
        root,
        Some(json!({"body":sensitive,"provenance":"captured"})),
        "owner",
    )
    .await;
    assert_eq!(r.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!r.1.to_string().contains("CANARY"));
    assert_eq!(
        k.get(&owner, "demo", uuid::Uuid::parse_str(id).unwrap())
            .await
            .unwrap_err(),
        wk_knowledge::Error::NotFound
    );
    assert!(k.review_queue(&reviewer, "demo").await.unwrap().is_empty());
    assert!(k
        .search(&reviewer, "demo", "limiter", 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(k.graph(&owner, "demo").await.unwrap()["nodes"], json!([]));
    assert_eq!(
        call(
            &app,
            "PATCH",
            &path,
            Some(json!({"expected_revision":9,"body":body()})),
            "owner"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let submit = format!("{path}/submit");
    assert_eq!(
        call(
            &app,
            "POST",
            &submit,
            Some(json!({"expected_revision":1})),
            "reviewer"
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, shared) = call(
        &app,
        "POST",
        &submit,
        Some(json!({"expected_revision":1})),
        "owner",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(shared["card"]["published"], false);
    assert_eq!(
        call(
            &app,
            "POST",
            &submit,
            Some(json!({"expected_revision":1})),
            "owner"
        )
        .await
        .1["card"]["id"],
        shared["card"]["id"]
    );
    assert_eq!(k.review_queue(&reviewer, "demo").await.unwrap().len(), 1);
    assert!(k
        .search(&reviewer, "demo", "limiter", 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        call(
            &app,
            "DELETE",
            &path,
            Some(json!({"expected_revision":1})),
            "owner"
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "GET", &path, None, "owner").await.0,
        StatusCode::NOT_FOUND
    );
}
