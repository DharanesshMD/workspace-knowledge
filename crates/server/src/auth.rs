//! Fixed issuer/audience/key source. Never follows token-controlled jku/x5u URLs.
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
const KEY_MAX_AGE: Duration = Duration::from_secs(300);
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
struct Snapshot {
    keys: JwkSet,
    expires: Instant,
}
#[derive(Debug)]
pub struct InvalidToken;
#[derive(Clone)]
pub struct Oidc {
    issuer: String,
    audience: String,
    keys: Arc<RwLock<Snapshot>>,
    source: Option<reqwest::Url>,
}
#[derive(Clone, Deserialize)]
struct Claims {
    exp: u64,
    #[serde(default)]
    iat: Option<u64>,
    sub: String,
    aud: serde_json::Value,
}
impl Oidc {
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
    pub fn audience(&self) -> &str {
        &self.audience
    }
    pub fn from_jwks(issuer: String, audience: String, keys: JwkSet) -> Result<Self, String> {
        Self::from_keys(issuer, audience, keys, false)
    }
    fn from_keys(
        issuer: String,
        audience: String,
        keys: JwkSet,
        development: bool,
    ) -> Result<Self, String> {
        let parsed = reqwest::Url::parse(&issuer).map_err(|_| "invalid issuer URL")?;
        if (parsed.scheme() != "https" && !development)
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || audience.is_empty()
        {
            return Err("invalid OIDC configuration".into());
        }
        Self::validate_keys(&keys)?;
        Ok(Self {
            issuer,
            audience,
            keys: Arc::new(RwLock::new(Snapshot {
                keys,
                expires: Instant::now() + KEY_MAX_AGE,
            })),
            source: None,
        })
    }
    pub async fn load(issuer: String, audience: String, url: String) -> Result<Self, String> {
        let parsed = reqwest::Url::parse(&url).map_err(|_| "invalid JWKS URL")?;
        if parsed.scheme() != "https"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err("JWKS requires HTTPS without URL credentials".into());
        }
        let keys = Self::fetch_keys(parsed.clone()).await?;
        let mut verifier = Self::from_jwks(issuer, audience, keys)?;
        verifier.source = Some(parsed);
        Ok(verifier)
    }
    /// Explicit synthetic fixture loader. Ordinary production constructors remain HTTPS-only.
    pub async fn load_for_development(
        issuer: String,
        audience: String,
        url: String,
    ) -> Result<Self, String> {
        let parsed = reqwest::Url::parse(&url).map_err(|_| "invalid JWKS URL")?;
        for value in [&issuer, &url] {
            let u = reqwest::Url::parse(value).map_err(|_| "invalid test OIDC URL")?;
            if u.scheme() != "http"
                || !matches!(u.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))
                || !u.username().is_empty()
                || u.password().is_some()
                || u.query().is_some()
                || u.fragment().is_some()
            {
                return Err("test OIDC requires literal loopback URLs".into());
            }
        }
        let keys = Self::fetch_keys(parsed.clone()).await?;
        let mut verifier = Self::from_keys(issuer, audience, keys, true)?;
        verifier.source = Some(parsed);
        Ok(verifier)
    }
    fn validate_keys(keys: &JwkSet) -> Result<(), String> {
        if keys.keys.is_empty() || keys.keys.len() > 128 {
            return Err("invalid JWKS key count".into());
        }
        let mut ids = std::collections::HashSet::new();
        for key in &keys.keys {
            let id = key.common.key_id.as_deref().ok_or("JWKS key missing ID")?;
            if id.is_empty() || id.len() > 256 || !ids.insert(id) {
                return Err("invalid or duplicate JWKS key ID".into());
            }
        }
        Ok(())
    }
    fn replace_keys(&self, keys: JwkSet) -> Result<(), String> {
        Self::validate_keys(&keys)?;
        let mut snapshot = self.keys.write().map_err(|_| "JWKS cache unavailable")?;
        *snapshot = Snapshot {
            keys,
            expires: Instant::now() + KEY_MAX_AGE,
        };
        Ok(())
    }
    /// Called by one scheduled worker, never triggered by an untrusted token.
    /// Failure leaves the last good snapshot usable only until its fixed expiry.
    pub async fn refresh(&self) -> Result<(), String> {
        let url = self.source.clone().ok_or("no configured JWKS source")?;
        self.replace_keys(Self::fetch_keys(url).await?)
    }
    async fn fetch_keys(parsed: reqwest::Url) -> Result<JwkSet, String> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| "JWKS client unavailable")?;
        let mut response = client
            .get(parsed)
            .send()
            .await
            .map_err(|_| "JWKS unavailable")?
            .error_for_status()
            .map_err(|_| "JWKS unavailable")?;
        if response.content_length().unwrap_or(0) > 262144 {
            return Err("JWKS too large".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "JWKS unavailable")? {
            if bytes.len() + chunk.len() > 262144 {
                return Err("JWKS too large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let keys = serde_json::from_slice(&bytes).map_err(|_| "invalid JWKS")?;
        Self::validate_keys(&keys)?;
        Ok(keys)
    }
    pub fn verify(&self, token: &str) -> Result<(&str, String), InvalidToken> {
        self.verify_credential(token)
            .map(|(issuer, subject, _, _)| (issuer, subject))
    }
    pub fn verify_credential(
        &self,
        token: &str,
    ) -> Result<(&str, String, u64, Option<u64>), InvalidToken> {
        if token.len() > 16384 {
            return Err(InvalidToken);
        }
        let h = decode_header(token).map_err(|_| InvalidToken)?;
        if !matches!(h.alg, Algorithm::RS256 | Algorithm::ES256) {
            return Err(InvalidToken);
        }
        let snapshot = self.keys.read().map_err(|_| InvalidToken)?;
        if Instant::now() >= snapshot.expires {
            return Err(InvalidToken);
        }
        let jwk = snapshot
            .keys
            .find(h.kid.as_deref().ok_or(InvalidToken)?)
            .ok_or(InvalidToken)?;
        use jsonwebtoken::jwk::{KeyAlgorithm, KeyOperations, PublicKeyUse};
        let expected = match h.alg {
            Algorithm::RS256 => KeyAlgorithm::RS256,
            _ => KeyAlgorithm::ES256,
        };
        if jwk.common.key_algorithm.is_some_and(|alg| alg != expected)
            || jwk
                .common
                .public_key_use
                .as_ref()
                .is_some_and(|usage| *usage != PublicKeyUse::Signature)
            || jwk
                .common
                .key_operations
                .as_ref()
                .is_some_and(|ops| !ops.contains(&KeyOperations::Verify))
        {
            return Err(InvalidToken);
        }
        let key = DecodingKey::from_jwk(jwk).map_err(|_| InvalidToken)?;
        let mut v = Validation::new(h.alg);
        v.set_issuer(&[&self.issuer]);
        v.set_audience(&[&self.audience]);
        v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        v.validate_nbf = true;
        v.leeway = 0;
        let c = decode::<Claims>(token, &key, &v)
            .map_err(|_| InvalidToken)?
            .claims;
        let single_audience = c.aud.as_str() == Some(self.audience.as_str())
            || c.aud
                .as_array()
                .is_some_and(|v| v.len() == 1 && v[0].as_str() == Some(self.audience.as_str()));
        if !single_audience || c.sub.is_empty() || c.sub.len() > 512 {
            return Err(InvalidToken);
        }
        Ok((&self.issuer, c.sub, c.exp, c.iat))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;
    fn fixture(id: &str) -> (JwkSet, String) {
        // Fixed synthetic keys: "new" uses a second pair so rotation is observable.
        let raw = if id == "new" {
            include_str!("../tests/fixtures/synthetic-rsa-b.json")
        } else {
            include_str!("../tests/fixtures/synthetic-rsa-a.json")
        };
        let key: serde_json::Value = serde_json::from_str(raw).unwrap();
        let jwks = serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":id,"alg":"RS256","use":"sig","n":key["n"],"e":key["e"]}]})).unwrap();
        let pem = key["private_pkcs8_pem"].as_str().unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(id.into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = encode(
            &header,
            &json!({"iss":"https://issuer.example","aud":"wk-api","sub":"developer","exp":now+600}),
            &EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap(),
        )
        .unwrap();
        (jwks, token)
    }
    #[tokio::test]
    async fn rotation_replaces_keys_for_all_clones_and_expired_cache_fails_closed() {
        let (old, old_token) = fixture("old");
        let (new, new_token) = fixture("new");
        let mut verifier =
            Oidc::from_jwks("https://issuer.example".into(), "wk-api".into(), old).unwrap();
        let sibling = verifier.clone();
        assert!(sibling.verify(&old_token).is_ok());
        assert!(sibling.verify(&new_token).is_err());
        // HTTP is injected only inside this module; production load requires HTTPS.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .route(
                "/keys",
                axum::routing::get(move || {
                    let keys = new.clone();
                    async move { axum::Json(keys) }
                }),
            )
            .route(
                "/bad",
                axum::routing::get(|| async { axum::Json(json!({"keys":[]})) }),
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        verifier.source = Some(format!("http://{address}/keys").parse().unwrap());
        verifier.refresh().await.unwrap();
        assert!(sibling.verify(&new_token).is_ok());
        assert!(sibling.verify(&old_token).is_err());
        let deadline = verifier.keys.read().unwrap().expires;
        verifier.source = Some(format!("http://{address}/bad").parse().unwrap());
        assert!(verifier.refresh().await.is_err());
        assert_eq!(verifier.keys.read().unwrap().expires, deadline);
        assert!(sibling.verify(&new_token).is_ok());
        verifier.keys.write().unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert!(sibling.verify(&new_token).is_err());
        assert!(verifier.refresh().await.is_err());
        assert!(sibling.verify(&new_token).is_err());
        verifier.source = Some(format!("http://{address}/keys").parse().unwrap());
        verifier.refresh().await.unwrap();
        assert!(sibling.verify(&new_token).is_ok());
        server.abort();
    }
    #[test]
    fn tokens_with_another_audience_cannot_cross_api_roles() {
        let (keys, _) = fixture("old");
        let verifier =
            Oidc::from_jwks("https://issuer.example".into(), "wk-api".into(), keys).unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/synthetic-rsa-a.json")).unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("old".into());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token=encode(&header,&json!({"iss":"https://issuer.example","aud":["wk-api","wk-collector"],"sub":"developer","exp":now+600}),&EncodingKey::from_rsa_pem(raw["private_pkcs8_pem"].as_str().unwrap().as_bytes()).unwrap()).unwrap();
        assert!(verifier.verify(&token).is_err());
    }
    #[test]
    fn duplicate_ids_and_wrong_key_purpose_are_rejected() {
        let (keys, token) = fixture("key");
        let mut duplicates = keys.clone();
        duplicates.keys.push(keys.keys[0].clone());
        assert!(
            Oidc::from_jwks("https://issuer.example".into(), "wk-api".into(), duplicates).is_err()
        );
        for (field, value) in [
            ("use", json!("enc")),
            ("alg", json!("RS512")),
            ("key_ops", json!(["sign"])),
        ] {
            let mut altered = serde_json::to_value(&keys).unwrap();
            altered["keys"][0][field] = value;
            let verifier = Oidc::from_jwks(
                "https://issuer.example".into(),
                "wk-api".into(),
                serde_json::from_value(altered).unwrap(),
            )
            .unwrap();
            assert!(verifier.verify(&token).is_err());
        }
    }
    #[tokio::test]
    async fn development_loader_accepts_only_literal_loopback_and_keeps_production_strict() {
        let (keys, _) = fixture("old");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/keys",
            axum::routing::get(move || {
                let keys = keys.clone();
                async move { axum::Json(keys) }
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let issuer = format!("http://{address}");
        assert!(Oidc::load_for_development(
            issuer.clone(),
            "wk-collector".into(),
            format!("{issuer}/keys")
        )
        .await
        .is_ok());
        assert!(Oidc::load(
            issuer.clone(),
            "wk-collector".into(),
            format!("{issuer}/keys")
        )
        .await
        .is_err());
        for value in [
            "http://localhost:1234",
            "http://example.org",
            "http://user@127.0.0.1",
            "https://example.org",
            "http://127.0.0.1?query=1",
        ] {
            assert!(Oidc::load_for_development(
                value.into(),
                "wk-collector".into(),
                format!("{issuer}/keys")
            )
            .await
            .is_err());
            assert!(Oidc::load_for_development(
                issuer.clone(),
                "wk-collector".into(),
                format!("{value}/keys")
            )
            .await
            .is_err());
        }
        server.abort();
    }
    #[tokio::test]
    async fn production_disallows_insecure_or_credentialed_key_urls() {
        for url in [
            "http://127.0.0.1/keys",
            "https://user:password@example.org/keys",
            "https://example.org/keys#fragment",
        ] {
            assert!(
                Oidc::load("https://issuer.example".into(), "wk-api".into(), url.into())
                    .await
                    .is_err()
            );
        }
    }
}
