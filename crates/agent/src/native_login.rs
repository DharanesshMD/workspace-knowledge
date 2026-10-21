//! Fixed upstream public-client PKCE login through the system browser.
use crate::{daemon, network};
use serde::Deserialize;
#[derive(Deserialize)]
struct Config {
    enabled: bool,
    issuer: Option<String>,
    client_id: Option<String>,
    audience: Option<String>,
}
pub struct Attempt {
    pub url: String,
    inner: wk_login::Attempt,
}
pub struct Credential {
    pub token: zeroize::Zeroizing<String>,
    pub expires_at: i64,
}
/// Start a login. `ca` is the operator-approved private CA bundle for the server and its
/// identity provider; without one the public roots apply.
pub async fn begin(server: &str, ca: Option<&[reqwest::Certificate]>) -> Result<Attempt, String> {
    let origin = network::server_url(server)?;
    let response = network::client_trusting(ca, 3)?
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
    let inner = wk_login::begin(
        &wk_login::Request {
            issuer: &issuer,
            client_id: &client_id,
            development: origin.starts_with("http://"),
            resource: None,
            scopes: &[],
            done_message: "Sign-in received. Return to the collector.",
        },
        ca,
    )
    .await?;
    Ok(Attempt {
        url: inner.url.clone(),
        inner,
    })
}
impl Attempt {
    pub fn open_browser(&self) -> Result<(), String> {
        self.inner.open_browser()
    }
    pub async fn complete(self) -> Result<Credential, String> {
        let tokens = self.inner.complete().await.map_err(|e| {
            if e == "sign-in expired" {
                "collector sign-in expired".into()
            } else {
                e
            }
        })?;
        Ok(Credential {
            token: tokens.access_token,
            expires_at: daemon::now()? + tokens.expires_in as i64,
        })
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
    use openidconnect::{PkceCodeChallenge, PkceCodeVerifier};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    type Grant = Arc<Mutex<HashMap<String, String>>>;
    fn oidc_router(issuer: &str, grant: &Grant, bad_nonce: bool) -> axum::Router {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../server/tests/fixtures/synthetic-rsa-a.json"
        ))
        .unwrap();
        let metadata_issuer = issuer.to_string();
        let config_issuer = issuer.to_string();
        let keys = fixture.clone();
        let token_fixture = fixture.clone();
        let token_issuer = issuer.to_string();
        let token_grant = grant.clone();
        axum::Router::new().route("/collector/config",get(move||{let issuer=config_issuer.clone();async move{Json(json!({"enabled":true,"issuer":issuer,"client_id":"collector-test","audience":"wk-collector"}))}})).route("/.well-known/openid-configuration",get(move||{let issuer=metadata_issuer.clone();async move{Json(json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"jwks_uri":format!("{issuer}/keys"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"]}))}})).route("/keys",get(move||{let keys=keys.clone();async move{Json(json!({"keys":[{"kty":"RSA","kid":"test","alg":"RS256","use":"sig","n":keys["n"],"e":keys["e"]}]}))}})).route("/token",post(move|Form(body):Form<HashMap<String,String>>|{let fixture=token_fixture.clone();let issuer=token_issuer.clone();let grant=token_grant.clone();async move{let saved=grant.lock().unwrap().clone();assert_eq!(body["grant_type"],"authorization_code");assert_eq!(body["client_id"],"collector-test");assert_eq!(body["redirect_uri"],saved["redirect_uri"]);assert_eq!(PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(body["code_verifier"].clone())).as_str(),saved["code_challenge"]);let mut header=Header::new(Algorithm::RS256);header.kid=Some("test".into());let claims=json!({"iss":issuer,"aud":"collector-test","sub":"synthetic-developer","nonce":if bad_nonce{"wrong"}else{saved["nonce"].as_str()},"iat":daemon::now().unwrap(),"exp":daemon::now().unwrap()+300});let token=encode(&header,&claims,&EncodingKey::from_rsa_pem(fixture["private_pkcs8_pem"].as_str().unwrap().as_bytes()).unwrap()).unwrap();Json(json!({"access_token":"synthetic-upstream-credential","token_type":"Bearer","expires_in":300,"id_token":token}))}}))
    }
    /// Complete a started login the way the system browser would, returning its outcome.
    async fn finish(attempt: Attempt, grant: &Grant) -> Result<Credential, String> {
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
        completion.await.unwrap()
    }
    #[tokio::test]
    async fn real_loopback_pkce_exchange_rejects_wrong_state_and_nonce() {
        for bad_nonce in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let issuer = format!("http://{}", listener.local_addr().unwrap());
            let grant = Grant::default();
            let router = oidc_router(&issuer, &grant, bad_nonce);
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let attempt = begin(&issuer, None).await.unwrap();
            let result = finish(attempt, &grant).await;
            assert_eq!(result.is_err(), bad_nonce);
            if let Ok(credential) = result {
                assert_eq!(credential.token.as_str(), "synthetic-upstream-credential")
            }
            server.abort();
        }
    }
    #[tokio::test]
    async fn login_behind_a_private_ca_needs_the_pinned_bundle_for_discovery_and_exchange() {
        use crate::tls_fixture::{pki, serve};
        let grant = Grant::default();
        let routed = grant.clone();
        let (origin, server) = serve(move |origin| oidc_router(origin, &routed, false)).await;
        let ca = |pem: &[u8]| wk_summarize::tls::parse_ca_bundle(pem).unwrap();
        for refused in [None, Some(ca(&pki().other_ca))] {
            let error = begin(&origin, refused.as_deref()).await.err();
            assert_eq!(
                error.as_deref(),
                Some("collector configuration unavailable")
            );
        }
        let pinned = ca(&pki().ca);
        let attempt = begin(&origin, Some(&pinned)).await.unwrap();
        let credential = finish(attempt, &grant).await.unwrap();
        assert_eq!(credential.token.as_str(), "synthetic-upstream-credential");
        server.abort();
    }
    #[tokio::test]
    async fn native_login_rejects_incomplete_interactive_discovery() {
        use axum::{routing::get, Json};
        for missing in [
            "response_types_supported",
            "subject_types_supported",
            "id_token_signing_alg_values_supported",
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let issuer = format!("http://{}", listener.local_addr().unwrap());
            let config = serde_json::json!({"enabled":true,"issuer":issuer,"client_id":"collector-test","audience":"wk-collector"});
            let mut document = serde_json::json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"jwks_uri":format!("{issuer}/keys"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"]});
            document.as_object_mut().unwrap().remove(missing);
            let key: serde_json::Value = serde_json::from_str(include_str!(
                "../../server/tests/fixtures/synthetic-rsa-a.json"
            ))
            .unwrap();
            let keys = serde_json::json!({"keys":[{"kty":"RSA","kid":"test","alg":"RS256","use":"sig","n":key["n"],"e":key["e"]}]});
            let app = axum::Router::new()
                .route(
                    "/collector/config",
                    get(move || {
                        let config = config.clone();
                        async move { Json(config) }
                    }),
                )
                .route(
                    "/.well-known/openid-configuration",
                    get(move || {
                        let document = document.clone();
                        async move { Json(document) }
                    }),
                )
                .route(
                    "/keys",
                    get(move || {
                        let keys = keys.clone();
                        async move { Json(keys) }
                    }),
                );
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let result = begin(&issuer, None).await;
            task.abort();
            assert_eq!(
                result.err().as_deref(),
                Some("invalid identity metadata"),
                "accepted missing {missing}"
            );
        }
    }
}
