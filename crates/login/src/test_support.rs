//! Test-only HTTPS server whose certificate chains to a throwaway private CA.
use axum::http::StatusCode;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{
    rustls::{
        pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
        ServerConfig,
    },
    server::TlsStream,
    TlsAcceptor,
};

/// Throwaway PKI generated per test process, so no key material lives in the repository.
pub struct Pki {
    pub ca: Vec<u8>,
    pub other_ca: Vec<u8>,
    leaf: Vec<u8>,
    key: Vec<u8>,
}
pub fn pki() -> &'static Pki {
    static PKI: std::sync::OnceLock<Pki> = std::sync::OnceLock::new();
    PKI.get_or_init(|| {
        let authority = |name: &str| {
            let key = rcgen::KeyPair::generate().unwrap();
            let mut params = rcgen::CertificateParams::new(Vec::new()).unwrap();
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, name);
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            params.key_usages = vec![
                rcgen::KeyUsagePurpose::KeyCertSign,
                rcgen::KeyUsagePurpose::CrlSign,
            ];
            let certificate = params.self_signed(&key).unwrap();
            (certificate, params, key)
        };
        let (ca, ca_params, ca_key) = authority("WK Login Test CA");
        let (other, _, _) = authority("WK Login Unrelated CA");
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf = rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])
            .unwrap()
            .signed_by(&leaf_key, &rcgen::Issuer::new(ca_params, ca_key))
            .unwrap();
        Pki {
            ca: ca.pem().into_bytes(),
            other_ca: other.pem().into_bytes(),
            leaf: leaf.pem().into_bytes(),
            key: leaf_key.serialize_pem().into_bytes(),
        }
    })
}

struct Tls {
    tcp: TcpListener,
    acceptor: TlsAcceptor,
}
impl axum::serve::Listener for Tls {
    type Io = TlsStream<TcpStream>;
    type Addr = std::net::SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let Ok((stream, address)) = self.tcp.accept().await else {
                continue;
            };
            // A client that rejects the certificate aborts the handshake; keep serving.
            if let Ok(stream) = self.acceptor.accept(stream).await {
                return (stream, address);
            }
        }
    }
    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}

/// Serve the router built from the `https://127.0.0.1:port` origin over TLS on that port.
pub async fn serve(
    router: impl FnOnce(&str) -> axum::Router,
) -> (String, tokio::task::JoinHandle<()>) {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", tcp.local_addr().unwrap());
    let chain = CertificateDer::pem_slice_iter(&pki().leaf)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_slice(&pki().key).unwrap();
    let config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .unwrap();
    let listener = Tls {
        tcp,
        acceptor: TlsAcceptor::from(Arc::new(config)),
    };
    let router = router(&origin);
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (origin, task)
}

