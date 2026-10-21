//! `wk-mcp login`: sign in once through the system browser and keep only a refresh token in the
//! OS credential store. The server names its identity provider (and, if configured, the sign-in
//! client) in its protected-resource metadata; this never trusts a token's contents.
use crate::store::{Store, Stored};
use base64::Engine;
use serde_json::{json, Value};
use wk_login::Request;

pub struct Options {
    pub server: String,
    pub ca: Option<Vec<reqwest::Certificate>>,
    /// Overrides the client the server advertises.
    pub client_id: Option<String>,
    /// Ask for a long-lived offline token (the provider must allow it for the client).
    pub offline_access: bool,
}
pub struct Discovered {
    pub resource: String,
    pub issuer: String,
    pub client_id: Option<String>,
}
/// Read `/.well-known/oauth-protected-resource`. The resource must be the very origin the
/// developer named, and there must be exactly one authorization server.
pub async fn discover(
    origin: &str,
    ca: Option<&[reqwest::Certificate]>,
) -> Result<Discovered, String> {
    let response = wk_login::client_trusting(ca, 5)?
        .get(format!("{origin}/.well-known/oauth-protected-resource"))
        .send()
        .await
        .map_err(|_| "server unavailable or not trusted; check the URL and --ca-file")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(
            "the server does not offer remote MCP sign-in (WK_MCP_RESOURCE is unset)".into(),
        );
    }
    let document = wk_summarize::bounded_json(response, 65536).await?;
    if document["resource"] != origin {
        return Err("the server's MCP resource does not match the server URL".into());
    }
    let issuers = document["authorization_servers"]
        .as_array()
        .filter(|a| a.len() == 1)
        .and_then(|a| a[0].as_str())
        .ok_or("the server must name exactly one authorization server")?;
    Ok(Discovered {
        resource: origin.into(),
        issuer: issuers.into(),
        client_id: document["wk_mcp_client_id"].as_str().map(str::to_string),
    })
}
/// The `aud` of an access token that is a JWT, read without verifying it: only to explain a
/// misconfigured provider early. The server alone decides whether a token is acceptable.
fn audiences(token: &str) -> Option<Vec<String>> {
    let payload = token
        .split('.')
        .nth(1)
        .filter(|_| token.split('.').count() == 3)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    match serde_json::from_slice::<Value>(&bytes).ok()?.get("aud")? {
        Value::String(one) => Some(vec![one.clone()]),
        Value::Array(many) => many
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => None,
    }
}
pub struct Outcome {
    pub expires_in: u64,
}
/// Sign in. `show` receives the authorization URL once the loopback listener is ready (print it
/// and open the browser); the call then waits for the browser to return.
pub async fn login(
    options: Options,
    store: &dyn Store,
    show: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Outcome, String> {
    let origin = crate::net::server_origin(&options.server)?;
    let found = discover(&origin, options.ca.as_deref()).await?;
    let client_id = options
        .client_id
        .or(found.client_id)
        .filter(|c| !c.is_empty() && c.len() <= 256)
        .ok_or("no sign-in client: pass --client-id or set WK_MCP_CLIENT_ID on the server")?;
    let scopes: &[&str] = if options.offline_access {
        &["offline_access"]
    } else {
        &[]
    };
    let attempt = wk_login::begin(
        &Request {
            issuer: &found.issuer,
            client_id: &client_id,
            development: origin.starts_with("http://"),
            resource: Some(&found.resource),
            scopes,
            done_message: "Signed in to Workspace Knowledge. You can close this tab.",
        },
        options.ca.as_deref(),
    )
    .await?;
    show(&attempt.url)?;
    let tokens = attempt.complete().await?;
    if let Some(aud) = audiences(&tokens.access_token) {
        if aud != [found.resource.as_str()] {
            return Err(format!(
                "the identity provider issued a token for audience {aud:?}, but the server accepts exactly [\"{}\"]; see docs/operations/oidc-keycloak.md",
                found.resource
            ));
        }
    }
    let refresh = tokens.refresh_token.ok_or(
        "the identity provider issued no refresh token; allow refresh tokens for this client",
    )?;
    store.save(&Stored {
        issuer: found.issuer,
        client_id,
        resource: found.resource,
        refresh_token: refresh.to_string(),
    })?;
    Ok(Outcome {
        expires_in: tokens.expires_in,
    })
}
pub fn summary(origin: &str, outcome: &Outcome) -> Value {
    json!({"authenticated":true,"server":origin,"access_token_expires_in":outcome.expires_in})
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Memory;
    use axum::{routing::get, Json};
    use wk_login::test_support::{pki, provider, serve, visit, Behaviour, Shared, CLIENT};

    #[derive(Clone, Default)]
    struct Setup {
        resource: Option<String>,
        servers: Option<Value>,
        advertise: bool,
        audience: Option<Value>,
        issue_refresh: bool,
    }
    /// One origin that is both the MCP server and its identity provider.
    async fn stack(setup: Setup) -> (String, Shared, tokio::task::JoinHandle<()>) {
        let seen = Shared::default();
        let routed = seen.clone();
        let (origin, task) = serve(move |origin| {
            let metadata = {
                let origin = origin.to_string();
                let setup = setup.clone();
                move || {
                    let mut document = json!({
                        "resource": setup.resource.clone().unwrap_or(origin.clone()),
                        "authorization_servers": setup.servers.clone().unwrap_or(json!([origin])),
                    });
                    if setup.advertise {
                        document["wk_mcp_client_id"] = json!(CLIENT);
                    }
                    async move { Json(document) }
                }
            };
            let behaviour = Behaviour {
                lifetime: 300,
                rotate: true,
                audience: setup
                    .audience
                    .clone()
                    .map(|a| if a == "origin" { json!(origin) } else { a }),
                issue_refresh: setup.issue_refresh,
            };
            provider(origin, &routed, behaviour)
                .route("/.well-known/oauth-protected-resource", get(metadata))
        })
        .await;
        (origin, seen, task)
    }
    fn ok() -> Setup {
        Setup {
            advertise: true,
            issue_refresh: true,
            ..Setup::default()
        }
    }
    fn options(origin: &str, ca: &[u8]) -> Options {
        Options {
            server: origin.into(),
            ca: Some(wk_summarize::tls::parse_ca_bundle(ca).unwrap()),
            client_id: None,
            offline_access: false,
        }
    }
    /// Run a whole login with a stand-in browser.
    async fn run(options: Options, store: &Memory, seen: &Shared) -> Result<Outcome, String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let browser_seen = seen.clone();
        let browser = tokio::spawn(async move {
            if let Ok(url) = rx.await {
                visit(&url, &browser_seen).await;
            }
        });
        let result = login(options, store, |url| {
            tx.send(url.to_string())
                .map_err(|_| "no browser".to_string())
        })
        .await;
        browser.abort();
        result
    }

    #[tokio::test]
    async fn login_discovers_the_provider_and_keeps_only_a_refresh_token() {
        let (origin, seen, task) = stack(ok()).await;
        let store = Memory::default();
        let outcome = run(options(&origin, &pki().ca), &store, &seen)
            .await
            .unwrap();
        assert_eq!(outcome.expires_in, 300);
        let stored = store.load().unwrap().unwrap();
        assert_eq!(
            (
                stored.issuer.as_str(),
                stored.client_id.as_str(),
                stored.resource.as_str(),
                stored.refresh_token.as_str()
            ),
            (origin.as_str(), CLIENT, origin.as_str(), "refresh-1")
        );
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.authorization["client_id"], CLIENT);
            assert_eq!(
                seen.authorization["resource"], origin,
                "the MCP resource is requested"
            );
            assert_eq!(seen.authorization["scope"], "openid");
            assert_eq!(seen.authorization["code_challenge_method"], "S256");
            assert!(seen.authorization["redirect_uri"].starts_with("http://127.0.0.1:"));
            assert_eq!(seen.tokens[0]["resource"], origin);
        }
        assert!(!serde_json::to_string(&summary(&origin, &outcome))
            .unwrap()
            .contains("access-1"));
        task.abort();
    }
    #[tokio::test]
    async fn an_explicit_client_and_offline_access_are_honoured() {
        let (origin, seen, task) = stack(Setup {
            advertise: false,
            ..ok()
        })
        .await;
        let store = Memory::default();
        let missing = run(options(&origin, &pki().ca), &store, &seen)
            .await
            .err()
            .unwrap();
        assert!(missing.contains("--client-id"), "{missing}");
        let mut chosen = options(&origin, &pki().ca);
        chosen.client_id = Some(CLIENT.into());
        chosen.offline_access = true;
        run(chosen, &store, &seen).await.unwrap();
        assert_eq!(
            seen.lock().unwrap().authorization["scope"],
            "openid offline_access"
        );
        task.abort();
    }
    #[tokio::test]
    async fn the_private_ca_is_required_and_failures_store_nothing() {
        let (origin, seen, task) = stack(ok()).await;
        let store = Memory::default();
        let mut public = options(&origin, &pki().ca);
        public.ca = None;
        let mut wrong = options(&origin, &pki().ca);
        wrong.ca = Some(wk_summarize::tls::parse_ca_bundle(&pki().other_ca).unwrap());
        for refused in [public, wrong] {
            let error = run(refused, &store, &seen).await.err().unwrap();
            assert!(error.contains("not trusted"), "{error}");
        }
        assert!(store.load().unwrap().is_none());
        assert!(run(
            Options {
                server: "http://wk.example".into(),
                ..options(&origin, &pki().ca)
            },
            &store,
            &seen
        )
        .await
        .is_err());
        task.abort();
    }
    #[tokio::test]
    async fn metadata_that_names_another_resource_or_several_servers_is_refused() {
        for setup in [
            Setup {
                resource: Some("https://elsewhere.example".into()),
                ..ok()
            },
            Setup {
                servers: Some(json!(["https://a.example", "https://b.example"])),
                ..ok()
            },
            Setup {
                servers: Some(json!([])),
                ..ok()
            },
        ] {
            let (origin, seen, task) = stack(setup).await;
            let store = Memory::default();
            assert!(run(options(&origin, &pki().ca), &store, &seen)
                .await
                .is_err());
            assert!(store.load().unwrap().is_none());
            task.abort();
        }
    }
    #[tokio::test]
    async fn a_token_for_the_wrong_audience_or_without_refresh_is_not_stored() {
        for (setup, expect) in [
            (
                Setup {
                    audience: Some(json!(["wk-api", "account"])),
                    ..ok()
                },
                "audience",
            ),
            (
                Setup {
                    audience: Some(json!("wk-api")),
                    ..ok()
                },
                "audience",
            ),
            (
                Setup {
                    issue_refresh: false,
                    ..ok()
                },
                "refresh token",
            ),
        ] {
            let (origin, seen, task) = stack(setup).await;
            let store = Memory::default();
            let error = run(options(&origin, &pki().ca), &store, &seen)
                .await
                .err()
                .unwrap();
            assert!(error.contains(expect), "{error}");
            assert!(store.load().unwrap().is_none());
            task.abort();
        }
        let (origin, seen, task) = stack(Setup {
            audience: Some(json!("origin")),
            ..ok()
        })
        .await;
        let store = Memory::default();
        run(options(&origin, &pki().ca), &store, &seen)
            .await
            .unwrap();
        assert!(
            store.load().unwrap().is_some(),
            "the exact resource audience is accepted"
        );
        task.abort();
    }
    #[test]
    fn audiences_are_read_only_from_jwt_shaped_tokens() {
        let jwt = |aud: Value| {
            let part =
                |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
            format!(
                "{}.{}.sig",
                part(json!({"alg":"none"})),
                part(json!({"aud":aud}))
            )
        };
        assert_eq!(audiences(&jwt(json!("a"))), Some(vec!["a".into()]));
        assert_eq!(
            audiences(&jwt(json!(["a", "b"]))),
            Some(vec!["a".into(), "b".into()])
        );
        assert_eq!(audiences("opaque-token"), None);
        assert_eq!(audiences("a.b"), None);
        assert_eq!(audiences(&jwt(json!(5))), None);
    }
}
