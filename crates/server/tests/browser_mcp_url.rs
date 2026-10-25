use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_knowledge::{Actor, Knowledge};

const RESOURCE: &str = "https://wk.example";

async fn me(app: &axum::Router, token: &str) -> (StatusCode, Value) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/me")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
fn urls(body: &Value) -> Vec<(String, Value)> {
    body["products"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["product"].as_str().unwrap().to_string(),
                p["mcp_url"].clone(),
            )
        })
        .collect()
}

/// One test owns the process-wide `WK_MCP_RESOURCE` variable, so scenarios run in sequence.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn products_carry_the_hosted_mcp_url_only_when_the_resource_is_valid() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let actor = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "developer".into(),
    };
    let subject = uuid::Uuid::new_v4().to_string();
    k.provision_identity("https://issuer.example", &subject, &actor)
        .await
        .unwrap();
    for product in ["beta", "alpha", "with_under-score"] {
        k.grant(&actor, product, &["backend".into()], false)
            .await
            .unwrap();
    }
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
    let app = wk_server::configured_router(
        k.clone(),
        vec![],
        Some(oidc),
        Some("http://127.0.0.1:4318".into()),
    );

    // Unset: every product is still listed, with a null address.
    std::env::remove_var("WK_MCP_RESOURCE");
    let (status, body) = me(&app, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        urls(&body),
        vec![
            ("alpha".into(), Value::Null),
            ("beta".into(), Value::Null),
            ("with_under-score".into(), Value::Null)
        ]
    );
    assert_eq!(body["products"][0]["services"], json!(["backend"]));

    // Configured and equal to the OIDC audience: one address per product, the other fields unchanged.
    std::env::set_var("WK_MCP_RESOURCE", RESOURCE);
    let (status, body) = me(&app, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        urls(&body),
        vec![
            ("alpha".into(), json!("https://wk.example/mcp/alpha")),
            ("beta".into(), json!("https://wk.example/mcp/beta")),
            (
                "with_under-score".into(),
                json!("https://wk.example/mcp/with_under-score")
            )
        ]
    );
    assert_eq!(body["user"], "developer");
    assert_eq!(body["products"][1]["can_review"], false);
    assert!(body["products"][1]["policy_revision"].is_i64());

    // Invalid values never fail the request and never produce an address.
    for invalid in [
        "",
        "not a url",
        "http://wk.example",
        "https://wk.example/",
        "https://wk.example/mcp",
        "https://other.example",
    ] {
        std::env::set_var("WK_MCP_RESOURCE", invalid);
        let (status, body) = me(&app, &token).await;
        assert_eq!(status, StatusCode::OK, "{invalid:?}");
        assert!(
            urls(&body).iter().all(|(_, url)| url.is_null()) && urls(&body).len() == 3,
            "{invalid:?}"
        );
    }

    // Without OIDC the resource cannot be validated, so it is not advertised either.
    std::env::set_var("WK_MCP_RESOURCE", RESOURCE);
    let static_app = wk_server::configured_router(
        k,
        vec![("static-token".into(), actor)],
        None,
        Some("http://127.0.0.1:4318".into()),
    );
    let (status, body) = me(&static_app, "static-token").await;
    assert_eq!(status, StatusCode::OK);
    assert!(urls(&body).iter().all(|(_, url)| url.is_null()) && urls(&body).len() == 3);
    std::env::remove_var("WK_MCP_RESOURCE");
}
