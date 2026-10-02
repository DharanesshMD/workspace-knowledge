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
