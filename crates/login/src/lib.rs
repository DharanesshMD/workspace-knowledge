//! Public-client OIDC login: authorization code with PKCE through the system browser and a
//! literal-loopback redirect, plus refresh-token renewal. Shared by the collector and `wk-mcp`.
//! Every request to the identity provider uses the caller's client, so a private CA bundle
//! replaces the public roots instead of adding to them.
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreProviderMetadata},
    AccessTokenHash, AuthorizationCode, ClientId, CsrfToken, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RefreshToken, RequestTokenError, Scope,
    TokenResponse,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

const MAX_IDENTITY_RESPONSE: usize = 262144;
/// Longest access-token lifetime honoured, whatever the provider claims.
const MAX_LIFETIME_SECONDS: u64 = 43200;

/// What to sign in to. `development` allows a literal-loopback `http://` provider.
pub struct Request<'a> {
    pub issuer: &'a str,
    pub client_id: &'a str,
    pub development: bool,
    /// RFC 8707 resource indicator sent with the authorization and token requests.
    pub resource: Option<&'a str>,
    /// Scopes beyond `openid`, which is always requested.
    pub scopes: &'a [&'a str],
    /// Shown by the browser once the callback has been received.
    pub done_message: &'static str,
}
pub struct Tokens {
    pub access_token: Zeroizing<String>,
    pub refresh_token: Option<Zeroizing<String>>,
    pub expires_in: u64,
}
/// Why a refresh-token exchange failed: the provider refusing the token is final, anything else
/// is worth retrying later.
#[derive(Debug, PartialEq, Eq)]
pub enum RefreshError {
    Rejected,
    Unavailable(String),
}
pub struct Attempt {
    pub url: String,
    issuer: String,
    client_id: String,
    done_message: &'static str,
    resource: Option<String>,
    metadata: CoreProviderMetadata,
    listener: tokio::net::TcpListener,
    state: String,
    nonce: String,
    verifier: PkceCodeVerifier,
    provider: reqwest::Client,
}

