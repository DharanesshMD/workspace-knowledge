//! Synthetic load fixture only; never the deployment entrypoint.
use wk_knowledge::Knowledge;

async fn verifiers(
    keys: jsonwebtoken::jwk::JwkSet,
    bind: &str,
) -> Result<
    (
        wk_server::auth::Oidc,
        wk_server::auth::Oidc,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let issuer = format!("http://{}", listener.local_addr()?);
    let jwks = axum::Router::new().route(
        "/jwks",
        axum::routing::get(move || {
            let keys = keys.clone();
            async move { axum::Json(keys) }
        }),
    );
    let worker = tokio::spawn(async move {
        let _ = axum::serve(listener, jwks).await;
    });
    let ordinary = wk_server::auth::Oidc::load_for_development(
        issuer.clone(),
        "https://pilot.synthetic.example".into(),
        format!("{issuer}/jwks"),
    )
    .await?;
    let collector = wk_server::auth::Oidc::load_for_development(
        issuer.clone(),
        "wk-pilot-collector".into(),
        format!("{issuer}/jwks"),
    )
    .await?;
    Ok((ordinary, collector, worker))
}

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
    // Port is confined to this newly owned container's loopback namespace.
    let (ordinary, collector, jwks_worker) = verifiers(keys, "127.0.0.1:41417").await?;
    let refresh_ordinary = ordinary.clone();
    let refresh_collector = collector.clone();
    let refresh_worker = tokio::spawn(async move {
        let mut interval = tokio::time::interval(wk_server::auth::REFRESH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            if refresh_ordinary.refresh().await.is_err()
                || refresh_collector.refresh().await.is_err()
            {
                eprintln!("Synthetic JWKS refresh failed; normal key expiry remains enforced");
            }
        }
    });
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
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    refresh_worker.abort();
    jwks_worker.abort();
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn owned_loopback_jwks_supports_both_audience_refreshes() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/synthetic-rsa-a.json")).unwrap();
        let keys = serde_json::from_value(serde_json::json!({"keys":[{"kty":"RSA","kid":"synthetic-pilot","alg":"RS256","use":"sig","n":fixture["n"],"e":fixture["e"]}]})).unwrap();
        let (ordinary, collector, worker) = super::verifiers(keys, "127.0.0.1:0").await.unwrap();
        assert_eq!(ordinary.issuer(), collector.issuer());
        assert_ne!(ordinary.audience(), collector.audience());
        ordinary.refresh().await.unwrap();
        collector.refresh().await.unwrap();
        worker.abort();
    }
}
