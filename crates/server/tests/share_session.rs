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
    author: Actor,
    member: Actor,
}
/// `demo` is enabled for publish on submit, `closed` is not. `multi` holds two services.
async fn setup() -> World {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url)
        .await
        .unwrap()
        .with_auto_publish_products(AutoPublish::Only(vec!["demo".into()]))
        .unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let actor = |user: &str| Actor {
        org: org.clone(),
        user: user.into(),
    };
    let (author, member, multi) = (actor("author"), actor("member"), actor("multi"));
    for product in ["demo", "closed"] {
        for a in [&author, &member] {
            k.grant(a, product, &["backend".into()], false)
                .await
                .unwrap();
        }
        k.grant(
            &multi,
            product,
            &["backend".into(), "frontend".into()],
            false,
        )
        .await
        .unwrap();
    }
    World { k, author, member }
}
async fn router(w: &World) -> axum::Router {
    let org = w.author.org.clone();
    let user = |u: &str| Actor {
        org: org.clone(),
        user: u.into(),
    };
    wk_server::router(
        w.k.clone(),
        vec![
            ("author-token".into(), w.author.clone()),
            ("member-token".into(), w.member.clone()),
            ("multi-token".into(), user("multi")),
            ("stranger-token".into(), user("stranger")),
        ],
    )
}
async fn post(app: &axum::Router, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if !token.is_empty() {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let r = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn found(app: &axum::Router, token: &str, product: &str) -> usize {
    let (status, body) = post(
        app,
        token,
        "/v1/search",
        json!({"product":product,"query":"retries","limit":10}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["cards"].as_array().unwrap().len()
}
async fn stored(w: &World) -> i64 {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query_scalar("SELECT count(*) FROM cards WHERE org=$1")
        .bind(&w.author.org)
        .fetch_one(&pool)
        .await
        .unwrap()
}
fn summary(product: &str) -> Value {
    json!({"product":product,"title":"Cap retries","intent":"Avoid repeat charges","rationale":"The gateway charges twice on duplicate calls.","decisions":["cap at three"],"constraints":[],"open_questions":["configurable cap?"]})
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn share_route_publishes_for_an_enabled_product_and_reports_how_to_withdraw() {
    let w = setup().await;
    let app = router(&w).await;
    let (status, body) = post(&app, "author-token", "/v1/share", summary("demo")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["card_id"].as_str().unwrap().to_string();
    assert_eq!(body["published"], true);
    assert_eq!(body["status"], "published");
    assert_eq!(
        body["withdraw"],
        format!("Open the card in the portal (My context, Withdraw knowledge) or POST /v1/cards/{id}/withdraw")
    );
    assert_eq!(found(&app, "member-token", "demo").await, 1);
    // Repeating the same share is idempotent.
    let (status, again) = post(&app, "author-token", "/v1/share", summary("demo")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["card_id"], body["card_id"]);
    assert_eq!(stored(&w).await, 1);
    // The reported revision is what the withdraw route takes.
    let (status, _) = post(
        &app,
        "author-token",
        &format!("/v1/cards/{id}/withdraw"),
        json!({"product":"demo","revision":body["revision"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found(&app, "member-token", "demo").await, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn share_route_without_publishing_enabled_is_a_review_copy() {
    let w = setup().await;
    let app = router(&w).await;
    let (status, body) = post(&app, "author-token", "/v1/share", summary("closed")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["published"], false);
    assert_eq!(body["status"], "submitted_for_review");
    assert_eq!(found(&app, "member-token", "closed").await, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn share_route_refuses_what_it_should() {
    let w = setup().await;
    let app = router(&w).await;
    // Several services: 422 naming them; a held one works; one not held is forbidden.
    let (status, body) = post(&app, "multi-token", "/v1/share", summary("demo")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "service_required");
    assert_eq!(body["services"], json!(["backend", "frontend"]));
    let mut pick = summary("demo");
    pick["service"] = json!("frontend");
    assert_eq!(
        post(&app, "multi-token", "/v1/share", pick).await.0,
        StatusCode::OK
    );
    let mut other = summary("demo");
    other["service"] = json!("payments");
    assert_eq!(
        post(&app, "multi-token", "/v1/share", other).await.0,
        StatusCode::FORBIDDEN
    );
    let before = stored(&w).await;
    // Non-member and missing credentials.
    assert_eq!(
        post(&app, "stranger-token", "/v1/share", summary("demo"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&app, "", "/v1/share", summary("demo")).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Sensitive text: same error as /v1/cards, and nothing is stored.
    let mut leaked = summary("demo");
    leaked["rationale"] = json!("password = CANARY");
    let (status, body) = post(&app, "author-token", "/v1/share", leaked).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "sensitive_summary");
    assert!(!body.to_string().contains("CANARY"));
    // A caller cannot supply provenance or any other card field, or leave out required text.
    for (field, value) in [
        ("provenance", json!("assistant_written")),
        ("publish_on_submit", json!(true)),
        ("card", json!({})),
        ("policy_revision", json!(1)),
    ] {
        let mut extra = summary("demo");
        extra[field] = value;
        assert_eq!(
            post(&app, "author-token", "/v1/share", extra).await.0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{field}"
        );
    }
    let mut missing = summary("demo");
    missing.as_object_mut().unwrap().remove("title");
    assert_eq!(
        post(&app, "author-token", "/v1/share", missing).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut blank = summary("demo");
    blank["title"] = json!(" ");
    assert_eq!(
        post(&app, "author-token", "/v1/share", blank).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(stored(&w).await, before);
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
async fn rpc(app: &axum::Router, token: &str, method: &str, params: Value) -> Value {
    let r = app
        .clone()
        .oneshot(mcp(token, method, params))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
async fn share_tool(app: &axum::Router, token: &str, arguments: Value) -> Value {
    rpc(
        app,
        token,
        "tools/call",
        json!({"name":"knowledge.share_session","arguments":arguments}),
    )
    .await
}
fn tool_data(v: &Value) -> Value {
    assert_ne!(v["result"]["isError"], true, "{v}");
    serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn failed(v: &Value) -> bool {
    v["result"]["isError"] == true || v["error"].is_object()
}
fn arguments() -> Value {
    let mut v = summary("demo");
    v.as_object_mut().unwrap().remove("product");
    v
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn hosted_tool_is_listed_with_its_guidance_and_shares_with_only_the_users_sign_in() {
    let w = setup().await;
    let app = router(&w).await;
    let listing = rpc(&app, "author-token", "tools/list", json!({})).await;
    let tools = listing["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 12);
    let tool = tools
        .iter()
        .find(|t| t["name"] == "knowledge.share_session")
        .unwrap();
    let description = tool["description"].as_str().unwrap();
    assert!(description.starts_with(
        "Share a summary of the current work with your team. Call this ONLY when the developer has asked you to share or track this session or project, including through an explicit project instruction to share finished work."
    ));
    assert!(description.ends_with("the developer can withdraw it later."));
    assert!(
        description.contains("say in words what it showed"),
        "an attached image reaches teammates only as words"
    );
    let properties = tool["inputSchema"]["properties"].as_object().unwrap();
    let mut names: Vec<_> = properties.keys().cloned().collect();
    names.sort();
    assert_eq!(
        names,
        [
            "constraints",
            "decisions",
            "files",
            "intent",
            "open_questions",
            "rationale",
            "service",
            "title"
        ]
    );
    // The existing draft tool is unchanged.
    assert!(tools.iter().any(|t| t["name"] == "knowledge.record"));
    let data = tool_data(&share_tool(&app, "author-token", arguments()).await);
    assert_eq!(data["published"], true);
    assert_eq!(data["status"], "published");
    assert!(data["card_id"].is_string());
    assert!(data["withdraw"]
        .as_str()
        .unwrap()
        .contains("Withdraw knowledge"));
    assert_eq!(found(&app, "member-token", "demo").await, 1);
    // Identical repeat: same card.
    let again = tool_data(&share_tool(&app, "author-token", arguments()).await);
    assert_eq!(again["card_id"], data["card_id"]);
    assert_eq!(stored(&w).await, 1);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn hosted_tool_rejects_provenance_sensitive_text_and_ambiguous_services() {
    let w = setup().await;
    let app = router(&w).await;
    // The caller has no way to claim provenance.
    let mut claim = arguments();
    claim["provenance"] = json!("assistant_written");
    assert!(failed(&share_tool(&app, "author-token", claim).await));
    let mut leaked = arguments();
    leaked["intent"] = json!("password = CANARY");
    let v = share_tool(&app, "author-token", leaked).await;
    assert!(failed(&v));
    assert!(!v.to_string().contains("CANARY"));
    // Several services: the error says which.
    let v = share_tool(&app, "multi-token", arguments()).await;
    assert!(failed(&v));
    assert!(v.to_string().contains("service_required"), "{v}");
    assert!(v.to_string().contains("backend") && v.to_string().contains("frontend"));
    // A held service works for that member; a stranger is refused.
    let mut pick = arguments();
    pick["service"] = json!("backend");
    assert!(tool_data(&share_tool(&app, "multi-token", pick).await)["published"] == true);
    let r = app
        .clone()
        .oneshot(mcp(
            "stranger-token",
            "tools/call",
            json!({"name":"knowledge.share_session","arguments":arguments()}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(found(&app, "member-token", "demo").await, 1);
    assert_eq!(stored(&w).await, 1);
}
