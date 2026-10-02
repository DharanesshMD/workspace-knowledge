use serde::Deserialize;
use wk_knowledge::{Actor, Knowledge};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provision {
    issuer: String,
    subject: String,
    org: String,
    user: String,
    product: String,
    #[serde(default)]
    services: Vec<String>,
    #[serde(default)]
    can_review: bool,
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    std::env::var(name).map_err(|_| format!("{name} required").into())
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args()
        .nth(1)
        .ok_or("choose --dev, --production, migrate, provision, revoke, or replay-tombstones")?;
    let url = required("WK_DATABASE_URL")?;
    if mode == "migrate" {
        Knowledge::open(&url).await?.migrate().await?;
        return Ok(());
    }
    if mode == "replay-tombstones" {
        Knowledge::open(&url).await?.replay_tombstones().await?;
        return Ok(());
    }
    if mode == "provision" || mode == "revoke" {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::io::stdin().take(65537).read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            return Err("provisioning input too large".into());
        }
        let p: Provision =
            serde_json::from_slice(&bytes).map_err(|_| "invalid provisioning JSON")?;
        let k = Knowledge::open(&url).await?;
        let a = Actor {
            org: p.org,
            user: p.user,
        };
        if mode == "revoke" {
            k.revoke(&a, &p.product).await?
        } else {
            k.provision_identity(&p.issuer, &p.subject, &a).await?;
            k.grant(&a, &p.product, &p.services, p.can_review).await?
        }
        return Ok(());
    }
    let origin = std::env::var("WK_CONSOLE_ORIGIN").ok();
    if let Some(ref value) = origin {
        let u = reqwest::Url::parse(value)?;
        if !["https", "http"].contains(&u.scheme()) || u.origin().ascii_serialization() != *value {
            return Err("WK_CONSOLE_ORIGIN must be an exact origin without a path".into());
        }
    }
    let (knowledge, tokens, oidc, address) = match mode.as_str() {
        "--production" => {
            let oidc = wk_server::auth::Oidc::load(
                required("WK_OIDC_ISSUER")?,
                required("WK_OIDC_AUDIENCE")?,
                required("WK_OIDC_JWKS_URL")?,
            )
            .await
            .map_err(|e| format!("OIDC initialization failed: {e}"))?;
            (
                Knowledge::open(&url).await?,
                vec![],
                Some(oidc),
                std::env::var("WK_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into()),
            )
        }
        "--dev" => {
            let name = url
                .split('?')
                .next()
                .unwrap_or_default()
                .rsplit('/')
                .next()
                .unwrap_or_default();
            if name != "wk_test" && name != "wk_dev" {
                return Err(
                    "Development server requires dedicated wk_test or wk_dev database".into(),
                );
            }
            let token = required("WK_DEV_TOKEN")?;
            if token.len() < 32 {
                return Err("WK_DEV_TOKEN must contain at least 32 characters".into());
            }
            let k = Knowledge::connect(&url).await?;
            let who = Actor {
                org: "development".into(),
                user: "developer".into(),
            };
            // Do not rotate policy on every development restart.
            if k.policy(&who, "demo").await.is_err() {
                k.grant(
                    &who,
                    "demo",
                    &["backend".into(), "frontend".into(), "ai-engine".into()],
                    true,
                )
                .await?
            }
            let port = std::env::var("WK_PORT")
                .unwrap_or_else(|_| "0".into())
                .parse::<u16>()
                .map_err(|_| "invalid WK_PORT")?;
            (k, vec![(token, who)], None, format!("127.0.0.1:{port}"))
        }
        _ => return Err("unknown mode".into()),
    };
    let listener = tokio::net::TcpListener::bind(&address).await?;
    eprintln!(
        "{mode} API listening at http://{} (authenticated)",
        listener.local_addr()?
    );
    let refresh_worker = oidc.clone().map(|verifier| {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(wk_server::auth::REFRESH_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                interval.tick().await;
                if verifier.refresh().await.is_err() {
                    eprintln!("OIDC key refresh failed; cached keys expire after five minutes");
                }
            }
        })
    });
    let mut router = wk_server::configured_router(knowledge, tokens, oidc, origin);
    if let Ok(dir) = std::env::var("WK_WEB_DIR") {
        router = router.fallback_service(
            tower_http::services::ServeDir::new(&dir).append_index_html_on_directories(true),
        );
    }
    router = router.layer(
        tower_http::set_header::SetResponseHeaderLayer::if_not_present(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        ),
    );
    router=router.layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(axum::http::header::CONTENT_SECURITY_POLICY,axum::http::HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'")));
    let served = axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    if let Some(worker) = refresh_worker {
        worker.abort();
    }
    served?;
    Ok(())
}
