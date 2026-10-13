use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
#[test]
fn oidc_requires_signature_issuer_audience_and_expiration() {
    let key: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/synthetic-rsa-a.json")).unwrap();
    let jwks=serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":"synthetic","alg":"RS256","use":"sig","n":key["n"],"e":key["e"]}]})).unwrap();
    let verifier =
        wk_server::auth::Oidc::from_jwks("https://issuer.example".into(), "wk-api".into(), jwks)
            .unwrap();
    let pem = key["private_pkcs8_pem"].as_str().unwrap();
    let signing = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some("synthetic".into());
    let claims = json!({"iss":"https://issuer.example","aud":"wk-api","sub":"developer","exp":now+60,"nbf":now-1});
    let token = encode(&h, &claims, &signing).unwrap();
    assert_eq!(verifier.verify(&token).unwrap().1, "developer");
    for (field, value) in [
        ("aud", json!("other")),
        ("iss", json!("https://evil.example")),
        ("exp", json!(now - 1)),
        ("nbf", json!(now + 60)),
    ] {
        let mut bad = claims.clone();
        bad[field] = value;
        assert!(verifier
            .verify(&encode(&h, &bad, &signing).unwrap())
            .is_err());
    }
    let mut forged = h;
    forged.alg = Algorithm::HS256;
    assert!(verifier
        .verify(&encode(&forged, &claims, &EncodingKey::from_secret(b"synthetic")).unwrap())
        .is_err());
}

#[test]
fn browser_and_native_id_tokens_keep_strict_claim_and_signature_checks() {
    use openidconnect::{
        core::{CoreIdToken, CoreIdTokenVerifier, CoreJsonWebKeySet},
        ClientId, ClientSecret, IssuerUrl, Nonce,
    };
    let key: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/synthetic-rsa-a.json")).unwrap();
    let other: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/synthetic-rsa-b.json")).unwrap();
    let jwks: CoreJsonWebKeySet = serde_json::from_value(json!({"keys":[{
        "kty":"RSA","kid":"synthetic","alg":"RS256","use":"sig","n":key["n"],"e":key["e"]
    }]}))
    .unwrap();
    let issuer = IssuerUrl::new("https://issuer.example".into()).unwrap();
    let public = CoreIdTokenVerifier::new_public_client(
        ClientId::new("wk-login".into()),
        issuer.clone(),
        jwks.clone(),
    );
    let confidential = CoreIdTokenVerifier::new_confidential_client(
        ClientId::new("wk-login".into()),
        ClientSecret::new("synthetic-client-secret".into()),
        issuer,
        jwks,
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = json!({"iss":"https://issuer.example","aud":"wk-login","sub":"synthetic-developer","iat":now,"exp":now+300,"nonce":"synthetic-nonce"});
    let signing =
        EncodingKey::from_rsa_pem(key["private_pkcs8_pem"].as_str().unwrap().as_bytes()).unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("synthetic".into());
    for verifier in [&public, &confidential] {
        let valid: CoreIdToken = encode(&header, &claims, &signing).unwrap().parse().unwrap();
        assert_eq!(
            valid
                .claims(verifier, &Nonce::new("synthetic-nonce".into()))
                .unwrap()
                .subject()
                .as_str(),
            "synthetic-developer"
        );
        for (field, value) in [
            ("iss", json!("https://evil.example")),
            ("aud", json!("other-client")),
            ("exp", json!(0)),
            ("nonce", json!("wrong-nonce")),
        ] {
            let mut bad = claims.clone();
            bad[field] = value;
            let token: CoreIdToken = encode(&header, &bad, &signing).unwrap().parse().unwrap();
            assert!(
                token
                    .claims(verifier, &Nonce::new("synthetic-nonce".into()))
                    .is_err(),
                "accepted wrong {field}"
            );
        }
        let wrong_key =
            EncodingKey::from_rsa_pem(other["private_pkcs8_pem"].as_str().unwrap().as_bytes())
                .unwrap();
        let forged: CoreIdToken = encode(&header, &claims, &wrong_key)
            .unwrap()
            .parse()
            .unwrap();
        assert!(forged
            .claims(verifier, &Nonce::new("synthetic-nonce".into()))
            .is_err());
        let hmac: CoreIdToken = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(b"synthetic-client-secret"),
        )
        .unwrap()
        .parse()
        .unwrap();
        assert!(hmac
            .claims(verifier, &Nonce::new("synthetic-nonce".into()))
            .is_err());
    }
}
