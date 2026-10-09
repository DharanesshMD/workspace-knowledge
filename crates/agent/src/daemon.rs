use crate::{network, state};
use axum::{
    extract::{rejection::JsonRejection, DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
#[derive(Clone)]
struct App {
    dir: PathBuf,
    auth: [u8; 32],
    events: Arc<AtomicU64>,
    prompts: Arc<AtomicU64>,
    responses: Arc<AtomicU64>,
    gaps: Arc<AtomicU64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hook {
    client: String,
    payload: Value,
}
fn authorized(a: &App, h: &HeaderMap) -> bool {
    if h.contains_key("origin") {
        return false;
    }
    let Some(token) = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    hash.iter()
        .zip(a.auth)
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}
async fn status(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    if !authorized(&a, &h) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Json(
        json!({"available":true,"events_received":a.events.load(Ordering::Relaxed),"prompts_captured":a.prompts.load(Ordering::Relaxed),"responses_captured":a.responses.load(Ordering::Relaxed),"gaps":a.gaps.load(Ordering::Relaxed),"coverage":"partial","auto_publish":false}),
    ))
}
async fn hook(
    State(a): State<App>,
    h: HeaderMap,
    body: Result<Json<Hook>, JsonRejection>,
) -> Result<Json<Value>, StatusCode> {
    if !authorized(&a, &h) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let Json(request) = body.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    match capture(&a.dir, &request.client, &request.payload).await {
        Ok(Some(kind)) => {
            a.events.fetch_add(1, Ordering::Relaxed);
            if kind == "user_prompt" {
                a.prompts.fetch_add(1, Ordering::Relaxed);
            }
            if kind == "assistant_response" {
                a.responses.fetch_add(1, Ordering::Relaxed);
            }
            Ok(Json(json!({"accepted":true})))
        }
        Ok(None) => Ok(Json(json!({"ignored":true}))),
        Err(_) => {
            a.gaps.fetch_add(1, Ordering::Relaxed);
            let _ = state::save(&a.dir.join("capture-gap.json"), &json!({"gap":true}));
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
pub fn now() -> Result<i64, String> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "clock unavailable")?
        .as_secs() as i64)
}
pub async fn capture(dir: &Path, client: &str, payload: &Value) -> Result<Option<String>, String> {
    if client == "cursor" && dir.join("cursor-consent.json").exists() {
        crate::cursor_capture::receive(dir, payload).await?;
        return Ok(Some("native_observation".into()));
    }
    if let Ok(native) = crate::native_enrollment::parse_client(client) {
        crate::native_capture::receive(dir, native, payload).await?;
        return Ok(Some("native_observation".into()));
    }
    let Some(o) = wk_adapters::normalize(client, payload)? else {
        return Ok(None);
    };
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let token = state::token(dir, "server-token")?;
    network::policy(&c, &b, &token).await?;
    let mut store = state::store(dir)?;
    let primary = c.root.join(&c.service);
    let outside = o.paths.iter().find(|p| {
        **p != c.root
            && (!b.eligible(p) || !p.canonicalize().is_ok_and(|p| p.starts_with(&primary)))
    });
    let kind = if outside.is_some() {
        "capture_gap".to_string()
    } else {
        o.kind.clone()
    };
    let path = outside.cloned().unwrap_or(primary);
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(payload).map_err(|_| "invalid hook")?)
    );
    let turn = o.turn.unwrap_or_else(|| "unattributed-turn".into());
    let event = json!({"schema_version":1,"event_id":digest,"session_id":o.session,"agent_id":o.agent,"turn_id":turn,"sequence":match o.kind.as_str(){"user_prompt"=>1,"assistant_response"=>2,"turn_end"=>3,_=>0},"kind":kind,"path":path,"text":if outside.is_some(){String::new()}else{o.text}});
    wk_capture::journal::ingest(&mut store, &b, &event.to_string(), now()?)?;
    Ok(Some(o.kind))
}
fn ipc(dir: &Path) -> Result<String, String> {
    Ok(state::secret(dir, "ipc-key")?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub async fn send(dir: &Path, value: Value) -> Result<(), String> {
    let c = state::load(dir)?;
    let r = network::client(1)?
        .post(format!("http://127.0.0.1:{}/capture", c.port))
        .bearer_auth(ipc(dir)?)
        .json(&value)
        .send()
        .await
        .map_err(|_| "collector unavailable")?;
    if !r.status().is_success() {
        return Err("collector rejected event".into());
    }
    Ok(())
}
pub async fn health(dir: &Path) -> Result<Value, String> {
    let c = state::load(dir)?;
    let r = network::client(1)?
        .get(format!("http://127.0.0.1:{}/health", c.port))
        .bearer_auth(ipc(dir)?)
        .send()
        .await
        .map_err(|_| "collector unavailable")?;
    wk_summarize::bounded_json(r, 65536).await
}
async fn retention_loop(mut store: wk_local::LocalStore) {
    let mut ticks = tokio::time::interval(std::time::Duration::from_secs(15));
    loop {
        ticks.tick().await;
        if let Ok(now) = now() {
            let _ = store.purge_expired(now);
        }
    }
}
pub async fn serve(dir: PathBuf) -> Result<Value, String> {
    let c = state::load(&dir)?;
    state::binding(&c)?;
    state::store(&dir)?;
    let auth = Sha256::digest(ipc(&dir)?.as_bytes()).into();
    let a = App {
        dir: dir.clone(),
        auth,
        events: Arc::new(AtomicU64::new(0)),
        prompts: Arc::new(AtomicU64::new(0)),
        responses: Arc::new(AtomicU64::new(0)),
        gaps: Arc::new(AtomicU64::new(0)),
    };
    let router = Router::new()
        .route("/capture", post(hook))
        .route("/health", get(status))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(a);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, c.port))
        .await
        .map_err(|_| "collector port unavailable")?;
    let retention = tokio::spawn(retention_loop(state::store(&dir)?));
    let background = tokio::spawn(async move {
        let mut ticks = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            ticks.tick().await;
            let _ = crate::process::run(&dir).await;
        }
    });
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|_| "collector stopped unexpectedly".to_string());
    background.abort();
    retention.abort();
    result?;
    Ok(json!({"stopped":true}))
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    #[tokio::test]
    async fn expiry_runs_while_inference_is_pending() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("expiry.db");
        let at = now().unwrap();
        let mut store = wk_local::LocalStore::open(&db, [7; 32], 100000).unwrap();
        store
            .enqueue("native:test", b"private", at - 10, 1)
            .unwrap();
        assert!(store.read_record("native:test", at - 10).unwrap().is_some());
        let task = tokio::spawn(retention_loop(store));
        tokio::time::timeout(
            std::time::Duration::from_millis(30),
            std::future::pending::<()>(),
        )
        .await
        .unwrap_err();
        let store = wk_local::LocalStore::open(&db, [7; 32], 100000).unwrap();
        // Read at the old timestamp to prove physical purge, not read-time filtering.
        assert!(store.read_record("native:test", at - 10).unwrap().is_none());
        task.abort();
    }
}
