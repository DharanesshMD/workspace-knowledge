//! Fixed upstream public-client PKCE login through the system browser.
use crate::{daemon, network};
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreProviderMetadata},
    AccessTokenHash, AuthorizationCode, ClientId, CsrfToken, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, TokenResponse,
};
use serde::Deserialize;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[derive(Deserialize)]
struct Config {
    enabled: bool,
    issuer: Option<String>,
    client_id: Option<String>,
    audience: Option<String>,
}
pub struct Attempt {
    pub url: String,
    issuer: String,
    client_id: String,
    metadata: CoreProviderMetadata,
    listener: tokio::net::TcpListener,
    state: String,
    nonce: String,
    verifier: PkceCodeVerifier,
}
pub struct Credential {
    pub token: zeroize::Zeroizing<String>,
    pub expires_at: i64,
}
pub fn provider_url(value: &str, development: bool) -> Result<reqwest::Url, String> {
    let u = reqwest::Url::parse(value).map_err(|_| "invalid identity endpoint")?;
    if !(u.scheme() == "https"
        || development
            && u.scheme() == "http"
            && matches!(u.host_str(), Some("127.0.0.1" | "[::1]" | "::1")))
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err("identity endpoint requires HTTPS or explicit test loopback".into());
    }
    Ok(u)
}
async fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut r = network::client(10)?
        .get(url)
        .send()
        .await
        .map_err(|_| "identity provider unavailable")?
        .error_for_status()
        .map_err(|_| "identity provider unavailable")?;
    let mut bytes = Vec::new();
    while let Some(c) = r
        .chunk()
        .await
        .map_err(|_| "identity provider unavailable")?
    {
        if bytes.len() + c.len() > 262144 {
            return Err("identity response too large".into());
        }
        bytes.extend_from_slice(&c)
    }
    Ok(bytes)
}
async fn oidc_http(
    request: openidconnect::HttpRequest,
) -> Result<openidconnect::HttpResponse, std::io::Error> {
    let fail = || std::io::Error::other("identity exchange unavailable");
    let (parts, body) = request.into_parts();
    let mut r = network::client(10)
        .map_err(|_| fail())?
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(|_| fail())?;
    let mut out = openidconnect::http::Response::builder().status(r.status());
    for (k, v) in r.headers() {
        out = out.header(k, v)
    }
    let mut bytes = Vec::new();
    while let Some(c) = r.chunk().await.map_err(|_| fail())? {
        if bytes.len() + c.len() > 262144 {
            return Err(fail());
        }
        bytes.extend_from_slice(&c)
    }
    out.body(bytes).map_err(|_| fail())
}
pub async fn begin(server: &str) -> Result<Attempt, String> {
    let origin = network::server_url(server)?;
    let response = network::client(3)?
        .get(format!("{origin}/collector/config"))
        .send()
        .await
        .map_err(|_| "collector configuration unavailable")?;
    let config: Config = serde_json::from_value(wk_summarize::bounded_json(response, 65536).await?)
        .map_err(|_| "invalid collector configuration")?;
    if !config.enabled {
        return Err("hosted collector login is not configured".into());
    }
    let issuer = config
        .issuer
        .ok_or("collector identity provider is not configured")?;
    let client_id = config
        .client_id
        .ok_or("collector public client is not configured")?;
    if client_id.is_empty() || client_id.len() > 256 || config.audience.is_none_or(|a| a.is_empty())
    {
        return Err("invalid collector identity configuration".into());
    }
    let development = origin.starts_with("http://");
    provider_url(&issuer, development)?;
    let mut metadata: CoreProviderMetadata = serde_json::from_slice(
        &get_bytes(&format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        ))
        .await?,
    )
    .map_err(|_| "invalid identity metadata")?;
    if metadata.issuer().as_str() != issuer {
        return Err("identity issuer mismatch".into());
    }
    provider_url(metadata.authorization_endpoint().as_str(), development)?;
    provider_url(
        metadata
            .token_endpoint()
            .ok_or("identity token endpoint missing")?
            .as_str(),
        development,
    )?;
    provider_url(metadata.jwks_uri().as_str(), development)?;
    let keys: CoreJsonWebKeySet =
        serde_json::from_slice(&get_bytes(metadata.jwks_uri().as_str()).await?)
            .map_err(|_| "invalid identity keys")?;
    if keys.keys().is_empty() || keys.keys().len() > 128 {
        return Err("invalid identity keys".into());
    }
    metadata = metadata.set_jwks(keys);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "login callback unavailable")?;
    let callback = format!(
        "http://{}/callback",
        listener
            .local_addr()
            .map_err(|_| "login callback unavailable")?
    );
    let client = CoreClient::from_provider_metadata(
        metadata.clone(),
        ClientId::new(client_id.clone()),
        None,
    )
    .set_redirect_uri(RedirectUrl::new(callback).map_err(|_| "invalid callback")?);
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(challenge)
        .url();
    Ok(Attempt {
        url: url.to_string(),
        issuer,
        client_id,
        metadata,
        listener,
        state: state.secret().clone(),
        nonce: nonce.secret().clone(),
        verifier,
    })
}
pub fn callback(raw: &str, host: &str, state: &str, issuer: &str) -> Result<String, String> {
    let invalid = || "invalid login callback".to_string();
    let mut lines = raw.split("\r\n");
    let request: Vec<&str> = lines.next().ok_or_else(invalid)?.split(' ').collect();
    if request.len() != 3
        || request[0] != "GET"
        || request[2] != "HTTP/1.1"
        || !request[1].starts_with("/callback?")
    {
        return Err(invalid());
    }
    let hosts: Vec<&str> = lines
        .filter_map(|l| l.split_once(':'))
        .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.trim())
        .collect();
    if hosts != [host] {
        return Err(invalid());
    }
    let u = reqwest::Url::parse(&format!("http://{host}{}", request[1])).map_err(|_| invalid())?;
    if u.path() != "/callback" || u.fragment().is_some() {
        return Err(invalid());
    }
    let pairs: Vec<_> = u.query_pairs().collect();
    let one = |k: &str| -> Result<String, String> {
        let values: Vec<_> = pairs
            .iter()
            .filter(|(name, _)| name == k)
            .map(|(_, v)| v.to_string())
            .collect();
        if values.len() != 1 {
            return Err(invalid());
        }
        Ok(values[0].clone())
    };
    if one("state")? != state || pairs.iter().any(|(k, v)| k == "iss" && v != issuer) {
        return Err(invalid());
    }
    let code = one("code")?;
    if code.is_empty() || code.len() > 4096 || code.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(code)
}
impl Attempt {
    pub fn open_browser(&self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let result = std::process::Command::new("open").arg(&self.url).status();
        #[cfg(target_os = "linux")]
        let result = std::process::Command::new("xdg-open")
            .arg(&self.url)
            .status();
        #[cfg(target_os = "windows")]
        let result = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Process -FilePath $env:WK_DEVICE_AUTH_URL",
            ])
            .env("WK_DEVICE_AUTH_URL", &self.url)
            .status();
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        return Err("system browser unsupported; open the displayed authorization URL".into());
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        if !result.map_err(|_| "system browser unavailable")?.success() {
            return Err("system browser unavailable; open the displayed authorization URL".into());
        }
        Ok(())
    }
    pub async fn complete(self) -> Result<Credential, String> {
        let address = self
            .listener
            .local_addr()
            .map_err(|_| "login callback unavailable")?
            .to_string();
        let code=tokio::time::timeout(Duration::from_secs(300),async {for _ in 0..32 {let(mut stream,_)=self.listener.accept().await.map_err(|_|"login callback unavailable")?;let mut bytes=Vec::new();let read=tokio::time::timeout(Duration::from_secs(3),async {let mut chunk=[0u8;1024];while !bytes.windows(4).any(|w|w==b"\r\n\r\n"){let n=stream.read(&mut chunk).await.map_err(|_|"invalid callback")?;if n==0||bytes.len()+n>8192{return Err("invalid callback")}bytes.extend_from_slice(&chunk[..n]);}Ok::<(),&str>(())}).await;let result=if matches!(read,Ok(Ok(()))){std::str::from_utf8(&bytes).map_err(|_|"invalid callback".into()).and_then(|raw|callback(raw,&address,&self.state,&self.issuer))}else{Err("invalid callback".into())};let response=if result.is_ok(){"HTTP/1.1 200 OK\r\nContent-Length: 41\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nSign-in received. Return to the collector."}else{"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"};let _=stream.write_all(response.as_bytes()).await;if let Ok(code)=result{return Ok(code)}}Err("login callback rejected")}).await.map_err(|_|"collector sign-in expired")??;
        let client =
            CoreClient::from_provider_metadata(self.metadata, ClientId::new(self.client_id), None)
                .set_redirect_uri(
                    RedirectUrl::new(format!("http://{address}/callback"))
                        .map_err(|_| "invalid callback")?,
                );
        let tokens = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|_| "identity exchange rejected")?
            .set_pkce_verifier(self.verifier)
            .request_async(&oidc_http)
            .await
            .map_err(|_| "identity exchange rejected")?;
        let id = tokens.id_token().ok_or("identity token missing")?;
        let verifier = client.id_token_verifier();
        let transaction_nonce = Nonce::new(self.nonce);
        let claims = id
            .claims(&verifier, &transaction_nonce)
            .map_err(|_| "identity verification failed")?;
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                tokens.access_token(),
                id.signing_alg()
                    .map_err(|_| "identity verification failed")?,
                id.signing_key(&verifier)
                    .map_err(|_| "identity verification failed")?,
            )
            .map_err(|_| "identity verification failed")?;
            if actual != *expected {
                return Err("identity token binding failed".into());
            }
        }
        let expires = tokens
            .expires_in()
            .ok_or("access-token lifetime missing")?
            .as_secs()
            .min(43200);
        if expires == 0 || tokens.access_token().secret().len() > 16384 {
            return Err("invalid upstream credential".into());
        }
        Ok(Credential {
            token: zeroize::Zeroizing::new(tokens.access_token().secret().clone()),
            expires_at: daemon::now()? + expires as i64,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_is_bound_to_host_path_state_and_issuer() {
        let host = "127.0.0.1:4318";
        let issuer = "https://issuer.example";
        let valid="GET /callback?code=authorized&state=expected&iss=https%3A%2F%2Fissuer.example HTTP/1.1\r\nHost: 127.0.0.1:4318\r\n\r\n";
        assert_eq!(
            callback(valid, host, "expected", issuer).unwrap(),
            "authorized"
        );
        for bad in [
            valid.replace("expected", "wrong"),
            valid.replace("/callback", "/other"),
            valid.replace("code=authorized", "code=one&code=two"),
            valid.replace("Host: 127.0.0.1:4318", "Host: attacker.example"),
            valid.replace("issuer.example", "other.example"),
        ] {
            assert!(callback(&bad, host, "expected", issuer).is_err())
        }
    }
    #[test]
    fn provider_urls_require_https_or_explicit_test_loopback() {
        assert!(provider_url("https://issuer.example/authorize", false).is_ok());
        assert!(provider_url("http://127.0.0.1:4318", true).is_ok());
        for bad in [
            "http://issuer.example",
            "http://localhost:4318",
            "https://user:password@issuer.example",
            "https://issuer.example/#private",
        ] {
            assert!(provider_url(bad, false).is_err());
        }
    }
}
#[cfg(test)]
mod live_tests {
    use super::*;
    use axum::{
        routing::{get, post},
        Form, Json,
    };
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    #[tokio::test]
    async fn real_loopback_pkce_exchange_rejects_wrong_state_and_nonce() {
        for bad_nonce in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let issuer = format!("http://{}", listener.local_addr().unwrap());
            let fixture: Value = serde_json::from_str(include_str!(
                "../../server/tests/fixtures/synthetic-rsa-a.json"
            ))
            .unwrap();
            let grant = Arc::new(Mutex::new(HashMap::<String, String>::new()));
            let metadata_issuer = issuer.clone();
            let config_issuer = issuer.clone();
            let keys = fixture.clone();
            let token_fixture = fixture.clone();
            let token_issuer = issuer.clone();
            let token_grant = grant.clone();
            let router=axum::Router::new().route("/collector/config",get(move||{let issuer=config_issuer.clone();async move{Json(json!({"enabled":true,"issuer":issuer,"client_id":"collector-test","audience":"wk-collector"}))}})).route("/.well-known/openid-configuration",get(move||{let issuer=metadata_issuer.clone();async move{Json(json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"jwks_uri":format!("{issuer}/keys"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"]}))}})).route("/keys",get(move||{let keys=keys.clone();async move{Json(json!({"keys":[{"kty":"RSA","kid":"test","alg":"RS256","use":"sig","n":keys["n"],"e":keys["e"]}]}))}})).route("/token",post(move|Form(body):Form<HashMap<String,String>>|{let fixture=token_fixture.clone();let issuer=token_issuer.clone();let grant=token_grant.clone();async move{let saved=grant.lock().unwrap().clone();assert_eq!(body["grant_type"],"authorization_code");assert_eq!(body["client_id"],"collector-test");assert_eq!(body["redirect_uri"],saved["redirect_uri"]);assert_eq!(PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(body["code_verifier"].clone())).as_str(),saved["code_challenge"]);let mut header=Header::new(Algorithm::RS256);header.kid=Some("test".into());let claims=json!({"iss":issuer,"aud":"collector-test","sub":"synthetic-developer","nonce":if bad_nonce{"wrong"}else{saved["nonce"].as_str()},"iat":daemon::now().unwrap(),"exp":daemon::now().unwrap()+300});let token=encode(&header,&claims,&EncodingKey::from_rsa_pem(fixture["private_pkcs8_pem"].as_str().unwrap().as_bytes()).unwrap()).unwrap();Json(json!({"access_token":"synthetic-upstream-credential","token_type":"Bearer","expires_in":300,"id_token":token}))}}));
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let attempt = begin(&issuer).await.unwrap();
            let url = reqwest::Url::parse(&attempt.url).unwrap();
            let values: HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            assert_eq!(values["code_challenge_method"], "S256");
            *grant.lock().unwrap() = values.clone();
            let completion = tokio::spawn(attempt.complete());
            let wrong = format!("{}?code=code&state=wrong", values["redirect_uri"]);
            assert_eq!(
                network::client(3)
                    .unwrap()
                    .get(wrong)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                reqwest::StatusCode::BAD_REQUEST
            );
            let mut callback_url = reqwest::Url::parse(&values["redirect_uri"]).unwrap();
            callback_url
                .query_pairs_mut()
                .append_pair("code", "code")
                .append_pair("state", &values["state"]);
            assert_eq!(
                network::client(3)
                    .unwrap()
                    .get(callback_url)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                reqwest::StatusCode::OK
            );
            let result = completion.await.unwrap();
            assert_eq!(result.is_err(), bad_nonce);
            if let Ok(credential) = result {
                assert_eq!(credential.token.as_str(), "synthetic-upstream-credential")
            }
            server.abort();
        }
    }
}
