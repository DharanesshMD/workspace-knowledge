//! Team overlap notices and activity heartbeats for agent hooks.
//! Best effort: a slow or unavailable server never blocks the developer's agent.
use crate::{network, state};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};
use wk_adapters::team_context;
use wk_contracts::change_context::{valid_targets, CodeTarget};

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

/// Bind the operator-created repository identity to the explicitly approved local checkout.
pub async fn enroll_repository(
    dir: &Path,
    repository_id: &str,
    requested_git_dir: Option<&Path>,
    yes: bool,
) -> Result<Value, String> {
    if !yes {
        return Err(
            "review the repository ID and local Git metadata boundary, then pass --yes".into(),
        );
    }
    let id = repository_id
        .parse::<uuid::Uuid>()
        .map_err(|_| "invalid repository ID")?;
    if id.is_nil() {
        return Err("invalid repository ID".into());
    }
    let mut config = state::load(dir)?;
    let binding = state::binding(&config)?;
    if config.root != binding.root || !binding.services.contains(&config.service) {
        return Err("repository is outside the approved workspace".into());
    }
    let dot_git = config.root.join(".git");
    let meta =
        std::fs::symlink_metadata(&dot_git).map_err(|_| "repository metadata unavailable")?;
    if meta.file_type().is_symlink() {
        return Err("repository metadata symlink denied".into());
    }
    let actual = if meta.is_dir() {
        dot_git
            .canonicalize()
            .map_err(|_| "Git directory unavailable")?
    } else if meta.is_file() {
        if requested_git_dir.is_none() {
            return Err("worktree Git directory must be explicitly approved with --git-dir".into());
        }
        let text = state::read(&dot_git, 4096)?;
        let target = text
            .trim()
            .strip_prefix("gitdir: ")
            .ok_or("invalid Git directory")?;
        config
            .root
            .join(target)
            .canonicalize()
            .map_err(|_| "Git directory unavailable")?
    } else {
        return Err("invalid Git directory".into());
    };
    let git_dir = match requested_git_dir {
        Some(path) if path.is_absolute() => path
            .canonicalize()
            .map_err(|_| "Git directory unavailable")?,
        Some(_) => return Err("Git directory must be an absolute path".into()),
        None => actual.clone(),
    };
    if git_dir != actual || !git_dir.is_dir() {
        return Err("Git directory does not match this checkout".into());
    }
    let binding_revision = if config.device.is_some() {
        let token = state::token(dir, "server-token")?;
        let registration = crate::device::signed(
            &config,
            &token,
            "repository-enroll",
            &json!({"repository_id":id}),
            5,
        )
        .await?;
        if registration["repository_id"] != id.to_string() {
            return Err("server repository binding mismatch".into());
        }
        registration["repository_revision"]
            .as_i64()
            .filter(|r| *r > 0)
            .ok_or("invalid server repository revision")?
    } else {
        1
    };
    config.repository = Some(crate::repository::RepositoryEnrollment {
        repository_id: id,
        root: config.root.clone(),
        git_dirs: vec![git_dir],
        binding_revision,
    });
    state::save(&dir.join("agent.json"), &config)?;
    if config.device.is_some() {
        crate::device::save_receipt(&config, &binding)?;
    }
    Ok(json!({"repository_enrolled":true,"repository_id":id,"local_observation_only":true}))
}

/// Fetch advisory source-backed context for explicit workspace-relative files.
pub async fn change_context(
    dir: &Path,
    files: &[String],
    operation_class: &str,
) -> Result<Value, String> {
    let config = state::load(dir)?;
    let binding = state::binding(&config)?;
    let enrollment = config
        .repository
        .as_ref()
        .ok_or("repository enrollment required; run wk-agent repository-enroll")?;
    if enrollment.root != config.root || enrollment.binding_revision < 1 {
        return Err("repository enrollment is stale".into());
    }
    if !["read_context", "edit", "write", "delete", "multi_edit"].contains(&operation_class) {
        return Err("invalid operation class".into());
    }
    let request_targets: Vec<CodeTarget> = files
        .iter()
        .map(|path| CodeTarget {
            path: path.clone(),
            start: None,
            end: None,
        })
        .collect();
    if !valid_targets(&request_targets) {
        return Err("provide 1 to 20 unique workspace-relative files".into());
    }
    for target in &request_targets {
        let path = config.root.join(&target.path);
        if !binding.eligible(&path) {
            return Err("target outside approved service scope".into());
        }
    }
    let observation = crate::repository::observe(
        &config,
        enrollment,
        &request_targets,
        operation_class,
        &json!({}),
    )
    .await?;
    let token = state::token(dir, "server-token")?;
    let product_path = product_path(&binding.product, "change-context");
    let result = if config.device.is_some() {
        crate::device::signed(
            &config,
            &token,
            "change-context",
            &serde_json::to_value(&observation.request).map_err(|_| "invalid context request")?,
            5,
        )
        .await?
    } else {
        crate::device::require_device_for_remote(&config.server, false)?;
        let response = network::client(5)?
            .post(format!(
                "{}{product_path}",
                network::server_url(&config.server)?
            ))
            .bearer_auth(token.as_str())
            .json(&observation.request)
            .send()
            .await
            .map_err(|_| "knowledge service unavailable")?;
        wk_summarize::bounded_json(response, 16 * 1024).await?
    };
    let brief_value = result.get("brief").unwrap_or(&result);
    let brief: wk_contracts::change_context::ChangeContextBrief =
        serde_json::from_value(brief_value.clone()).map_err(|_| "invalid context response")?;
    Ok(
        json!({"advisory_only":true,"local_observation_gaps":observation.gaps,"context":wk_contracts::change_context::render_change_context(&brief),"brief":brief}),
    )
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
