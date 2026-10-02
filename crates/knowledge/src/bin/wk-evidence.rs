use std::io::Read;
use wk_knowledge::{
    signed_evidence::{Envelope, Trust},
    Actor, Knowledge,
};
#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("Evidence rejected or service unavailable; no payload logged");
        std::process::exit(1)
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let policy = args.next().ok_or("trust policy path required")?;
    if args.next().is_some() {
        return Err("expected trust policy path only; envelope on stdin".into());
    }
    let mut trust = Vec::new();
    std::fs::File::open(policy)?
        .take(65537)
        .read_to_end(&mut trust)?;
    if trust.len() > 65536 {
        return Err("policy too large".into());
    }
    let trust: Trust = serde_json::from_slice(&trust)?;
    let mut bytes = Vec::new();
    std::io::stdin().take(32769).read_to_end(&mut bytes)?;
    if bytes.len() > 32768 {
        return Err("envelope too large".into());
    }
    let envelope: Envelope = serde_json::from_slice(&bytes)?;
    let actor = Actor {
        org: std::env::var("WK_OPERATOR_ORG")?,
        user: std::env::var("WK_OPERATOR_USER")?,
    };
    let k = Knowledge::open(&std::env::var("WK_DATABASE_URL")?).await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        .try_into()?;
    trust.ingest(&k, &actor, envelope, now).await?;
    println!("Signed evidence accepted");
    Ok(())
}
