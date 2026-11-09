use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_knowledge::{Actor, Knowledge};
use wk_server::personal_tokens::{config, generate, looks_like_token, Config};

const RESOURCE: &str = "https://wk.example";

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl FnMut(&str) -> Option<String> + 'a {
    move |key| {
        pairs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_string())
    }
}

#[test]
fn settings_are_off_by_default_and_malformed_values_fail_startup() {
    assert_eq!(config(env(&[])), Ok(None));
    assert_eq!(config(env(&[("WK_PERSONAL_TOKENS", "0")])), Ok(None));
    assert_eq!(config(env(&[("WK_PERSONAL_TOKENS", " ")])), Ok(None));
    // A lifetime cap alone does not switch the feature on.
    assert_eq!(
        config(env(&[("WK_PERSONAL_TOKEN_MAX_DAYS", "30")])),
        Ok(None)
    );
    assert_eq!(
        config(env(&[("WK_PERSONAL_TOKENS", "1")])),
        Ok(Some(Config { max_days: 90 }))
    );
    assert_eq!(
        config(env(&[
            ("WK_PERSONAL_TOKENS", "1"),
            ("WK_PERSONAL_TOKEN_MAX_DAYS", "30")
        ])),
        Ok(Some(Config { max_days: 30 }))
    );
    for bad in ["yes", "true", "2", "on"] {
        assert!(
            config(env(&[("WK_PERSONAL_TOKENS", bad)])).is_err(),
            "{bad}"
        );
    }
    for bad in ["0", "366", "-1", "ninety", "1.5"] {
        assert!(
            config(env(&[
                ("WK_PERSONAL_TOKENS", "1"),
                ("WK_PERSONAL_TOKEN_MAX_DAYS", bad)
            ]))
            .is_err(),
            "{bad}"
        );
    }
}

#[test]
fn generated_tokens_are_recognizable_unique_and_hashed() {
    let (a, hash_a) = generate();
    let (b, _) = generate();
    assert_ne!(a, b);
    assert!(a.starts_with("wkpt_") && a.len() == 69 && looks_like_token(&a));
    assert_eq!(hash_a.len(), 64);
    assert!(!hash_a.contains(&a[5..]));
    for not_ours in [
        "".to_string(),
        "wkpt_".into(),
        "wkpt_abc".into(),
        a.to_uppercase(),
        format!("{a}0"),
        format!("xkpt_{}", &a[5..]),
        format!("wkpt_{}z", &a[5..68]),
    ] {
        assert!(!looks_like_token(&not_ours), "{not_ours}");
    }
}

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
fn router(k: &Knowledge, oidc: wk_server::auth::Oidc) -> axum::Router {
    wk_server::configured_router(
        k.clone(),
        vec![],
        Some(oidc),
        Some("http://127.0.0.1:4318".into()),
    )
}
async fn send(
    app: &axum::Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let r = app.clone().oneshot(request).await.unwrap();
    let (status, headers) = (r.status(), r.headers().clone());
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    // MCP replies as JSON here, but tolerate an event stream.
    let body = text
        .lines()
        .find_map(|l| l.strip_prefix("data:").map(str::trim))
        .map(|d| serde_json::from_str(d).unwrap_or(Value::Null))
        .or_else(|| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    (status, headers, body)
}
fn mcp(product: &str, bearer: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{RESOURCE}/mcp/{product}"))
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-06-18")
        .body(Body::from(body.to_string()))
        .unwrap()
}
fn get(uri: &str, bearer: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {bearer}"))
        .body(Body::empty())
        .unwrap()
}
fn search() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge.search","arguments":{"query":"idempotency"}}})
}

