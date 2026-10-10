//! Synthetic load fixture only; never the deployment entrypoint.
use wk_knowledge::Knowledge;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("WK_DATABASE_URL")?;
    let parsed = reqwest::Url::parse(&url)?;
    if !parsed.path().starts_with("/wk_pilot_") {
        return Err("pilot requires its newly allocated synthetic database".into());
    }
    let limits = wk_server::runtime::Limits::from_env()?;
    let k = Knowledge::open_with_config(&url, limits.pool).await?;
    let keys: jsonwebtoken::jwk::JwkSet =
        serde_json::from_str(&std::env::var("WK_PILOT_PUBLIC_JWKS")?)?;
    let issuer = "https://synthetic-pilot.example".to_string();
    let ordinary = wk_server::auth::Oidc::from_jwks(
        issuer.clone(),
        "https://pilot.synthetic.example".into(),
        keys.clone(),
    )?;
    let collector = wk_server::auth::Oidc::from_jwks(issuer, "wk-pilot-collector".into(), keys)?;
    let origin = std::env::var("WK_PILOT_ORIGIN")?;
    let app = wk_server::portal_router_with_collector(
        k,
        vec![],
        Some(ordinary),
        Some(origin.clone()),
        None,
        Some(wk_server::device_routes::CollectorAuth {
            origin,
            oidc: Some(collector),
            tokens: vec![],
            client_id: Some("synthetic-pilot-native".into()),
        }),
    )?;
    let listener = tokio::net::TcpListener::bind(
        std::env::var("WK_PILOT_BIND").unwrap_or("127.0.0.1:8080".into()),
    )
    .await?;
    eprintln!("Synthetic pilot ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
