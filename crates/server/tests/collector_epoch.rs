use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use ed25519_dalek::{Signer, SigningKey};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
use tower::ServiceExt;
use wk_knowledge::{Actor, Knowledge};
async fn register(app: &axum::Router, token: &str, key: &SigningKey) -> StatusCode {
    let id = uuid::Uuid::new_v4();
    let nonce = uuid::Uuid::new_v4();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let body=serde_json::to_vec(&json!({"id":id,"product":"demo","services":["backend"],"public_key":hex::encode(key.verifying_key().to_bytes()),"approval_hash":"22".repeat(32)})).unwrap();
    let signature = key.sign(&wk_contracts::device::signing_bytes(
        "http://127.0.0.1:4318",
        "POST",
        "/collector/register",
        now,
        nonce,
        id,
        token,
        &body,
    ));
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collector/register")
                .header("authorization", format!("Bearer {token}"))
                .header("x-wk-device", id.to_string())
                .header("x-wk-time", now.to_string())
                .header("x-wk-nonce", nonce.to_string())
                .header("x-wk-signature", hex::encode(signature.to_bytes()))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn restored_auth_cutoff_blocks_unseen_old_credentials_and_requires_fresh_sign_in() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let owner = sqlx::PgPool::connect(&url).await.unwrap();
    let name = format!("wk_epoch_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&owner)
        .await
        .unwrap();
    let mut testurl = reqwest::Url::parse(&url).unwrap();
    testurl.set_path(&format!("/{name}"));
    let pool = sqlx::PgPool::connect(testurl.as_str()).await.unwrap();
    let k = Knowledge::from_pool(pool.clone());
    k.migrate().await.unwrap();
    let a = Actor {
        org: "synthetic".into(),
        user: "alice".into(),
    };
    k.provision_identity("https://synthetic.example", "alice", &a)
        .await
        .unwrap();
    k.grant(&a, "demo", &["backend".into()], false)
        .await
        .unwrap();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/synthetic-rsa-a.json")).unwrap();
    let keys=serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":"synthetic","alg":"RS256","use":"sig","n":fixture["n"],"e":fixture["e"]}]})).unwrap();
    let ordinary =
        wk_server::auth::Oidc::from_jwks("https://synthetic.example".into(), "wk-api".into(), keys)
            .unwrap();
    let collector=wk_server::auth::Oidc::from_jwks("https://synthetic.example".into(),"wk-collector".into(),serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":"synthetic","alg":"RS256","use":"sig","n":fixture["n"],"e":fixture["e"]}]})).unwrap()).unwrap();
    let signing =
        EncodingKey::from_rsa_pem(fixture["private_pkcs8_pem"].as_str().unwrap().as_bytes())
            .unwrap();
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some("synthetic".into());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = |aud: &str, issued: Option<u64>| {
        let mut claims =
            json!({"iss":"https://synthetic.example","sub":"alice","aud":aud,"exp":now+300});
        if let Some(at) = issued {
            claims["iat"] = json!(at)
        }
        encode(&h, &claims, &signing).unwrap()
    };
    let app = wk_server::portal_router_with_collector(
        k.clone(),
        vec![],
        Some(ordinary),
        Some("http://127.0.0.1:4318".into()),
        None,
        Some(wk_server::device_routes::CollectorAuth {
            origin: "http://127.0.0.1:4318".into(),
            oidc: Some(collector),
            tokens: vec![],
            client_id: Some("synthetic-native".into()),
        }),
    )
    .unwrap();
    assert_eq!(
        register(
            &app,
            &token("wk-collector", Some(now - 30)),
            &SigningKey::from_bytes(&[41; 32])
        )
        .await,
        StatusCode::OK
    );
    let cutoff = k.collector_reset_auth().await.unwrap();
    assert_eq!(
        register(
            &app,
            &token("wk-collector", Some(now - 20)),
            &SigningKey::from_bytes(&[42; 32])
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        register(
            &app,
            &token("wk-collector", None),
            &SigningKey::from_bytes(&[43; 32])
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert!(cutoff >= now + 60);
    assert_eq!(
        register(
            &app,
            &token("wk-collector", Some(now + 60)),
            &SigningKey::from_bytes(&[45; 32])
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    // Advance only this isolated test fixture's cutoff to model the completed cooldown.
    sqlx::query("UPDATE collector_security_epoch SET not_before=$1")
        .bind(now as i64 - 10)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        register(
            &app,
            &token("wk-collector", Some(now)),
            &SigningKey::from_bytes(&[44; 32])
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/me")
                    .header(
                        "authorization",
                        format!("Bearer {}", token("wk-api", Some(now - 30)))
                    )
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    drop(app);
    drop(k);
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&owner)
        .await
        .unwrap();
}
#[tokio::test]
async fn collector_storage_outage_is_temporary_instead_of_authorization_denial() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://127.0.0.1:1/wk_test")
        .unwrap();
    pool.close().await;
    let k = Knowledge::from_pool(pool);
    let a = Actor {
        org: "synthetic".into(),
        user: "alice".into(),
    };
    let app = wk_server::portal_router_with_collector(
        k,
        vec![("ordinary".into(), a.clone())],
        None,
        Some("http://127.0.0.1:4318".into()),
        None,
        Some(wk_server::device_routes::CollectorAuth {
            origin: "http://127.0.0.1:4318".into(),
            oidc: None,
            tokens: vec![("collector".into(), a)],
            client_id: None,
        }),
    )
    .unwrap();
    let id = uuid::Uuid::new_v4();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let r = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/collector/policy")
                .header("authorization", "Bearer collector")
                .header("x-wk-device", id.to_string())
                .header("x-wk-time", now.to_string())
                .header("x-wk-nonce", uuid::Uuid::new_v4().to_string())
                .header("x-wk-signature", "00".repeat(64))
                .body(Body::from(
                    json!({"product":"demo","approval_hash":"22".repeat(32),"request":{}})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
}
