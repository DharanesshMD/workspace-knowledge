use serde_json::json;
use wk_server::admin_cli::{parse, AdminRequest};
#[test]
fn administrative_input_is_strict_bounded_and_contains_no_actor_argument() {
    let request = json!({"token":"synthetic","product":"demo","command":{"kind":"members"}});
    let _: AdminRequest = parse(request.to_string().as_bytes()).unwrap();
    for bad in [
        json!({"token":"synthetic","product":"demo","actor":{"org":"evil","user":"admin"},"command":{"kind":"members"}}),
        json!({"token":"synthetic","product":"demo","command":{"kind":"shell","text":"ignored"}}),
        json!({"token":"synthetic","product":"demo","command":{"kind":"members","extra":true}}),
        json!({"product":"demo","command":{"kind":"members"}}),
    ] {
        assert!(parse::<AdminRequest, _>(bad.to_string().as_bytes()).is_err());
    }
    assert!(parse::<AdminRequest, _>(vec![b' '; 65537].as_slice()).is_err());
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn administrative_command_requires_ordinary_upstream_token_and_explicit_role() {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use wk_knowledge::{Actor, Error, Knowledge};
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let actor = Actor {
        org: org.clone(),
        user: "admin".into(),
    };
    let issuer = format!("https://{org}.synthetic.example");
    k.provision_identity(&issuer, "mapped-admin", &actor)
        .await
        .unwrap();
    k.grant(&actor, "demo", &["backend".into()], false)
        .await
        .unwrap();
    let key: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/synthetic-rsa-a.json")).unwrap();
    let jwks=serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":"synthetic","alg":"RS256","use":"sig","n":key["n"],"e":key["e"]}]})).unwrap();
    let oidc = wk_server::auth::Oidc::from_jwks(issuer.clone(), "wk-api".into(), jwks).unwrap();
    let signer =
        EncodingKey::from_rsa_pem(key["private_pkcs8_pem"].as_str().unwrap().as_bytes()).unwrap();
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some("synthetic".into());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = |aud: &str| {
        encode(
            &h,
            &json!({"iss":issuer,"aud":aud,"sub":"mapped-admin","exp":now+120}),
            &signer,
        )
        .unwrap()
    };
    let command = |token: &str| {
        parse::<AdminRequest, _>(
            json!({"token":token,"product":"demo","command":{"kind":"members"}})
                .to_string()
                .as_bytes(),
        )
        .unwrap()
    };
    for bad in [
        token("wk-collector"),
        token("wrong"),
        "invalid-token".into(),
    ] {
        let error = wk_server::admin_cli::execute(&k, &oidc, command(&bad))
            .await
            .unwrap_err();
        assert_eq!(error, "unauthenticated");
        assert!(!error.contains(&bad));
    }
    assert_eq!(
        wk_server::admin_cli::execute(&k, &oidc, command(&token("wk-api")))
            .await
            .unwrap_err(),
        Error::Denied.to_string()
    );
    k.admin_assign(&actor, "demo", &["backend".into()])
        .await
        .unwrap();
    let result = wk_server::admin_cli::execute(&k, &oidc, command(&token("wk-api")))
        .await
        .unwrap();
    assert_eq!(result["members"][0]["user"], "admin");
    assert!(!result.to_string().contains("token"));
}
