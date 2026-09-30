use wk_knowledge::{Actor, Knowledge};
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() != Some("--dev") {
        return Err(
            "Only --dev loopback mode is available; production OIDC is not implemented".into(),
        );
    }
    let url = std::env::var("WK_DATABASE_URL").map_err(|_| "WK_DATABASE_URL required")?;
    let name = url
        .split('?')
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default();
    if name != "wk_test" && name != "wk_dev" {
        return Err("Development server requires a dedicated wk_test or wk_dev database".into());
    }
    let token = std::env::var("WK_DEV_TOKEN").map_err(|_| "WK_DEV_TOKEN required")?;
    if token.len() < 32 {
        return Err("WK_DEV_TOKEN must contain at least 32 characters".into());
    }
    let knowledge = Knowledge::connect(&url).await?;
    let who = Actor {
        org: "development".into(),
        user: "developer".into(),
    };
    knowledge
        .grant(
            &who,
            "demo",
            &["backend".into(), "frontend".into(), "ai-engine".into()],
            true,
        )
        .await?;
    let port = std::env::var("WK_PORT")
        .unwrap_or_else(|_| "0".into())
        .parse::<u16>()
        .map_err(|_| "invalid WK_PORT")?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!(
        "Development API listening at http://{} (synthetic data only)",
        listener.local_addr()?
    );
    axum::serve(listener, wk_server::router(knowledge, vec![(token, who)]))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