// ---- a synthetic OIDC provider ------------------------------------------------------------
use crate::{Attempt, Tokens};
use axum::{
    extract::State,
    routing::{get, post},
    Form, Json,
};
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub const CLIENT: &str = "login-test";
type Params = HashMap<String, String>;
/// What the provider saw and handed out; shared with the test that drives it.
#[derive(Default)]
pub struct Seen {
    /// The query of the authorization request the "browser" was sent.
    pub authorization: Params,
    /// Every request to the token endpoint, in order.
    pub tokens: Vec<Params>,
    /// Refresh tokens currently usable.
    pub valid: HashSet<String>,
    /// How many access tokens have been issued; they are named `access-N`.
    pub issued: u32,
    /// The most recent access token issued.
    pub latest_access: String,
}
pub type Shared = Arc<Mutex<Seen>>;
#[derive(Clone)]
pub struct Behaviour {
    /// `expires_in` of issued access tokens.
    pub lifetime: u64,
    /// Whether each refresh replaces the refresh token (and revokes the old one).
    pub rotate: bool,
    /// When set, access tokens are JWT-shaped with this `aud`; otherwise they are opaque.
    pub audience: Option<Value>,
    /// Whether the authorization-code exchange returns a refresh token at all.
    pub issue_refresh: bool,
}
fn key() -> Value {
    serde_json::from_str(include_str!(
        "../../server/tests/fixtures/synthetic-rsa-a.json"
    ))
    .unwrap()
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn id_token(issuer: &str, nonce: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test".into());
    encode(
        &header,
        &json!({"iss":issuer,"aud":CLIENT,"sub":"synthetic-developer","nonce":nonce,"iat":now(),"exp":now()+300}),
        &EncodingKey::from_rsa_pem(key()["private_pkcs8_pem"].as_str().unwrap().as_bytes())
            .unwrap(),
    )
    .unwrap()
}
fn access_token(seen: &mut Seen, behaviour: &Behaviour) -> String {
    seen.issued += 1;
    let token = match &behaviour.audience {
        None => format!("access-{}", seen.issued),
        Some(aud) => {
            let part =
                |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
            format!(
                "{}.{}.signature",
                part(json!({"alg":"none"})),
                part(json!({"aud":aud,"jti":seen.issued}))
            )
        }
    };
    seen.latest_access = token.clone();
    token
}
/// Discovery, keys and a token endpoint for [`CLIENT`], on `issuer`. Refresh tokens are
/// `refresh-N`; anything else is `invalid_grant`, and `flaky` answers 503.
pub fn provider(issuer: &str, seen: &Shared, behaviour: Behaviour) -> axum::Router {
    let issuer = issuer.to_string();
    let metadata_issuer = issuer.clone();
    axum::Router::new()
        .route("/.well-known/openid-configuration", get(move || {
            let issuer = metadata_issuer.clone();
            async move {
                Json(json!({"issuer":issuer,"authorization_endpoint":format!("{issuer}/authorize"),"token_endpoint":format!("{issuer}/token"),"jwks_uri":format!("{issuer}/keys"),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"]}))
            }
        }))
        .route("/keys", get(|| async {
            let key = key();
            Json(json!({"keys":[{"kty":"RSA","kid":"test","alg":"RS256","use":"sig","n":key["n"],"e":key["e"]}]}))
        }))
        .route("/token", post(move |State(seen): State<Shared>, Form(body): Form<Params>| {
            let (issuer, behaviour) = (issuer.clone(), behaviour.clone());
            async move {
                let mut seen = seen.lock().unwrap();
                seen.tokens.push(body.clone());
                let refresh_for = |seen: &mut Seen, n: u32| {
                    let token = format!("refresh-{n}");
                    seen.valid.insert(token.clone());
                    token
                };
                let ok = StatusCode::OK;
                match body["grant_type"].as_str() {
                    "authorization_code" => {
                        let nonce = seen.authorization.get("nonce").cloned().unwrap_or_default();
                        let access = access_token(&mut seen, &behaviour);
                        let mut reply = json!({"access_token":access,"token_type":"Bearer","expires_in":behaviour.lifetime,"id_token":id_token(&issuer,&nonce)});
                        if behaviour.issue_refresh {
                            reply["refresh_token"] = json!(refresh_for(&mut seen, 1));
                        }
                        (ok, Json(reply))
                    }
                    "refresh_token" if body["refresh_token"] == "flaky" => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":"temporarily_unavailable"}))),
                    "refresh_token" if seen.valid.contains(&body["refresh_token"]) => {
                        let access = access_token(&mut seen, &behaviour);
                        let mut reply = json!({"access_token":access,"token_type":"Bearer","expires_in":behaviour.lifetime});
                        if behaviour.rotate {
                            let number: u32 = body["refresh_token"].trim_start_matches("refresh-").parse().unwrap();
                            seen.valid.remove(&body["refresh_token"]);
                            reply["refresh_token"] = json!(refresh_for(&mut seen, number + 1));
                        }
                        (ok, Json(reply))
                    }
                    _ => (StatusCode::BAD_REQUEST, Json(json!({"error":"invalid_grant"}))),
                }
            }
        }))
        .with_state(seen.clone())
}
/// Play the system browser: record the authorization request, then deliver the callback.
pub async fn visit(url: &str, seen: &Shared) {
    let url = reqwest::Url::parse(url).unwrap();
    let values: Params = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    seen.lock().unwrap().authorization = values.clone();
    let mut callback = reqwest::Url::parse(&values["redirect_uri"]).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("code", "code")
        .append_pair("state", &values["state"]);
    let answer = crate::client_trusting(None, 3)
        .unwrap()
        .get(callback)
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), reqwest::StatusCode::OK);
}
/// Complete a started login the way the system browser would.
pub async fn browser(attempt: Attempt, seen: &Shared) -> Result<Tokens, String> {
    let url = attempt.url.clone();
    let completion = tokio::spawn(attempt.complete());
    visit(&url, seen).await;
    completion.await.unwrap()
}
