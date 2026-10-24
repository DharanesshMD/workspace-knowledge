use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_knowledge::{Actor, AutoPublish, Knowledge};

fn card(key: &str, completeness: &str, assistant: bool) -> Value {
    let mut v = json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Cap retries","intent":"Avoid repeat charges","rationale":"The gateway charges twice on duplicate calls.","decisions":["cap at three"],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":completeness,"base_revision":null});
    if assistant {
        v["provenance"] = json!("assistant_written");
    }
    v
}
struct World {
    k: Knowledge,
    author: Actor,
    member: Actor,
}
/// Two non-reviewing members of `demo` (enabled) and `closed` (not enabled).
async fn setup() -> World {
    setup_with(AutoPublish::Only(vec!["demo".into()])).await
}
async fn setup_with(setting: AutoPublish) -> World {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url)
        .await
        .unwrap()
        .with_auto_publish_products(setting)
        .unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let actor = |user: &str| Actor {
        org: org.clone(),
        user: user.into(),
    };
    let (author, member) = (actor("author"), actor("member"));
    for product in ["demo", "closed"] {
        for a in [&author, &member] {
            k.grant(a, product, &["backend".into()], false)
                .await
                .unwrap();
        }
    }
    World { k, author, member }
}
fn router(w: &World) -> axum::Router {
    wk_server::router(
        w.k.clone(),
        vec![
            ("author-token".into(), w.author.clone()),
            ("member-token".into(), w.member.clone()),
        ],
    )
}
async fn post(app: &axum::Router, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
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

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn http_submit_publishes_only_when_enabled_assistant_written_and_requested() {
    let w = setup().await;
    let app = router(&w);
    let submit = |product: &str, card: Value, flag: Option<bool>| {
        let mut body = json!({"product":product,"card":card});
        if let Some(f) = flag {
            body["publish_on_submit"] = json!(f);
        }
        post(&app, "author-token", "/v1/cards", body)
    };
    // Enabled product, assistant-written card, flag set: live for the other member at once.
    let (status, body) = submit("demo", card("h1", "partial", true), Some(true)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["card"]["published"], true);
    assert_eq!(found(&app, "member-token", "demo").await, 1);
    // Idempotent retry: same card, still published, no error.
    let (status, again) = submit("demo", card("h1", "partial", true), Some(true)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["card"]["id"], body["card"]["id"]);
    assert_eq!(again["card"]["published"], true);
    // Everything else is an unpublished review copy, not an error.
    for (product, card, flag) in [
        ("demo", card("h2", "partial", true), None),
        ("demo", card("h3", "partial", true), Some(false)),
        ("demo", card("h4", "complete", false), Some(true)),
        ("demo", card("h5", "partial", false), Some(true)),
        ("closed", card("h6", "partial", true), Some(true)),
    ] {
        let (status, body) = submit(product, card, flag).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["card"]["published"], false);
    }
    assert_eq!(found(&app, "member-token", "demo").await, 1);
    assert_eq!(found(&app, "member-token", "closed").await, 0);
    // Unknown fields still fail closed, and the correction route does not take the flag.
    let (status, _) = post(
        &app,
        "author-token",
        "/v1/cards",
        json!({"product":"demo","card":card("h7","partial",true),"publish":true}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let id = body["card"]["id"].as_str().unwrap();
    let (status, _) = post(
        &app,
        "author-token",
        &format!("/v1/cards/{id}/correct"),
        json!({"product":"demo","card":card("h1","partial",true),"publish_on_submit":true}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn http_sensitive_card_is_rejected_before_anything_is_stored() {
    let w = setup().await;
    let app = router(&w);
    let mut leaked = card("sens", "partial", true);
    leaked["rationale"] = json!("password = CANARY");
    let (status, body) = post(
        &app,
        "author-token",
        "/v1/cards",
        json!({"product":"demo","card":leaked,"publish_on_submit":true}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "sensitive_summary");
    assert!(!body.to_string().contains("CANARY"));
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM cards WHERE org=$1")
        .bind(&w.author.org)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn http_non_member_and_services_mismatch_are_forbidden() {
    let w = setup().await;
    let stranger = Actor {
        org: w.author.org.clone(),
        user: "stranger".into(),
    };
    let app = wk_server::router(
        w.k.clone(),
        vec![
            ("author-token".into(), w.author.clone()),
            ("stranger-token".into(), stranger),
        ],
    );
    let (status, _) = post(
        &app,
        "stranger-token",
        "/v1/cards",
        json!({"product":"demo","card":card("n1","partial",true),"publish_on_submit":true}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let mut wider = card("n2", "partial", true);
    wider["service_ids"] = json!(["backend", "frontend"]);
    let (status, _) = post(
        &app,
        "author-token",
        "/v1/cards",
        json!({"product":"demo","card":wider,"publish_on_submit":true}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn http_author_withdraws_an_auto_published_card_and_the_member_stops_seeing_it() {
    let w = setup().await;
    let app = router(&w);
    let (_, body) = post(
        &app,
        "author-token",
        "/v1/cards",
        json!({"product":"demo","card":card("wd","partial",true),"publish_on_submit":true}),
    )
    .await;
    let (id, revision) = (&body["card"]["id"], &body["card"]["revision"]);
    assert_eq!(found(&app, "member-token", "demo").await, 1);
    let (status, _) = post(
        &app,
        "author-token",
        &format!("/v1/cards/{}/withdraw", id.as_str().unwrap()),
        json!({"product":"demo","revision":revision}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found(&app, "member-token", "demo").await, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn policy_route_advertises_publish_on_submit_for_enabled_products_only() {
    let w = setup().await;
    let app = router(&w);
    for (product, expected) in [
        ("demo", json!(["card_provenance", "publish_on_submit"])),
        ("closed", json!(["card_provenance"])),
    ] {
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/policy?product={product}"))
                    .header("authorization", "Bearer author-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let v: Value =
            serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(v["policy"]["features"], expected, "{product}");
    }
}

fn mcp_call(token: &str, name: &str, arguments: Value) -> Request<Body> {
    Request::builder().method("POST").uri("http://localhost/mcp/demo").header("authorization",format!("Bearer {token}")).header("content-type","application/json").header("accept","application/json, text/event-stream").header("mcp-protocol-version","2025-06-18").body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}).to_string())).unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn knowledge_record_never_publishes_even_where_publish_on_submit_is_enabled() {
    let w = setup().await;
    let app = router(&w);
    let r = app
        .clone()
        .oneshot(mcp_call(
            "author-token",
            "knowledge.record",
            json!({"card":card("rec","complete",false)}),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value =
        serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let data: Value =
        serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(data["card"]["published"], false);
    // A tool caller cannot claim assistant authorship to reach the publish path either.
    let r = app
        .clone()
        .oneshot(mcp_call(
            "author-token",
            "knowledge.record",
            json!({"card":card("rec2","partial",true)}),
        ))
        .await
        .unwrap();
    let v: Value =
        serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        v["result"]["isError"] == true || v["error"].is_object(),
        "{v}"
    );
    assert_eq!(found(&app, "member-token", "demo").await, 0);
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let published: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cards WHERE org=$1 AND published")
            .bind(&w.author.org)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(published, 0);
}

async fn signed(
    app: &axum::Router,
    path: &str,
    token: &str,
    key: &SigningKey,
    id: uuid::Uuid,
    body: Value,
) -> (StatusCode, Value) {
    let raw = serde_json::to_vec(&body).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let nonce = uuid::Uuid::new_v4();
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
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .header("x-wk-device", id.to_string())
                .header("x-wk-time", now.to_string())
                .header("x-wk-nonce", nonce.to_string())
                .header("x-wk-signature", hex::encode(signature.to_bytes()))
                .body(Body::from(raw))
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

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn device_cards_operation_publishes_on_submit_under_the_same_rules() {
    let w = setup().await;
    let collector_token = uuid::Uuid::new_v4().to_string();
    let app = wk_server::portal_router_with_collector(
        w.k.clone(),
        vec![("member-token".into(), w.member.clone())],
        None,
        Some("http://127.0.0.1:4318".into()),
        None,
        Some(wk_server::device_routes::CollectorAuth {
            origin: "http://127.0.0.1:4318".into(),
            oidc: None,
            tokens: vec![(collector_token.clone(), w.author.clone())],
            client_id: None,
        }),
    )
    .unwrap();
    let key = SigningKey::from_bytes(&[44; 32]);
    let id = uuid::Uuid::new_v4();
    let hash = "33".repeat(32);
    let registration = json!({"id":id,"public_key":hex::encode(key.verifying_key().to_bytes()),"product":"demo","services":["backend"],"approval_hash":hash});
    assert_eq!(
        signed(
            &app,
            "/collector/register",
            &collector_token,
            &key,
            id,
            registration
        )
        .await
        .0,
        StatusCode::OK
    );
    let op = |path: &'static str, request: Value| {
        let (app, token, key) = (app.clone(), collector_token.clone(), key.clone());
        let body = json!({"product":"demo","approval_hash":hash,"request":request});
        async move { signed(&app, path, &token, &key, id, body).await }
    };
    let (status, policy) = op("/collector/policy", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        policy["policy"]["features"],
        json!(["card_provenance", "publish_on_submit"])
    );
    let (status, body) = op(
        "/collector/cards",
        json!({"product":"demo","card":card("dv1","partial",true),"publish_on_submit":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["card"]["published"], true);
    assert_eq!(found(&router(&w), "member-token", "demo").await, 1);
    // Not requested, or not assistant-written: stored as a review copy.
    for request in [
        json!({"product":"demo","card":card("dv2","partial",true)}),
        json!({"product":"demo","card":card("dv3","complete",false),"publish_on_submit":true}),
    ] {
        let (status, body) = op("/collector/cards", request).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["card"]["published"], false);
    }
    // Sensitive text is refused and nothing is stored.
    let mut leaked = card("dv4", "partial", true);
    leaked["rationale"] = json!("password = CANARY");
    let (status, body) = op(
        "/collector/cards",
        json!({"product":"demo","card":leaked,"publish_on_submit":true}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!body.to_string().contains("CANARY"));
    assert_eq!(found(&router(&w), "member-token", "demo").await, 1);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn default_server_publishes_only_when_the_collector_asks() {
    let w = setup_with(AutoPublish::default()).await;
    let app = router(&w);
    // Default is on for every product, and advertised so.
    for product in ["demo", "closed"] {
        let (status, body) = get_policy(&app, product).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["policy"]["features"],
            json!(["card_provenance", "publish_on_submit"]),
            "{product}"
        );
    }
    // No flag: an ordinary review copy, even for an assistant-written card.
    for (key, flag) in [("u1", None), ("u2", Some(false))] {
        let mut body = json!({"product":"closed","card":card(key,"partial",true)});
        if let Some(f) = flag {
            body["publish_on_submit"] = json!(f);
        }
        let (status, body) = post(&app, "author-token", "/v1/cards", body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["card"]["published"], false, "{key}");
    }
    assert_eq!(found(&app, "member-token", "closed").await, 0);
    // Asked for, in a product nobody listed: published.
    let (status, body) = post(
        &app,
        "author-token",
        "/v1/cards",
        json!({"product":"closed","card":card("u3","partial",true),"publish_on_submit":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["card"]["published"], true);
    assert_eq!(found(&app, "member-token", "closed").await, 1);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn none_disables_publish_on_submit_everywhere() {
    let w = setup_with(AutoPublish::Off).await;
    let app = router(&w);
    for product in ["demo", "closed"] {
        let (status, body) = get_policy(&app, product).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["policy"]["features"], json!(["card_provenance"]));
        let (status, body) = post(
            &app,
            "author-token",
            "/v1/cards",
            json!({"product":product,"card":card("n0","partial",true),"publish_on_submit":true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["card"]["published"], false, "{product}");
        assert_eq!(found(&app, "member-token", product).await, 0);
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn a_list_enables_only_the_listed_products() {
    let w = setup_with(AutoPublish::Only(vec!["closed".into()])).await;
    let app = router(&w);
    for (product, enabled) in [("demo", false), ("closed", true)] {
        let (_, body) = get_policy(&app, product).await;
        assert_eq!(
            body["policy"]["features"].as_array().unwrap().len(),
            if enabled { 2 } else { 1 },
            "{product}"
        );
        let (_, body) = post(
            &app,
            "author-token",
            "/v1/cards",
            json!({"product":product,"card":card("l0","partial",true),"publish_on_submit":true}),
        )
        .await;
        assert_eq!(body["card"]["published"], enabled, "{product}");
    }
}
async fn get_policy(app: &axum::Router, product: &str) -> (StatusCode, Value) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/policy?product={product}"))
                .header("authorization", "Bearer author-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
