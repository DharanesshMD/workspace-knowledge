use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_contracts::agent_setup::{rules, CLIENT_IDS};
use wk_knowledge::{Actor, Knowledge};

const RESOURCE: &str = "https://wk.example";

fn jwt(subject: &str) -> (String, wk_server::auth::Oidc) {
    let key: Value = serde_json::from_str(include_str!("fixtures/synthetic-rsa-a.json")).unwrap();
    let jwks = serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":"synthetic","alg":"RS256","use":"sig","n":key["n"],"e":key["e"]}]})).unwrap();
    let oidc =
        wk_server::auth::Oidc::from_jwks("https://issuer.example".into(), RESOURCE.into(), jwks)
            .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("synthetic".into());
    let token = encode(
        &header,
        &json!({"iss":"https://issuer.example","aud":RESOURCE,"sub":subject,"exp":now+300,"nbf":now-1}),
        &EncodingKey::from_rsa_pem(key["private_pkcs8_pem"].as_str().unwrap().as_bytes()).unwrap(),
    )
    .unwrap();
    (token, oidc)
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
fn get(uri: &str, bearer: &str) -> Request<Body> {
    let mut r = Request::builder().uri(uri);
    if !bearer.is_empty() {
        r = r.header("authorization", format!("Bearer {bearer}"));
    }
    r.body(Body::empty()).unwrap()
}
fn initialize(bearer: &str) -> Request<Body> {
    initialize_as(bearer, "t")
}
/// `initialize` from the coding client that calls itself `name`.
fn initialize_as(bearer: &str, name: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{RESOURCE}/mcp/demo"))
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":name,"version":"0"}}}).to_string()))
        .unwrap()
}

/// One test owns the process-wide `WK_MCP_RESOURCE` variable.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn members_get_the_setup_for_every_client_and_the_server_announces_the_same_rules() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let (member_subject, outsider_subject) = (
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    let member = Actor {
        org: org.clone(),
        user: "sam".into(),
    };
    let outsider = Actor {
        org: org.clone(),
        user: "outsider".into(),
    };
    k.provision_identity("https://issuer.example", &member_subject, &member)
        .await
        .unwrap();
    k.provision_identity("https://issuer.example", &outsider_subject, &outsider)
        .await
        .unwrap();
    k.grant(&member, "demo", &["backend".into()], false)
        .await
        .unwrap();
    let (token, oidc) = jwt(&member_subject);
    let (outsider_token, _) = jwt(&outsider_subject);
    let app = wk_server::configured_router(
        k.clone(),
        vec![],
        Some(oidc),
        Some("http://127.0.0.1:4318".into()),
    );
    let path = "/v1/products/demo/agent-setup";

    std::env::set_var("WK_MCP_RESOURCE", RESOURCE);
    let (status, body) = send(&app, get(path, &token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mcp_url"], "https://wk.example/mcp/demo");
    assert_eq!(body["token_env"], "WK_TOKEN");
    assert_eq!(body["rules"], rules(false));
    let ids: Vec<&str> = body["clients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, CLIENT_IDS);
    assert!(body.to_string().contains("https://wk.example/mcp/demo"));
    assert!(
        !body.to_string().contains("wkpt_"),
        "the endpoint never carries a token"
    );
    let codex = body["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "codex")
        .unwrap();
    assert!(codex["files"][0]["content"]
        .as_str()
        .unwrap()
        .contains("bearer_token_env_var = \"WK_TOKEN\""));

    // Automatic sharing is a choice the caller makes explicitly.
    let (_, auto) = send(&app, get(&format!("{path}?auto_share=true"), &token)).await;
    assert_eq!(auto["rules"], rules(true));
    assert_ne!(auto["rules"], body["rules"]);
    // Refusals: no sign-in, a user outside the product, a parameter that does not exist, another product.
    assert_eq!(send(&app, get(path, "")).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        send(&app, get(path, &outsider_token)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, get(&format!("{path}?client=cursor"), &token))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        send(&app, get("/v1/products/other/agent-setup", &token))
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // The hosted MCP server tells every connecting client the same standing instructions.
    let (status, init) = send(&app, initialize(&token)).await;
    assert_eq!(status, StatusCode::OK, "{init}");
    assert_eq!(init["result"]["instructions"], rules(false));
    // A client known to keep a project rule file is also told where to save them.
    let (status, init) = send(&app, initialize_as(&token, "cursor-vscode")).await;
    assert_eq!(status, StatusCode::OK, "{init}");
    let told = init["result"]["instructions"].as_str().unwrap();
    assert!(told.starts_with(&rules(false)), "{told}");
    assert!(
        told.contains(".cursor/rules/workspace-knowledge.mdc"),
        "{told}"
    );

    // Without a configured public address the product still answers, with nothing to connect to yet.
    std::env::remove_var("WK_MCP_RESOURCE");
    let (status, none) = send(&app, get(path, &token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (none["mcp_url"].clone(), none["clients"].clone()),
        (Value::Null, json!([]))
    );
}
