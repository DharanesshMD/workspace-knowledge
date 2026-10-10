//! Team overlap notices and activity heartbeats for agent hooks.
//! Best effort: a slow or unavailable server never blocks the developer's agent.
use crate::{network, state};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};
use wk_adapters::team_context;

const PAUSED: &str = "activity-paused";

fn product_path(product: &str, tail: &str) -> String {
    format!("/v1/products/{product}/{tail}")
}

async fn call(
    c: &state::Config,
    token: &str,
    method: reqwest::Method,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    crate::device::require_device_for_remote(&c.server, c.device.is_some())?;
    if c.device.is_some() {
        let b = state::base_binding(c)?;
        let operation = if path == product_path(&b.product, "overlaps")
            && method == reqwest::Method::POST
        {
            "overlaps"
        } else if path == product_path(&b.product, "activity") && method == reqwest::Method::POST {
            "activity"
        } else if path == product_path(&b.product, "activity") && method == reqwest::Method::DELETE
        {
            "activity-clear"
        } else {
            return Err("unsupported collector operation".into());
        };
        return crate::device::signed(c, token, operation, body, 2).await;
    }
    let r = network::client(2)?
        .request(method, format!("{}{path}", network::server_url(&c.server)?))
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .map_err(|_| "knowledge service unavailable")?;
    wk_summarize::bounded_json(r, 256 * 1024).await
}

/// Handle a hook event; returns context to inject, if any. Errors are swallowed by the caller.
pub async fn on_hook(
    dir: &Path,
    client: &str,
    event: &str,
    payload: &Value,
) -> Result<Option<String>, String> {
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let plan = team_context::plan(client, event, payload, &b.root);
    let wants_lookup = !plan.lookup_files.is_empty() || plan.lookup_keywords.is_some();
    if !wants_lookup && plan.heartbeat_files.is_empty() {
        return Ok(None);
    }
    let token = state::token(dir, "server-token")?;
    let session = team_context::session(payload).unwrap_or("unknown");
    let session_key = format!("{:x}", Sha256::digest(session.as_bytes()));

    // Share only approved, workspace-relative paths; skip entirely while paused.
    let files: Vec<String> = plan
        .heartbeat_files
        .into_iter()
        .filter(|f| {
            let path = b.root.join(f);
            b.eligible(&path) || path.parent().is_some_and(|p| b.eligible(p))
        })
        .collect();
    if !files.is_empty() && !dir.join(PAUSED).exists() {
        let folders: BTreeSet<&str> = files
            .iter()
            .map(|f| f.rsplit_once('/').map_or(".", |(d, _)| d))
            .collect();
        let intent = format!(
            "Editing files in {}",
            folders.into_iter().take(3).collect::<Vec<_>>().join(", ")
        );
        let body = json!({"session_id": &session_key[..32], "services": [c.service], "files": files, "intent": intent.chars().take(280).collect::<String>()});
        let _ = call(
            &c,
            &token,
            reqwest::Method::POST,
            &product_path(&b.product, "activity"),
            &body,
        )
        .await;
    }
    if !wants_lookup {
        return Ok(None);
    }
    // Keywords are prompt-derived: screen them locally and drop them if they look sensitive.
    let query = plan.lookup_keywords.filter(|k| !wk_summarize::sensitive(k));
    if plan.lookup_files.is_empty() && query.is_none() {
        return Ok(None);
    }
    let result = call(
        &c,
        &token,
        reqwest::Method::POST,
        &product_path(&b.product, "overlaps"),
        &json!({"files": plan.lookup_files, "query": query, "limit": 6}),
    )
    .await?;
    let notices = result["notices"].as_array().cloned().unwrap_or_default();
    // Remember what this session has been told, so notices are not repeated.
    let seen_file = dir.join(format!("notices-{}.json", &session_key[..32]));
    let mut seen: BTreeSet<String> = state::read(&seen_file, 256 * 1024)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let fresh = team_context::unseen(&notices, &mut seen);
    if !fresh.is_empty() {
        state::save(&seen_file, &seen)?;
    }
    Ok(team_context::render(&fresh))
}

/// `activity pause|resume|status`: stop or restart sharing this developer's live activity.
pub async fn activity(dir: &Path, action: &str) -> Result<Value, String> {
    let flag = dir.join(PAUSED);
    match action {
        "pause" => {
            state::save(&flag, &json!({"paused": true}))?;
            let c = state::load(dir)?;
            let b = state::binding(&c)?;
            let token = state::token(dir, "server-token")?;
            let cleared = call(
                &c,
                &token,
                reqwest::Method::DELETE,
                &product_path(&b.product, "activity"),
                &json!({}),
            )
            .await
            .is_ok();
            Ok(json!({"sharing": false, "server_activity_cleared": cleared}))
        }
        "resume" => {
            if flag.exists() {
                std::fs::remove_file(&flag).map_err(|_| "cannot clear pause flag")?;
            }
            Ok(json!({"sharing": true}))
        }
        "status" => Ok(json!({"sharing": !flag.exists()})),
        _ => Err("use pause, resume or status".into()),
    }
}
