use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_knowledge::{Actor, AutoPublish, Knowledge};

struct World {
    k: Knowledge,
    org: String,
}
async fn world() -> (World, axum::Router) {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url)
        .await
        .unwrap()
        .with_auto_publish_products(AutoPublish::Only(vec!["demo".into()]))
        .unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let actor = |u: &str| Actor {
        org: org.clone(),
        user: u.into(),
    };
    for (who, services) in [
        ("sam", vec!["backend"]),
        ("anika", vec!["backend", "frontend"]),
        ("jo", vec!["frontend"]),
    ] {
        let services: Vec<String> = services.into_iter().map(String::from).collect();
        k.grant(&actor(who), "demo", &services, false)
            .await
            .unwrap();
    }
    let app = wk_server::router(
        k.clone(),
        vec![
            ("sam-token".into(), actor("sam")),
            ("anika-token".into(), actor("anika")),
            ("jo-token".into(), actor("jo")),
            ("stranger-token".into(), actor("stranger")),
        ],
    );
    (World { k, org }, app)
}
async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let r = app.clone().oneshot(request).await.unwrap();
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn post(app: &axum::Router, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let mut r = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if !token.is_empty() {
        r = r.header("authorization", format!("Bearer {token}"));
    }
    send(app, r.body(Body::from(body.to_string())).unwrap()).await
}
fn mcp(token: &str, method: &str, params: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("http://localhost/mcp/demo")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-06-18")
        .body(Body::from(
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
        ))
        .unwrap()
}
async fn tool(app: &axum::Router, token: &str, name: &str, arguments: Value) -> Value {
    send(
        app,
        mcp(
            token,
            "tools/call",
            json!({"name":name,"arguments":arguments}),
        ),
    )
    .await
    .1
}
fn data(v: &Value) -> Value {
    assert_ne!(v["result"]["isError"], true, "{v}");
    serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn failed(v: &Value) -> bool {
    v["result"]["isError"] == true || v["error"].is_object()
}
fn message(v: &Value) -> String {
    v["result"]["content"][0]["text"]
        .as_str()
        .or_else(|| v["error"]["message"].as_str())
        .unwrap_or("")
        .to_string()
}
fn share(title: &str, files: &[&str]) -> Value {
    json!({"title":title,"intent":format!("Goal of {title}"),"rationale":"Because","decisions":["a decision"],"files":files})
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn the_rest_route_answers_who_did_what_where() {
    let (w, app) = world().await;
    let mut body = share("Retry with backoff", &["backend/payments/retry.rs"]);
    body["product"] = json!("demo");
    let (status, shared) = post(&app, "sam-token", "/v1/share", body).await;
    assert_eq!(status, StatusCode::OK, "{shared}");

    let path = "/v1/products/demo/team-work";
    let (status, found) = post(
        &app,
        "anika-token",
        path,
        json!({"developer":"SA","paths":["backend/payments"],"since_days":7}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["status"], "ok");
    assert_eq!(found["resolved"]["developer"], "sam");
    assert_eq!(found["entries"][0]["title"], "Retry with backoff");
    assert_eq!(
        found["entries"][0]["files_reported"],
        json!(["backend/payments/retry.rs"])
    );
    assert_eq!(
        found["entries"][0]["matched_files"],
        json!(["backend/payments/retry.rs"])
    );
    assert!(found["text"].as_str().unwrap().contains("sam"));

    // Refusals: no sign-in, a stranger, bad input, a field the API does not have.
    assert_eq!(
        post(&app, "", path, json!({})).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&app, "stranger-token", path, json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&app, "anika-token", path, json!({"paths":["../x"]}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(
            &app,
            "anika-token",
            path,
            json!({"developer":"sam","author":"sam"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Frontend-only jo cannot see backend work, so sam is simply unknown to jo.
    let (_, hidden) = post(&app, "jo-token", path, json!({"developer":"sam"})).await;
    assert_eq!(
        hidden["status"],
        "no_matches".replace("no_matches", "developer_not_found")
    );
    // The share route refuses a bad file list and stores nothing for it.
    let mut bad = share("Bad", &["../x"]);
    bad["product"] = json!("demo");
    assert_eq!(
        post(&app, "sam-token", "/v1/share", bad).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM cards WHERE org=$1")
        .bind(&w.org)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 1);
    let _ = &w.k;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn the_hosted_tool_is_listed_and_works_end_to_end_with_recoverable_mistakes() {
    let (_w, app) = world().await;
    let listing = send(&app, mcp("anika-token", "tools/list", json!({})))
        .await
        .1;
    let tools = listing["result"]["tools"].as_array().unwrap();
    let t = tools
        .iter()
        .find(|t| t["name"] == "knowledge.team_work")
        .expect("tool is listed");
    let description = t["description"].as_str().unwrap();
    for needle in [
        "what a particular teammate did",
        "file or module",
        "since_days",
        "files_reported",
        "an empty result means nothing was shared",
    ] {
        assert!(description.contains(needle), "description lacks {needle:?}");
    }
    let mut names: Vec<_> = t["inputSchema"]["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "budget_tokens",
            "developer",
            "limit",
            "paths",
            "query",
            "service",
            "since_days"
        ]
    );
    // Nothing is required: every filter is optional.
    assert!(t["inputSchema"]
        .get("required")
        .is_none_or(|r| r.as_array().unwrap().is_empty()));

    // Sam shares through the tool with files; Anika asks about sam, then about the module, by tool.
    let shared = tool(
        &app,
        "sam-token",
        "knowledge.share_session",
        share(
            "Retry with backoff",
            &["backend/payments/retry.rs", "./backend/payments/gateway.rs"],
        ),
    )
    .await;
    assert_eq!(data(&shared)["published"], true);
    let by_name = data(
        &tool(
            &app,
            "anika-token",
            "knowledge.team_work",
            json!({"developer":"sam"}),
        )
        .await,
    );
    assert_eq!(by_name["entries"][0]["developer"], "sam");
    let by_module = data(
        &tool(
            &app,
            "anika-token",
            "knowledge.team_work",
            json!({"paths":["backend/payments"],"since_days":1,"query":"retry"}),
        )
        .await,
    );
    assert_eq!(
        by_module["entries"][0]["files_reported"],
        json!(["backend/payments/gateway.rs", "backend/payments/retry.rs"])
    );
    assert_eq!(
        by_module["entries"][0]["matched_by"],
        json!(["path", "text"])
    );

    // A wrong name is a normal answer that names the right ones, so the agent can retry itself.
    let wrong = data(
        &tool(
            &app,
            "anika-token",
            "knowledge.team_work",
            json!({"developer":"samm"}),
        )
        .await,
    );
    assert_eq!(wrong["status"], "developer_not_found");
    assert_eq!(wrong["known_developers"], json!(["sam"]));
    // Bad arguments are an error the model can read and fix.
    let bad = tool(
        &app,
        "anika-token",
        "knowledge.team_work",
        json!({"paths":["/etc"],"since_days":400}),
    )
    .await;
    assert!(failed(&bad), "{bad}");
    assert!(message(&bad).contains("workspace-relative"), "{bad}");
    // Unknown arguments are rejected rather than ignored.
    assert!(failed(
        &tool(
            &app,
            "anika-token",
            "knowledge.team_work",
            json!({"author":"sam"})
        )
        .await
    ));
    // Someone outside the product never reaches a tool at all.
    let request = mcp(
        "stranger-token",
        "tools/call",
        json!({"name":"knowledge.team_work","arguments":{}}),
    );
    assert_eq!(send(&app, request).await.0, StatusCode::FORBIDDEN);
}
