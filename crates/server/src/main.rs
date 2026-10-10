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
    if mode == "healthcheck" {
        // Fixed local destination, no environment-supplied URL or credential forwarding.
        reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .redirect(reqwest::redirect::Policy::none())
            .build()?
            .get("http://127.0.0.1:8080/health/ready")
            .send()
            .await?
            .error_for_status()?;
        return Ok(());
    }
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
    if mode == "retention-once" {
        let counts = Knowledge::open(&url).await?.retention_batch(1000).await?;
        println!("{}", serde_json::to_string(&counts)?);
        return Ok(());
    }
    if mode == "replay-personal-deletions" {
        Knowledge::open(&url)
            .await?
            .replay_personal_deletions()
            .await?;
        return Ok(());
    }
    if mode == "retention-worker" {
        let k = Knowledge::open(&url).await?;
        wk_server::retention_worker::run(
            &k,
            async {
                let _ = tokio::signal::ctrl_c().await;
            },
            std::time::Duration::from_secs(60),
        )
        .await?;
        return Ok(());
    }
    if mode == "admin-assign" {
        let p: wk_server::admin_cli::AdminAssignment =
            wk_server::admin_cli::parse(std::io::stdin().lock())?;
        Knowledge::open(&url)
            .await?
            .admin_assign(
                &Actor {
                    org: p.org,
                    user: p.user,
                },
                &p.product,
                &p.services,
            )
            .await?;
        return Ok(());
    }
    if mode == "admin-policy-bootstrap" {
        let p: wk_server::admin_cli::PolicyBootstrap =
            wk_server::admin_cli::parse(std::io::stdin().lock())?;
        Knowledge::open(&url)
            .await?
            .admin_policy_bootstrap(
                &Actor {
                    org: p.org,
                    user: p.user,
                },
                &p.product,
                p.policy,
            )
            .await?;
        return Ok(());
    }
    if mode == "admin" || mode == "--dev-admin" {
        let input = wk_server::admin_cli::parse(std::io::stdin().lock())?;
        let verifier = if mode == "admin" {
            wk_server::auth::Oidc::load(
                required("WK_OIDC_ISSUER")?,
                required("WK_OIDC_AUDIENCE")?,
                required("WK_OIDC_JWKS_URL")?,
            )
            .await?
        } else {
            let name = url
                .split('?')
                .next()
                .unwrap_or_default()
                .rsplit('/')
                .next()
                .unwrap_or_default();
            if name != "wk_test" && name != "wk_dev" {
                return Err(
                    "development administration requires isolated wk_test or wk_dev".into(),
                );
            }
            wk_server::auth::Oidc::load_for_development(
                required("WK_ADMIN_TEST_ISSUER")?,
                required("WK_ADMIN_TEST_AUDIENCE")?,
                required("WK_ADMIN_TEST_JWKS_URL")?,
            )
            .await?
        };
        let result =
            wk_server::admin_cli::execute(&Knowledge::open(&url).await?, &verifier, input).await?;
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }
    let origin = std::env::var("WK_CONSOLE_ORIGIN").ok();
    if let Some(ref value) = origin {
        let u = reqwest::Url::parse(value)?;
        if !["https", "http"].contains(&u.scheme()) || u.origin().ascii_serialization() != *value {
            return Err("WK_CONSOLE_ORIGIN must be an exact origin without a path".into());
        }
    }
    let limits = wk_server::runtime::Limits::from_env()?;
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
                Knowledge::open_with_config(&url, limits.pool.clone()).await?,
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
            let mut tokens = vec![(token, who)];
            // Optional second development identity for multi-developer testing.
            if let Ok(second) = std::env::var("WK_DEV_TOKEN_B") {
                if second.len() < 32 {
                    return Err("WK_DEV_TOKEN_B must contain at least 32 characters".into());
                }
                let b = Actor {
                    org: "development".into(),
                    user: "developer-b".into(),
                };
                if k.policy(&b, "demo").await.is_err() {
                    k.grant(
                        &b,
                        "demo",
                        &["backend".into(), "frontend".into(), "ai-engine".into()],
                        false,
                    )
                    .await?
                }
                tokens.push((second, b));
            }
            (k, tokens, None, format!("127.0.0.1:{port}"))
        }
        _ => return Err("unknown mode".into()),
    };
    let browser = if let Ok(client_id) = std::env::var("WK_BROWSER_CLIENT_ID") {
        let browser_origin = origin
            .clone()
            .ok_or("WK_CONSOLE_ORIGIN required for browser login")?;
        let issuer = if mode == "--dev" {
            required("WK_BROWSER_TEST_ISSUER")?
        } else {
            required("WK_OIDC_ISSUER")?
        };
        Some(
            wk_server::browser_auth::BrowserConfig::load(
                browser_origin,
                issuer,
                client_id,
                std::env::var("WK_BROWSER_CLIENT_SECRET").ok(),
                mode == "--dev",
            )
            .await
            .map_err(|e| format!("Browser identity initialization failed: {e}"))?,
        )
    } else {
        None
    };
    let collector = if let Ok(audience) = std::env::var("WK_COLLECTOR_AUDIENCE") {
        if mode != "--production" {
            return Err("collector audience configuration requires production identity; use separate synthetic collector tokens in development".into());
        }
        Some(wk_server::device_routes::CollectorAuth {
            origin: origin
                .clone()
                .ok_or("WK_CONSOLE_ORIGIN required for collector binding")?,
            oidc: Some(
                wk_server::auth::Oidc::load(
                    required("WK_OIDC_ISSUER")?,
                    audience,
                    required("WK_OIDC_JWKS_URL")?,
                )
                .await?,
            ),
            tokens: vec![],
            client_id: Some(required("WK_COLLECTOR_CLIENT_ID")?),
        })
    } else if mode == "--dev" && std::env::var("WK_COLLECTOR_TEST_ISSUER").is_ok() {
        if std::env::var("WK_COLLECTOR_TEST_TOKEN").is_ok() {
            return Err("choose one synthetic collector authentication mode".into());
        }
        Some(wk_server::device_routes::CollectorAuth {
            origin: origin
                .clone()
                .ok_or("WK_CONSOLE_ORIGIN required for test collector")?,
            oidc: Some(
                wk_server::auth::Oidc::load_for_development(
                    required("WK_COLLECTOR_TEST_ISSUER")?,
                    required("WK_COLLECTOR_TEST_AUDIENCE")?,
                    required("WK_COLLECTOR_TEST_JWKS_URL")?,
                )
                .await?,
            ),
            tokens: vec![],
            client_id: Some(required("WK_COLLECTOR_TEST_CLIENT_ID")?),
        })
    } else if mode == "--dev" {
        std::env::var("WK_COLLECTOR_TEST_TOKEN").ok().map(|token| {
            wk_server::device_routes::CollectorAuth {
                origin: origin.clone().unwrap_or_default(),
                oidc: None,
                tokens: vec![(
                    token,
                    Actor {
                        org: "development".into(),
                        user: "developer".into(),
                    },
                )],
                client_id: None,
            }
        })
    } else {
        None
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
    let collector_refresh = collector.as_ref().and_then(|c|c.oidc.clone()).map(|verifier| {
        tokio::spawn(async move {let mut interval=tokio::time::interval(wk_server::auth::REFRESH_INTERVAL);interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);interval.tick().await;loop {interval.tick().await;if verifier.refresh().await.is_err(){eprintln!("Collector OIDC key refresh failed; cached keys expire after five minutes");}}})
    });
    let mut router = wk_server::portal_router_with_collector(
        knowledge, tokens, oidc, origin, browser, collector,
    )?;
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
    if let Some(worker) = collector_refresh {
        worker.abort();
    }
    served?;
    Ok(())
}
