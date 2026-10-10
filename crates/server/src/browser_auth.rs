//! Fixed-provider browser login. No upstream token survives a successful callback.
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreProviderMetadata},
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, TokenResponse,
};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}
pub fn validated_url(value: &str, development: bool) -> Result<reqwest::Url, String> {
    let u = reqwest::Url::parse(value).map_err(|_| "invalid_browser_url")?;
    let loopback = matches!(u.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
    if !(u.scheme() == "https" || (development && u.scheme() == "http" && loopback))
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err("invalid_browser_url".into());
    }
    Ok(u)
}
pub fn validated_origin(value: &str, development: bool) -> Result<String, String> {
    let u = validated_url(value, development)?;
    if u.origin().ascii_serialization() != value
        || (development && !matches!(u.host_str(), Some("127.0.0.1" | "[::1]" | "::1")))
    {
        return Err("invalid_browser_origin".into());
    }
    Ok(value.into())
}
struct Transaction {
    state: String,
    cookie: String,
    nonce: String,
    verifier: String,
    created: Instant,
}
fn transaction() -> Transaction {
    let (_, verifier) = PkceCodeChallenge::new_random_sha256();
    Transaction {
        state: CsrfToken::new_random().secret().clone(),
        cookie: random_secret(),
        nonce: Nonce::new_random().secret().clone(),
        verifier: verifier.secret().clone(),
        created: Instant::now(),
    }
}
#[derive(Default)]
struct Pending(HashMap<String, Transaction>);
impl Pending {
    fn insert(&mut self, t: Transaction) -> Result<(), String> {
        self.0
            .retain(|_, v| v.created.elapsed() < Duration::from_secs(300));
        if self.0.len() >= 1000 {
            return Err("login_busy".into());
        }
        self.0.insert(t.state.clone(), t);
        Ok(())
    }
    fn take(&mut self, cookie: &str, state: &str) -> Result<Transaction, String> {
        let t = self.0.get(state).ok_or("invalid_login")?;
        if hash(cookie) != hash(&t.cookie) {
            return Err("invalid_login".into());
        }
        let t = self.0.remove(state).ok_or("invalid_login")?;
        if t.created.elapsed() >= Duration::from_secs(300) {
            return Err("invalid_login".into());
        }
        Ok(t)
    }
}
#[derive(Clone)]
pub struct BrowserAuth {
    issuer: String,
    origin: String,
    client_id: String,
    client_secret: Option<String>,
    development: bool,
    metadata: Arc<Mutex<(CoreProviderMetadata, Instant, Instant)>>,
    pending: Arc<Mutex<Pending>>,
}
pub struct LoginStart {
    pub url: String,
    pub transaction_cookie: String,
}
pub struct BrowserConfig;
impl BrowserConfig {
    pub async fn load(
        origin: String,
        issuer: String,
        client_id: String,
        client_secret: Option<String>,
        development: bool,
    ) -> Result<BrowserAuth, String> {
        validated_origin(&origin, development)?;
        let u = validated_url(&issuer, development)?;
        if u.query().is_some() || client_id.is_empty() || client_id.len() > 256 {
            return Err("invalid_browser_configuration".into());
        }
        let metadata = metadata(&issuer, development).await?;
        Ok(BrowserAuth {
            issuer,
            origin,
            client_id,
            client_secret: client_secret.filter(|s| !s.is_empty()),
            development,
            metadata: Arc::new(Mutex::new((metadata, Instant::now(), Instant::now()))),
            pending: Arc::new(Mutex::new(Pending::default())),
        })
    }
}
async fn bounded_get(url: &str) -> Result<Vec<u8>, String> {
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "provider_unavailable")?;
    let mut response = http
        .get(url)
        .send()
        .await
        .map_err(|_| "provider_unavailable")?
        .error_for_status()
        .map_err(|_| "provider_unavailable")?;
    let mut bytes = Vec::new();
    while let Some(c) = response.chunk().await.map_err(|_| "provider_unavailable")? {
        if bytes.len() + c.len() > 262144 {
            return Err("provider_response_too_large".into());
        }
        bytes.extend_from_slice(&c);
    }
    Ok(bytes)
}
async fn metadata(issuer: &str, development: bool) -> Result<CoreProviderMetadata, String> {
    let source = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let mut m: CoreProviderMetadata = serde_json::from_slice(&bounded_get(&source).await?)
        .map_err(|_| "invalid_provider_metadata")?;
    if m.issuer().as_str() != issuer {
        return Err("provider_issuer_mismatch".into());
    }
    validated_url(m.authorization_endpoint().as_str(), development)?;
    validated_url(
        m.token_endpoint().ok_or("token_endpoint_missing")?.as_str(),
        development,
    )?;
    validated_url(m.jwks_uri().as_str(), development)?;
    let keys: CoreJsonWebKeySet =
        serde_json::from_slice(&bounded_get(m.jwks_uri().as_str()).await?)
            .map_err(|_| "invalid_provider_keys")?;
    if keys.keys().is_empty() || keys.keys().len() > 128 {
        return Err("invalid_provider_keys".into());
    }
    m = m.set_jwks(keys);
    Ok(m)
}
async fn oidc_http(
    request: openidconnect::HttpRequest,
) -> Result<openidconnect::HttpResponse, std::io::Error> {
    let fail = || std::io::Error::other("provider_unavailable");
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| fail())?;
    let (parts, body) = request.into_parts();
    let mut r = http
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(|_| fail())?;
    let mut result = openidconnect::http::Response::builder().status(r.status());
    for (k, v) in r.headers() {
        result = result.header(k, v);
    }
    let mut bytes = Vec::new();
    while let Some(c) = r.chunk().await.map_err(|_| fail())? {
        if bytes.len() + c.len() > 262144 {
            return Err(fail());
        }
        bytes.extend_from_slice(&c);
    }
    result.body(bytes).map_err(|_| fail())
}
impl BrowserAuth {
    pub fn development(&self) -> bool {
        self.development
    }
    async fn current(&self) -> Result<CoreProviderMetadata, String> {
        let mut m = self.metadata.lock().await;
        if m.1.elapsed() > Duration::from_secs(60) && m.2.elapsed() > Duration::from_secs(60) {
            m.2 = Instant::now();
            match metadata(&self.issuer, self.development).await {
                Ok(next) => *m = (next, Instant::now(), Instant::now()),
                Err(_) if m.1.elapsed() < Duration::from_secs(300) => {}
                Err(_) => return Err("provider_unavailable".into()),
            }
        }
        if m.1.elapsed() >= Duration::from_secs(300) {
            return Err("provider_unavailable".into());
        }
        Ok(m.0.clone())
    }
    pub async fn begin(&self) -> Result<LoginStart, String> {
        let client = CoreClient::from_provider_metadata(
            self.current().await?,
            ClientId::new(self.client_id.clone()),
            self.client_secret.clone().map(ClientSecret::new),
        )
        .set_redirect_uri(
            RedirectUrl::new(format!("{}/auth/callback", self.origin))
                .map_err(|_| "invalid_callback")?,
        );
        let t = transaction();
        let state = t.state.clone();
        let nonce = t.nonce.clone();
        let (url, _, _) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                move || CsrfToken::new(state),
                move || Nonce::new(nonce),
            )
            .set_pkce_challenge(PkceCodeChallenge::from_code_verifier_sha256(
                &PkceCodeVerifier::new(t.verifier.clone()),
            ))
            .url();
        let result = LoginStart {
            url: url.to_string(),
            transaction_cookie: t.cookie.clone(),
        };
        self.pending.lock().await.insert(t)?;
        Ok(result)
    }
    pub async fn complete(
        &self,
        cookie: &str,
        state: &str,
        code: &str,
    ) -> Result<(String, String), String> {
        if cookie.len() != 64 || state.len() > 256 || code.is_empty() || code.len() > 4096 {
            return Err("invalid_login".into());
        }
        let t = self.pending.lock().await.take(cookie, state)?;
        let client = CoreClient::from_provider_metadata(
            self.current().await?,
            ClientId::new(self.client_id.clone()),
            self.client_secret.clone().map(ClientSecret::new),
        )
        .set_redirect_uri(
            RedirectUrl::new(format!("{}/auth/callback", self.origin))
                .map_err(|_| "invalid_callback")?,
        );
        let tokens = client
            .exchange_code(AuthorizationCode::new(code.into()))
            .map_err(|_| "invalid_login")?
            .set_pkce_verifier(PkceCodeVerifier::new(t.verifier))
            .request_async(&oidc_http)
            .await
            .map_err(|_| "invalid_login")?;
        let id = tokens.id_token().ok_or("invalid_login")?;
        let verifier = client.id_token_verifier();
        let claims = id
            .claims(&verifier, &Nonce::new(t.nonce))
            .map_err(|_| "invalid_login")?;
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                tokens.access_token(),
                id.signing_alg().map_err(|_| "invalid_login")?,
                id.signing_key(&verifier).map_err(|_| "invalid_login")?,
            )
            .map_err(|_| "invalid_login")?;
            if actual != *expected {
                return Err("invalid_login".into());
            }
        }
        let subject = claims.subject().as_str();
        if subject.is_empty() || subject.len() > 512 {
            return Err("invalid_login".into());
        }
        Ok((self.issuer.clone(), subject.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_policy_is_https_or_explicit_literal_loopback() {
        assert!(validated_url("https://issuer.example/realms/team", false).is_ok());
        assert!(validated_url("http://127.0.0.1:4318", true).is_ok());
        for url in [
            "http://example.org",
            "http://localhost:4318",
            "https://user:secret@example.org",
            "https://example.org/#fragment",
        ] {
            assert!(validated_url(url, false).is_err());
        }
        assert!(validated_origin("https://example.org/path", false).is_err());
        assert!(validated_origin("https://portal.example", true).is_err());
    }
    #[tokio::test]
    async fn concurrent_provider_outage_performs_only_one_refresh_attempt() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let app = axum::Router::new().fallback(move || {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let metadata:CoreProviderMetadata=serde_json::from_value(serde_json::json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"jwks_uri":format!("{issuer}/keys"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"]})).unwrap();
        let old = Instant::now() - Duration::from_secs(301);
        let auth = BrowserAuth {
            issuer,
            origin: "http://127.0.0.1:4318".into(),
            client_id: "test".into(),
            client_secret: None,
            development: true,
            metadata: Arc::new(Mutex::new((metadata, old, old))),
            pending: Arc::new(Mutex::new(Pending::default())),
        };
        let mut tasks = Vec::new();
        for _ in 0..5 {
            let auth = auth.clone();
            tasks.push(tokio::spawn(async move { auth.current().await }));
        }
        for task in tasks {
            assert!(task.await.unwrap().is_err());
        }
        server.abort();
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn transactions_are_browser_bound_expiring_and_single_use() {
        let mut pending = Pending::default();
        let t = transaction();
        let state = t.state.clone();
        let cookie = t.cookie.clone();
        pending.insert(t).unwrap();
        assert!(pending.take("wrong", &state).is_err());
        assert!(pending.take(&cookie, "wrong").is_err());
        assert!(pending.take(&cookie, &state).is_ok());
        assert!(pending.take(&cookie, &state).is_err());
        let mut old = transaction();
        old.created = std::time::Instant::now() - std::time::Duration::from_secs(301);
        let cookie = old.cookie.clone();
        let state = old.state.clone();
        pending.insert(old).unwrap();
        assert!(pending.take(&cookie, &state).is_err());
    }
}