/// One test owns the process-wide `WK_MCP_RESOURCE` variable, so scenarios run in sequence.
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn a_personal_token_opens_only_its_own_products_mcp_endpoint() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let anika = Actor {
        org: org.clone(),
        user: "anika".into(),
    };
    let subject = uuid::Uuid::new_v4().to_string();
    k.provision_identity("https://issuer.example", &subject, &anika)
        .await
        .unwrap();
    for product in ["demo", "other"] {
        k.grant(&anika, product, &["backend".into()], true)
            .await
            .unwrap();
    }
    let (identity_token, oidc) = jwt(&subject);
    std::env::set_var("WK_MCP_RESOURCE", RESOURCE);

    std::env::remove_var("WK_PERSONAL_TOKENS");
    let off = router(&k, oidc.clone());
    std::env::set_var("WK_PERSONAL_TOKENS", "1");
    std::env::set_var("WK_PERSONAL_TOKEN_MAX_DAYS", "60");
    let on = router(&k, oidc);
    std::env::remove_var("WK_PERSONAL_TOKENS");
    std::env::remove_var("WK_PERSONAL_TOKEN_MAX_DAYS");

    let (token, hash) = generate();
    let made = k
        .create_personal_token(&anika, "demo", "laptop", &hash, 30)
        .await
        .unwrap();

    // Works on its own product's MCP endpoint, as the owner, for a real tool call.
    let (status, _, body) = send(&on, mcp("demo", &token, search())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["isError"], json!(false), "{body}");

    // Nowhere else: another product, any REST route, the user-info route.
    let (status, _, _) = send(&on, mcp("other", &token, search())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    for uri in [
        "/v1/me",
        "/v1/policy?product=demo",
        "/v1/products/demo/review",
    ] {
        let (status, _, _) = send(&on, get(uri, &token)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
    // A token cannot create, list or revoke tokens; only a portal session can.
    for request in [
        Request::builder()
            .method("POST")
            .uri("/v1/tokens")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"product":"demo","label":"x","days":1}).to_string(),
            ))
            .unwrap(),
        get("/v1/tokens", &token),
        Request::builder()
            .method("DELETE")
            .uri(format!("/v1/tokens/{}", made.id))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
        // Not even the owner's short-lived identity token may mint a long-lived one.
        get("/v1/tokens", &identity_token),
    ] {
        let (status, _, body) = send(&on, request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"], "session_required");
    }
    assert!(
        k.personal_token_actor(&hash, "demo").await.is_ok(),
        "still valid after those attempts"
    );

    // The owner's ordinary sign-in keeps working, and /v1/me tells the portal the limits.
    let (status, _, body) = send(&on, get("/v1/me", &identity_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["personal_tokens"], json!({"max_days": 60}));
    let (status, _, body) = send(&off, get("/v1/me", &identity_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["personal_tokens"], Value::Null);
    let (status, _, _) = send(&on, mcp("demo", &identity_token, search())).await;
    assert_eq!(status, StatusCode::OK);

    // A browser origin is refused even with a valid token.
    let mut request = mcp("demo", &token, search());
    request
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    assert_eq!(send(&on, request).await.0, StatusCode::FORBIDDEN);

    // Unknown and malformed tokens are the same plain 401, with the sign-in pointer for OAuth clients.
    let (unknown, _) = generate();
    for bearer in [unknown.as_str(), "wkpt_nope", "plainly-not-a-token"] {
        let (status, headers, _) = send(&on, mcp("demo", bearer, search())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{bearer}");
        assert!(headers["www-authenticate"]
            .to_str()
            .unwrap()
            .contains("resource_metadata"));
    }

    // Feature off: the same valid token is simply not accepted, and the management routes do not exist.
    let (status, _, _) = send(&off, mcp("demo", &token, search())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = send(&off, get("/v1/tokens", &identity_token)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Admin routes: /v1/me says who is an admin, and no bearer credential reaches the routes at all.
    k.admin_assign(&anika, "demo", &["backend".into()])
        .await
        .unwrap();
    let ravi = Actor {
        org: org.clone(),
        user: "ravi".into(),
    };
    let ravi_subject = uuid::Uuid::new_v4().to_string();
    k.provision_identity("https://issuer.example", &ravi_subject, &ravi)
        .await
        .unwrap();
    k.grant(&ravi, "demo", &["backend".into()], false)
        .await
        .unwrap();
    let (ravi_identity, _) = jwt(&ravi_subject);
    let is_admin = |body: &Value, product: &str| {
        body["products"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["product"] == product)
            .unwrap()["is_admin"]
            .clone()
    };
    let (_, _, me) = send(&on, get("/v1/me", &identity_token)).await;
    assert_eq!(
        (is_admin(&me, "demo"), is_admin(&me, "other")),
        (json!(true), json!(false))
    );
    let (_, _, me) = send(&on, get("/v1/me", &ravi_identity)).await;
    assert_eq!(is_admin(&me, "demo"), json!(false));
    let admin_path = "/v1/admin/products/demo/users";
    let body = json!({"user":"newdev","services":["backend"],"days":30}).to_string();
    for bearer in [
        identity_token.as_str(),
        token.as_str(),
        ravi_identity.as_str(),
    ] {
        let with = |method: &str, uri: &str| {
            Request::builder()
                .method(method)
                .uri(uri.to_string())
                .header("authorization", format!("Bearer {bearer}"))
                .header("content-type", "application/json")
                .body(Body::from(body.clone()))
                .unwrap()
        };
        for request in [
            with("GET", admin_path),
            with("POST", admin_path),
            with("POST", &format!("{admin_path}/newdev/token")),
            with("DELETE", &format!("{admin_path}/newdev")),
        ] {
            let (status, _, reply) = send(&on, request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{reply}");
            assert_eq!(reply["error"], "session_required");
        }
    }
    // With the feature off the routes do not exist.
    assert_eq!(
        send(&off, get(admin_path, &identity_token)).await.0,
        StatusCode::NOT_FOUND
    );
    // Nothing was created by any of those attempts.
    assert!(k
        .admin_users(&anika, "demo")
        .await
        .unwrap()
        .users
        .iter()
        .all(|u| u.user != "newdev"));

    // Revocation takes effect on the next request.
    k.revoke_personal_token(&anika, made.id).await.unwrap();
    let (status, _, _) = send(&on, mcp("demo", &token, search())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