/// A client for the hosted server and its identity provider. With a bundle it replaces the
/// public roots; without one the public roots apply. Nothing else changes the trust set.
pub fn client_trusting(
    ca: Option<&[reqwest::Certificate]>,
    seconds: u64,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(seconds));
    if let Some(certificates) = ca {
        builder = builder.tls_built_in_root_certs(false);
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate.clone());
        }
    }
    builder.build().map_err(|_| "network unavailable".into())
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
async fn get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let mut r = client
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
        if bytes.len() + c.len() > MAX_IDENTITY_RESPONSE {
            return Err("identity response too large".into());
        }
        bytes.extend_from_slice(&c)
    }
    Ok(bytes)
}
async fn oidc_http(
    client: reqwest::Client,
    request: openidconnect::HttpRequest,
) -> Result<openidconnect::HttpResponse, std::io::Error> {
    let fail = || std::io::Error::other("identity exchange unavailable");
    let (parts, body) = request.into_parts();
    let mut r = client
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
        if bytes.len() + c.len() > MAX_IDENTITY_RESPONSE {
            return Err(fail());
        }
        bytes.extend_from_slice(&c)
    }
    out.body(bytes).map_err(|_| fail())
}
/// Fetch and check the provider's discovery document. Every endpoint it names must itself be
/// HTTPS (or test loopback), and the advertised issuer must be exactly the one asked for.
async fn discover(
    provider: &reqwest::Client,
    issuer: &str,
    development: bool,
) -> Result<CoreProviderMetadata, String> {
    provider_url(issuer, development)?;
    let metadata: CoreProviderMetadata = serde_json::from_slice(
        &get_bytes(
            provider,
            &format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            ),
        )
        .await?,
    )
    .map_err(|_| "invalid identity metadata")?;
    if metadata.issuer().as_str() != issuer {
        return Err("identity issuer mismatch".into());
    }
    // Interactive PKCE needs complete discovery even if the library also
    // supports workload identities that omit these fields.
    if metadata.response_types_supported().is_empty()
        || metadata.subject_types_supported().is_empty()
        || metadata.id_token_signing_alg_values_supported().is_empty()
    {
        return Err("invalid identity metadata".into());
    }
    provider_url(
        metadata
            .authorization_endpoint()
            .ok_or("identity authorization endpoint missing")?
            .as_str(),
        development,
    )?;
    provider_url(
        metadata
            .token_endpoint()
            .ok_or("identity token endpoint missing")?
            .as_str(),
        development,
    )?;
    provider_url(metadata.jwks_uri().as_str(), development)?;
    Ok(metadata)
}
/// Start a login. `ca` is the operator-approved private CA bundle for the identity provider;
/// without one the public roots apply.
pub async fn begin(
    request: &Request<'_>,
    ca: Option<&[reqwest::Certificate]>,
) -> Result<Attempt, String> {
    let provider = client_trusting(ca, 10)?;
    let mut metadata = discover(&provider, request.issuer, request.development).await?;
    let keys: CoreJsonWebKeySet =
        serde_json::from_slice(&get_bytes(&provider, metadata.jwks_uri().as_str()).await?)
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
        ClientId::new(request.client_id.into()),
        None,
    )
    .set_redirect_uri(RedirectUrl::new(callback).map_err(|_| "invalid callback")?);
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut authorization = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(challenge);
    for scope in request.scopes {
        authorization = authorization.add_scope(Scope::new((*scope).into()));
    }
    if let Some(resource) = request.resource {
        authorization = authorization.add_extra_param("resource", resource);
    }
    let (url, state, nonce) = authorization.url();
    Ok(Attempt {
        url: url.to_string(),
        issuer: request.issuer.into(),
        client_id: request.client_id.into(),
        done_message: request.done_message,
        resource: request.resource.map(Into::into),
        metadata,
        listener,
        state: state.secret().clone(),
        nonce: nonce.secret().clone(),
        verifier,
        provider,
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
/// Read one callback request, at most 8 KiB, and answer it. Returns the code when it is valid.
async fn serve_callback(
    listener: &tokio::net::TcpListener,
    address: &str,
    state: &str,
    issuer: &str,
    done_message: &str,
) -> Result<Option<String>, &'static str> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|_| "login callback unavailable")?;
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(3), async {
        let mut chunk = [0u8; 1024];
        while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|_| "invalid callback")?;
            if n == 0 || bytes.len() + n > 8192 {
                return Err("invalid callback");
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
        Ok::<(), &str>(())
    })
    .await;
    let result = if matches!(read, Ok(Ok(()))) {
        std::str::from_utf8(&bytes)
            .map_err(|_| "invalid callback".to_string())
            .and_then(|raw| callback(raw, address, state, issuer))
    } else {
        Err("invalid callback".to_string())
    };
    let response = if result.is_ok() {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{done_message}", done_message.len())
    } else {
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n".into()
    };
    let _ = stream.write_all(response.as_bytes()).await;
    Ok(result.ok())
}
fn lifetime(seconds: Option<Duration>, access: &str) -> Result<u64, String> {
    let expires = seconds
        .ok_or("access-token lifetime missing")?
        .as_secs()
        .min(MAX_LIFETIME_SECONDS);
    if expires == 0 || access.len() > 16384 {
        return Err("invalid upstream credential".into());
    }
    Ok(expires)
}
/// Open `url` in the system browser; failure leaves the caller to show the URL instead.
pub fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Process -FilePath $env:WK_DEVICE_AUTH_URL",
        ])
        .env("WK_DEVICE_AUTH_URL", url)
        .status();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err("system browser unsupported; open the displayed authorization URL".into());
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    if !result.map_err(|_| "system browser unavailable")?.success() {
        return Err("system browser unavailable; open the displayed authorization URL".into());
    }
    Ok(())
}
impl Attempt {
    pub fn open_browser(&self) -> Result<(), String> {
        open_browser(&self.url)
    }
    pub async fn complete(self) -> Result<Tokens, String> {
        let address = self
            .listener
            .local_addr()
            .map_err(|_| "login callback unavailable")?
            .to_string();
        let code = tokio::time::timeout(Duration::from_secs(300), async {
            for _ in 0..32 {
                if let Some(code) = serve_callback(
                    &self.listener,
                    &address,
                    &self.state,
                    &self.issuer,
                    self.done_message,
                )
                .await?
                {
                    return Ok(code);
                }
            }
            Err("login callback rejected")
        })
        .await
        .map_err(|_| "sign-in expired")??;
        let provider = self.provider;
        let client =
            CoreClient::from_provider_metadata(self.metadata, ClientId::new(self.client_id), None)
                .set_redirect_uri(
                    RedirectUrl::new(format!("http://{address}/callback"))
                        .map_err(|_| "invalid callback")?,
                );
        let mut exchange = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|_| "identity exchange rejected")?
            .set_pkce_verifier(self.verifier);
        if let Some(resource) = &self.resource {
            exchange = exchange.add_extra_param("resource", resource);
        }
        let tokens = exchange
            .request_async(&|request| oidc_http(provider.clone(), request))
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
        let access = tokens.access_token().secret();
        Ok(Tokens {
            expires_in: lifetime(tokens.expires_in(), access)?,
            access_token: Zeroizing::new(access.clone()),
            refresh_token: tokens
                .refresh_token()
                .map(|t| Zeroizing::new(t.secret().clone())),
        })
    }
}
/// Exchange a refresh token for a new access token. The provider may rotate the refresh token;
/// the caller must then store the new one.
pub async fn refresh(
    request: &Request<'_>,
    ca: Option<&[reqwest::Certificate]>,
    refresh_token: &str,
) -> Result<Tokens, RefreshError> {
    let unavailable = |m: &str| RefreshError::Unavailable(m.to_string());
    let provider = client_trusting(ca, 10).map_err(|m| unavailable(&m))?;
    let metadata = discover(&provider, request.issuer, request.development)
        .await
        .map_err(|m| unavailable(&m))?;
    let client =
        CoreClient::from_provider_metadata(metadata, ClientId::new(request.client_id.into()), None);
    let token = RefreshToken::new(refresh_token.into());
    let response = client
        .exchange_refresh_token(&token)
        .map_err(|_| unavailable("identity exchange rejected"))?
        .request_async(&|r| oidc_http(provider.clone(), r))
        .await
        .map_err(|e| match e {
            RequestTokenError::ServerResponse(r)
                if matches!(
                    r.error(),
                    openidconnect::core::CoreErrorResponseType::InvalidGrant
                ) =>
            {
                RefreshError::Rejected
            }
            _ => unavailable("identity exchange unavailable"),
        })?;
    let access = response.access_token().secret();
    Ok(Tokens {
        expires_in: lifetime(response.expires_in(), access).map_err(|m| unavailable(&m))?,
        access_token: Zeroizing::new(access.clone()),
        refresh_token: response
            .refresh_token()
            .map(|t| Zeroizing::new(t.secret().clone())),
    })
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
    use crate::test_support::{browser, pki, provider, serve, Behaviour, Shared};
    fn request<'a>(
        issuer: &'a str,
        scopes: &'a [&'a str],
        resource: Option<&'a str>,
    ) -> Request<'a> {
        Request {
            issuer,
            client_id: crate::test_support::CLIENT,
            development: false,
            resource,
            scopes,
            done_message: "Signed in.",
        }
    }
    #[tokio::test]
    async fn login_and_refresh_pin_the_private_ca_and_carry_resource_scopes_and_rotation() {
        let seen = Shared::default();
        let routed = seen.clone();
        let (origin, server) = serve(move |origin| {
            provider(
                origin,
                &routed,
                Behaviour {
                    lifetime: 300,
                    rotate: true,
                    audience: None,
                    issue_refresh: true,
                },
            )
        })
        .await;
        let ca = |pem: &[u8]| wk_summarize::tls::parse_ca_bundle(pem).unwrap();
        let (pinned, other) = (ca(&pki().ca), ca(&pki().other_ca));
        let want = request(&origin, &["offline_access"], Some("https://mcp.example"));
        for refused in [None, Some(&other[..])] {
            assert_eq!(
                begin(&want, refused).await.err().as_deref(),
                Some("identity provider unavailable")
            );
            assert!(matches!(
                refresh(&want, refused, "refresh-1").await,
                Err(RefreshError::Unavailable(_))
            ));
        }
        let attempt = begin(&want, Some(&pinned)).await.unwrap();
        let tokens = browser(attempt, &seen).await.unwrap();
        assert_eq!(tokens.access_token.as_str(), "access-1");
        assert_eq!(
            tokens.refresh_token.as_deref().map(String::as_str),
            Some("refresh-1")
        );
        assert_eq!(tokens.expires_in, 300);
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.authorization["scope"], "openid offline_access");
            assert_eq!(seen.authorization["resource"], "https://mcp.example");
            assert_eq!(seen.authorization["code_challenge_method"], "S256");
            assert_eq!(seen.tokens[0]["resource"], "https://mcp.example");
        }
        let rotated = refresh(&want, Some(&pinned), "refresh-1").await.unwrap();
        assert_eq!(rotated.access_token.as_str(), "access-2");
        assert_eq!(
            rotated.refresh_token.as_deref().map(String::as_str),
            Some("refresh-2")
        );
        assert_eq!(
            refresh(&want, Some(&pinned), "refresh-1").await.err(),
            Some(RefreshError::Rejected),
            "a rotated-out token is revoked"
        );
        assert_eq!(
            refresh(&want, Some(&pinned), "revoked").await.err(),
            Some(RefreshError::Rejected)
        );
        let last = seen.lock().unwrap().tokens.last().cloned().unwrap();
        assert_eq!(
            (last["client_id"].as_str(), last["grant_type"].as_str()),
            (crate::test_support::CLIENT, "refresh_token")
        );
        server.abort();
    }
}
