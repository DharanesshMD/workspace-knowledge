//! Where the bridge gets its bearer token: a fixed environment token, or a stored sign-in that is
//! refreshed silently. Access tokens live only in memory.
use crate::store::{Store, Stored};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use wk_login::{RefreshError, Request};
use zeroize::Zeroizing;

/// A token this close to expiry is renewed before it is used.
const RENEW_MARGIN: Duration = Duration::from_secs(30);
const SIGN_IN_REQUIRED: &str = "not signed in; run `wk-mcp login`";
const SIGN_IN_EXPIRED: &str = "sign-in expired or revoked; run `wk-mcp login` again";

struct Cached {
    token: Zeroizing<String>,
    expires: Instant,
}
struct SignedIn {
    /// Allow a literal-loopback `http://` provider, as for a loopback development server.
    development: bool,
    ca: Option<Vec<reqwest::Certificate>>,
    store: Box<dyn Store>,
    cached: Mutex<Option<Cached>>,
}
enum Kind {
    /// No token could be found; asking for one gives the reason.
    Missing(String),
    Fixed(Zeroizing<String>),
    SignedIn(Box<SignedIn>),
}
pub struct TokenSource(Kind);

impl TokenSource {
    /// A token the operator supplied in the environment. It is never refreshed.
    pub fn fixed(token: String) -> Result<Self, String> {
        if token.len() < 32 {
            return Err("invalid development credentials or product".into());
        }
        Ok(Self(Kind::Fixed(Zeroizing::new(token))))
    }
    /// A source with nothing in it, for a caller that can still do useful local work (a hook).
    pub fn missing(reason: String) -> Self {
        Self(Kind::Missing(reason))
    }
    /// The stored sign-in for one server, refreshed against its identity provider. `ca`, when
    /// the operator set one, replaces the public roots for the provider as well.
    pub fn signed_in(
        development: bool,
        ca: Option<Vec<reqwest::Certificate>>,
        store: Box<dyn Store>,
    ) -> Self {
        Self(Kind::SignedIn(Box::new(SignedIn {
            development,
            ca,
            store,
            cached: Mutex::new(None),
        })))
    }
    pub async fn token(&self) -> Result<Zeroizing<String>, String> {
        match &self.0 {
            Kind::Missing(reason) => Err(reason.clone()),
            Kind::Fixed(token) => Ok(token.clone()),
            Kind::SignedIn(s) => s.token().await,
        }
    }
    /// The server refused `token`. Returns whether a retry with a newly obtained one is worthwhile.
    pub async fn rejected(&self, token: &str) -> bool {
        let Kind::SignedIn(s) = &self.0 else {
            return false;
        };
        let mut cached = s.cached.lock().await;
        // Another request may already have replaced the refused token.
        if cached.as_ref().is_some_and(|c| c.token.as_str() != token) {
            return true;
        }
        *cached = None;
        true
    }
}
impl SignedIn {
    async fn token(&self) -> Result<Zeroizing<String>, String> {
        let mut cached = self.cached.lock().await;
        if let Some(c) = cached.as_ref().filter(|c| c.expires > Instant::now()) {
            return Ok(c.token.clone());
        }
        let fresh = self.renew().await?;
        let token = fresh.token.clone();
        *cached = Some(fresh);
        Ok(token)
    }
    async fn renew(&self) -> Result<Cached, String> {
        let mut stored = self.store.load()?.ok_or(SIGN_IN_REQUIRED)?;
        let mut retried = false;
        loop {
            let request = Request {
                issuer: &stored.issuer,
                client_id: &stored.client_id,
                development: self.development,
                resource: None,
                scopes: &[],
                done_message: "",
            };
            match wk_login::refresh(&request, self.ca.as_deref(), &stored.refresh_token).await {
                Ok(tokens) => {
                    if let Some(rotated) = &tokens.refresh_token {
                        let next = Stored {
                            issuer: stored.issuer.clone(),
                            client_id: stored.client_id.clone(),
                            resource: stored.resource.clone(),
                            refresh_token: rotated.to_string(),
                        };
                        // The new token is already usable; losing it only forces a later login.
                        if self.store.save(&next).is_err() {
                            eprintln!("wk-mcp: could not store the renewed sign-in; run `wk-mcp login` if requests start failing");
                        }
                    }
                    let lifetime = Duration::from_secs(tokens.expires_in);
                    return Ok(Cached {
                        token: tokens.access_token,
                        expires: Instant::now() + lifetime.saturating_sub(RENEW_MARGIN),
                    });
                }
                Err(RefreshError::Rejected) => {
                    // A concurrent bridge may have rotated the token and stored its successor.
                    let current = self.store.load()?.ok_or(SIGN_IN_REQUIRED)?;
                    if retried || current.refresh_token == stored.refresh_token {
                        return Err(SIGN_IN_EXPIRED.into());
                    }
                    retried = true;
                    stored = current;
                }
                Err(RefreshError::Unavailable(_)) => {
                    return Err("identity provider unavailable".into())
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Memory;
    use std::sync::Arc;
    use wk_login::test_support::{pki, provider, serve, Behaviour, Shared, CLIENT};

    struct Fixture {
        origin: String,
        seen: Shared,
        store: Arc<Memory>,
        server: tokio::task::JoinHandle<()>,
    }
    /// The store is shared so a test can look inside, as another process would.
    struct Handle(Arc<Memory>);
    impl Store for Handle {
        fn load(&self) -> Result<Option<Stored>, String> {
            self.0.load()
        }
        fn save(&self, s: &Stored) -> Result<(), String> {
            self.0.save(s)
        }
        fn delete(&self) -> Result<(), String> {
            self.0.delete()
        }
    }
    fn stored(origin: &str, refresh: &str) -> Stored {
        Stored {
            issuer: origin.into(),
            client_id: CLIENT.into(),
            resource: origin.into(),
            refresh_token: refresh.into(),
        }
    }
    async fn fixture(lifetime: u64, rotate: bool) -> Fixture {
        let seen = Shared::default();
        let routed = seen.clone();
        let (origin, server) = serve(move |origin| {
            provider(
                origin,
                &routed,
                Behaviour {
                    lifetime,
                    rotate,
                    audience: None,
                    issue_refresh: true,
                },
            )
        })
        .await;
        seen.lock().unwrap().valid.insert("refresh-1".into());
        let store = Arc::new(Memory::default());
        store.save(&stored(&origin, "refresh-1")).unwrap();
        Fixture {
            origin,
            seen,
            store,
            server,
        }
    }
    fn source(f: &Fixture, ca: &[u8]) -> TokenSource {
        TokenSource::signed_in(
            false,
            Some(wk_summarize::tls::parse_ca_bundle(ca).unwrap()),
            Box::new(Handle(f.store.clone())),
        )
    }
    fn refreshes(f: &Fixture) -> usize {
        f.seen.lock().unwrap().tokens.len()
    }

    #[tokio::test]
    async fn a_fresh_access_token_is_reused_until_it_nears_expiry() {
        let f = fixture(300, true).await;
        let tokens = source(&f, &pki().ca);
        let first = tokens.token().await.unwrap();
        assert_eq!(first.as_str(), "access-1");
        assert_eq!(tokens.token().await.unwrap().as_str(), "access-1");
        assert_eq!(refreshes(&f), 1, "second call is served from memory");
        f.server.abort();

        let short = fixture(10, true).await;
        let tokens = source(&short, &pki().ca);
        assert_eq!(tokens.token().await.unwrap().as_str(), "access-1");
        assert_eq!(
            tokens.token().await.unwrap().as_str(),
            "access-2",
            "a token inside the renewal margin is replaced"
        );
        short.server.abort();
    }
    #[tokio::test]
    async fn a_rotated_refresh_token_replaces_the_stored_one() {
        let f = fixture(300, true).await;
        source(&f, &pki().ca).token().await.unwrap();
        let now = f.store.load().unwrap().unwrap();
        assert_eq!(now.refresh_token, "refresh-2");
        assert_eq!(
            (now.issuer.as_str(), now.client_id.as_str()),
            (f.origin.as_str(), CLIENT)
        );
        f.server.abort();
        let fixed = fixture(300, false).await;
        source(&fixed, &pki().ca).token().await.unwrap();
        assert_eq!(
            fixed.store.load().unwrap().unwrap().refresh_token,
            "refresh-1"
        );
        fixed.server.abort();
    }
    #[tokio::test]
    async fn a_refused_token_is_dropped_so_the_next_request_renews_it() {
        let f = fixture(300, true).await;
        let tokens = source(&f, &pki().ca);
        let refused = tokens.token().await.unwrap();
        assert!(tokens.rejected(&refused).await);
        let next = tokens.token().await.unwrap();
        assert_ne!(next.as_str(), refused.as_str());
        // A second complaint about the old token must not discard the new one.
        assert!(tokens.rejected(&refused).await);
        assert_eq!(tokens.token().await.unwrap().as_str(), next.as_str());
        let fixed = TokenSource::fixed("x".repeat(32)).unwrap();
        assert!(
            !fixed.rejected("x").await,
            "a fixed token cannot be renewed"
        );
        f.server.abort();
    }
    #[tokio::test]
    async fn missing_expired_and_unreachable_sign_ins_say_what_to_do() {
        let f = fixture(300, true).await;
        f.store.delete().unwrap();
        let error = source(&f, &pki().ca).token().await.err().unwrap();
        assert!(error.contains("wk-mcp login"), "{error}");
        f.store.save(&stored(&f.origin, "revoked")).unwrap();
        let error = source(&f, &pki().ca).token().await.err().unwrap();
        assert!(
            error.contains("expired") && error.contains("wk-mcp login"),
            "{error}"
        );
        // The wrong CA is a failure, never a fall back to public roots.
        f.store.save(&stored(&f.origin, "refresh-1")).unwrap();
        let wrong = source(&f, &pki().other_ca).token().await.err().unwrap();
        assert_eq!(wrong, "identity provider unavailable");
        let public = TokenSource::signed_in(false, None, Box::new(Handle(f.store.clone())));
        assert_eq!(
            public.token().await.err().unwrap(),
            "identity provider unavailable"
        );
        f.server.abort();
    }
    #[tokio::test]
    async fn a_token_rotated_by_another_bridge_is_picked_up_once() {
        let f = fixture(300, true).await;
        // Another bridge already used refresh-1; its successor is in the store but ours is stale.
        {
            let mut seen = f.seen.lock().unwrap();
            seen.valid.remove("refresh-1");
            seen.valid.insert("refresh-2".into());
        }
        let stale = stored(&f.origin, "refresh-1");
        let before = refreshes(&f);
        let tokens = TokenSource::signed_in(
            false,
            Some(wk_summarize::tls::parse_ca_bundle(&pki().ca).unwrap()),
            Box::new(Swap {
                first: std::sync::Mutex::new(Some(stale)),
                store: f.store.clone(),
                successor: "refresh-2".into(),
                origin: f.origin.clone(),
            }),
        );
        assert_eq!(tokens.token().await.unwrap().as_str(), "access-1");
        assert_eq!(refreshes(&f) - before, 2, "one refused attempt, one retry");
        f.server.abort();
    }
    /// Hands out a stale record once, then whatever the "other process" has stored since.
    struct Swap {
        first: std::sync::Mutex<Option<Stored>>,
        store: Arc<Memory>,
        successor: String,
        origin: String,
    }
    impl Store for Swap {
        fn load(&self) -> Result<Option<Stored>, String> {
            if let Some(stale) = self.first.lock().unwrap().take() {
                return Ok(Some(stale));
            }
            self.store.save(&stored(&self.origin, &self.successor))?;
            self.store.load()
        }
        fn save(&self, s: &Stored) -> Result<(), String> {
            self.store.save(s)
        }
        fn delete(&self) -> Result<(), String> {
            self.store.delete()
        }
    }
    #[tokio::test]
    async fn a_fixed_token_must_look_like_a_credential() {
        assert!(TokenSource::fixed("short".into()).is_err());
        let token = TokenSource::fixed("t".repeat(40)).unwrap();
        assert_eq!(token.token().await.unwrap().as_str(), "t".repeat(40));
    }
}
