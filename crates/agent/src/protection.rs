#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::LocalObservation;
    use std::collections::{BTreeMap, BTreeSet};
    use wk_contracts::change_context::{ChangeContextRequest, CodeTarget};

    fn observation(digest: &str) -> LocalObservation {
        LocalObservation {
            request: ChangeContextRequest {
                schema_version: 1,
                repository_id: uuid::Uuid::from_u128(1),
                base_revision: "a".repeat(40),
                targets: vec![CodeTarget {
                    path: "backend/a.rs".into(),
                    start: None,
                    end: None,
                }],
                operation_class: "edit".into(),
                operation_handle: None,
                budget_bytes: 16384,
            },
            target_digests: BTreeMap::from([("backend/a.rs".into(), digest.into())]),
            operation_digest: "operation".into(),
            gaps: vec![],
            candidate_references: BTreeSet::new(),
        }
    }

    #[test]
    fn pending_retry_requires_same_operation_snapshot_and_live_expiry() {
        let pending = PendingOperation {
            observation: observation("same"),
            brief_id: uuid::Uuid::new_v4(),
            operation_handle: uuid::Uuid::new_v4(),
            expires_at_ms: 2000,
        };
        assert!(pending_matches(&pending, &observation("same"), 1999));
        assert!(!pending_matches(&pending, &observation("changed"), 1999));
        assert!(!pending_matches(&pending, &observation("same"), 2000));
    }

    #[tokio::test]
    async fn missing_enrollment_returns_explicit_block_instead_of_capture_success() {
        let dir = tempfile::tempdir().unwrap();
        let output=before_edit(dir.path(),"claude-code","2.1.295",serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{"file_path":"/repo/a.rs","old_string":"x","new_string":"y"}})).await;
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(output["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("edit is blocked"));
    }

    #[test]
    fn status_clears_an_expired_local_operation_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let pending_path = dir.path().join(PENDING);
        state::save(
            &pending_path,
            &PendingOperation {
                observation: observation("expired-digest"),
                brief_id: uuid::Uuid::new_v4(),
                operation_handle: uuid::Uuid::new_v4(),
                expires_at_ms: 0,
            },
        )
        .unwrap();

        let report = status(dir.path());

        assert_eq!(report["status"], "advisory_only");
        assert!(!pending_path.exists());
    }
}
use crate::{network, repository::LocalObservation, state};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use wk_adapters::change_guard::{self, GuardDecision};
use wk_contracts::change_context::{ChangeContextBrief, CodeTarget};

const PENDING: &str = "pending-code-operation.json";
const CONFIG: &str = "protection.json";

#[derive(clap::Subcommand)]
pub enum Action {
    /// Install a separate fail-closed Claude Code pre-edit hook in an explicit settings file.
    Enroll {
        #[arg(long)]
        settings_file: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    /// Show installed client/version coverage and its known bypasses.
    Status,
    /// Remove only the hook owned by this protection enrollment.
    Disable {
        #[arg(long)]
        settings_file: PathBuf,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOperation {
    pub observation: LocalObservation,
    pub brief_id: uuid::Uuid,
    pub operation_handle: uuid::Uuid,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Enrollment {
    client: String,
    version: String,
    config_file: PathBuf,
    fragment: Value,
}

fn pending_matches(pending: &PendingOperation, current: &LocalObservation, now_ms: i64) -> bool {
    pending.expires_at_ms > now_ms
        && crate::repository::same_snapshot(&pending.observation, current)
}

fn now_ms() -> Result<i64, String> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "clock unavailable")?
        .as_millis()
        .try_into()
        .map_err(|_| "clock unavailable")?)
}

fn denied(reason: &str, brief_url: Option<String>) -> Value {
    change_guard::guard_output(
        "claude-code",
        GuardDecision::Stop {
            reason: reason.into(),
            brief_url: brief_url.unwrap_or_else(|| {
                "Open your team's Workspace Knowledge portal and request a fresh review.".into()
            }),
        },
    )
}

fn brief_url(config: &crate::state::Config, id: uuid::Uuid) -> Result<String, String> {
    Ok(format!(
        "{}/?brief={id}",
        network::server_url(&config.server)?
    ))
}

fn targets_in_scope(
    config: &crate::state::Config,
    binding: &wk_scope::enrollment::Binding,
    edit: &change_guard::GuardEdit,
    payload: &Value,
) -> Result<Option<Vec<CodeTarget>>, String> {
    let root = config
        .root
        .canonicalize()
        .map_err(|_| "workspace unavailable")?;
    let cwd = payload["cwd"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| root.clone());
    if !cwd.is_absolute() {
        return Err("invalid hook working directory".into());
    }
    let cwd = cwd
        .canonicalize()
        .map_err(|_| "invalid hook working directory")?;
    let mut targets = Vec::new();
    for raw in &edit.targets {
        if raw
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err("unsafe target path".into());
        }
        let absolute = if raw.is_absolute() {
            raw.clone()
        } else {
            cwd.join(raw)
        };
        let Ok(relative) = absolute.strip_prefix(&root) else {
            return Ok(None);
        };
        let relative = relative
            .to_str()
            .ok_or("invalid target path")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        let target = CodeTarget {
            path: relative,
            start: None,
            end: None,
        };
        if !target.validate() {
            return Err("invalid target path".into());
        }
        let absolute_target = root.join(&target.path);
        if !absolute_target.starts_with(root.join(&config.service)) {
            return Ok(None);
        }
        if !binding.eligible(&absolute_target) {
            return Err("target failed workspace scope checks".into());
        }
        targets.push(target);
    }
    if !wk_contracts::change_context::valid_targets(&targets) {
        return Err("invalid target set".into());
    }
    Ok(Some(targets))
}

async fn acquire_lock(dir: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(dir.join("protection.lock"))
        .map_err(|_| "protection lock unavailable")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(_) => return Err("protection lock unavailable".into()),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("protection lock unavailable".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Hook entry point: every unhandled failure is a blocking decision, never capture's allow fallback.
pub async fn before_edit(dir: &Path, client: &str, version: &str, payload: Value) -> Value {
    match before_edit_inner(dir, client, version, payload).await {
        Ok(value) => value,
        Err(_) => denied(
            "Workspace Knowledge could not verify the operation; the edit is blocked.",
            None,
        ),
    }
}

async fn before_edit_inner(
    dir: &Path,
    client: &str,
    version: &str,
    payload: Value,
) -> Result<Value, String> {
    let enrollment: Enrollment = serde_json::from_str(&state::read(&dir.join(CONFIG), 16384)?)
        .map_err(|_| "protection enrollment unavailable")?;
    if client != enrollment.client
        || version != enrollment.version
        || !change_guard::supports_fail_closed(version)
    {
        return Err("protection version changed".into());
    }
    let edit = change_guard::parse_edit(client, version, &payload)
        .map_err(|_| "unsupported protected tool input")?;
    let config = state::load(dir)?;
    let binding = state::binding(&config)?;
    let targets = match targets_in_scope(&config, &binding, &edit, &payload)? {
        Some(v) => v,
        None => return Ok(json!({})),
    };
    let repo = config
        .repository
        .as_ref()
        .ok_or("repository enrollment required")?;
    if config.device.is_none() {
        return Err("hosted device binding required".into());
    }
    let _lock = acquire_lock(dir).await?;
    let observation = crate::repository::observe(
        &config,
        repo,
        &targets,
        &edit.operation_class,
        &edit.tool_input,
    )
    .await?;
    let pending_path = dir.join(PENDING);
    if pending_path.exists() {
        let pending: PendingOperation = serde_json::from_str(&state::read(&pending_path, 65536)?)
            .map_err(|_| "pending operation unavailable")?;
        if pending_matches(&pending, &observation, now_ms()?) {
            let token = state::token(dir, "server-token")?;
            let consumed = crate::device::signed(
                &config,
                &token,
                "preflight-consume",
                &json!({"brief_id":pending.brief_id,"operation_handle":pending.operation_handle}),
                5,
            )
            .await;
            if consumed.is_ok() {
                let current = crate::repository::observe(
                    &config,
                    repo,
                    &targets,
                    &edit.operation_class,
                    &edit.tool_input,
                )
                .await?;
                if crate::repository::same_snapshot(&observation, &current) {
                    std::fs::remove_file(&pending_path)
                        .map_err(|_| "pending operation cleanup failed")?;
                    return Ok(json!({}));
                }
                return create_preflight(dir, &config, &observation, &edit.operation_class).await;
            }
            return Ok(denied(
                "Review or refresh this operation before it can proceed.",
                Some(brief_url(&config, pending.brief_id)?),
            ));
        }
    }
    create_preflight(dir, &config, &observation, &edit.operation_class).await
}

async fn create_preflight(
    dir: &Path,
    config: &crate::state::Config,
    observation: &LocalObservation,
    operation_class: &str,
) -> Result<Value, String> {
    let mut request = observation.request.clone();
    request.operation_class = operation_class.into();
    let handle = uuid::Uuid::new_v4();
    request.operation_handle = Some(handle);
    let token = state::token(dir, "server-token")?;
    let result = crate::device::signed(
        config,
        &token,
        "change-context",
        &serde_json::to_value(request).map_err(|_| "invalid context request")?,
        5,
    )
    .await?;
    let brief: ChangeContextBrief =
        serde_json::from_value(result.get("brief").cloned().unwrap_or(result))
            .map_err(|_| "invalid preflight response")?;
    let brief_id = brief.brief_id.ok_or("preflight identifier unavailable")?;
    let expires_at_ms = brief.expires_at_ms.ok_or("preflight expiry unavailable")?;
    let pending = PendingOperation {
        observation: observation.clone(),
        brief_id,
        operation_handle: handle,
        expires_at_ms,
    };
    state::save(&dir.join(PENDING), &pending)?;
    Ok(change_guard::guard_output(
        "claude-code",
        GuardDecision::Stop {
            reason: "Context is incomplete or needs your review".into(),
            brief_url: brief_url(config, brief_id)?,
        },
    ))
}

/// Read a small JSON event without allowing prompt-sized input into memory.
pub fn read_hook_input() -> Result<Value, String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "invalid hook input")?;
    if bytes.len() > 256 * 1024 {
        return Err("hook input too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid hook input".into())
}

pub async fn run_hook(dir: &Path, client: &str, version: &str) -> Value {
    match read_hook_input() {
        Ok(payload) => before_edit(dir, client, version, payload).await,
        Err(_) => denied(
            "Workspace Knowledge could not read this operation; the edit is blocked.",
            None,
        ),
    }
}

pub fn enroll(dir: &Path, file: &Path, yes: bool) -> Result<Value, String> {
    if !yes {
        return Err(
            "review the fail-closed Claude Code 2.1.295+ hook scope, then pass --yes".into(),
        );
    }
    if !file.is_absolute() {
        return Err("explicit absolute settings file required".into());
    }
    if dir.join(CONFIG).exists() {
        return Err("protection is already configured; inspect status or disable it first".into());
    }
    let config = state::load(dir)?;
    state::binding(&config)?;
    if config.device.is_none() || config.repository.is_none() {
        return Err("hosted device and repository enrollment required".into());
    }
    let version = installed_claude_version()?;
    let executable = std::env::current_exe().map_err(|_| "wk-agent executable unavailable")?;
    let fragment = change_guard::hook_config(
        executable.to_str().ok_or("invalid executable path")?,
        dir.to_str().ok_or("invalid state path")?,
        &version,
    )?;
    let existing: Value = if file.exists() {
        serde_json::from_str(&state::read(file, 1024 * 1024)?)
            .map_err(|_| "Claude settings must be strict JSON; left unchanged")?
    } else {
        json!({})
    };
    let merged = wk_adapters::install::merge(&existing, &fragment)?;
    state::save(file, &merged)?;
    let enrollment = Enrollment {
        client: "claude-code".into(),
        version: version.clone(),
        config_file: file.to_path_buf(),
        fragment,
    };
    state::save(&dir.join(CONFIG), &enrollment)?;
    Ok(
        json!({"installed":true,"client":"claude-code","version":version,"status":"configured_unverified","certified":false,"covered_tools":["Edit","Write","MultiEdit"],"uncovered":["Bash and shell commands","NotebookEdit","manual or external file writes","other AI clients"]}),
    )
}

pub fn disable(dir: &Path, file: &Path) -> Result<Value, String> {
    let enrollment: Enrollment = serde_json::from_str(&state::read(&dir.join(CONFIG), 16384)?)
        .map_err(|_| "protection enrollment unavailable")?;
    if !file.is_absolute() || file != enrollment.config_file {
        return Err("settings file does not match the owned protection hook".into());
    }
    let existing: Value = serde_json::from_str(&state::read(file, 1024 * 1024)?)
        .map_err(|_| "Claude settings must be strict JSON; left unchanged")?;
    state::save(
        file,
        &wk_adapters::install::remove(&existing, &enrollment.fragment)?,
    )?;
    std::fs::remove_file(dir.join(CONFIG)).map_err(|_| "protection enrollment cleanup failed")?;
    let pending = dir.join(PENDING);
    if pending.exists() {
        std::fs::remove_file(pending).map_err(|_| "pending operation cleanup failed")?;
    }
    Ok(json!({"disabled":true}))
}

pub fn status(dir: &Path) -> Value {
    expire_pending(dir);
    match state::read(&dir.join(CONFIG), 16384).and_then(|s| {
        serde_json::from_str::<Enrollment>(&s).map_err(|_| "invalid protection state".into())
    }) {
        Ok(e) => {
            json!({"installed":true,"client":e.client,"version":e.version,"status":"configured_unverified","certified":false,"covered_tools":["Edit","Write","MultiEdit"],"uncovered":["Bash and shell commands","NotebookEdit","manual or external file writes","other AI clients"]})
        }
        Err(_) => json!({"installed":false,"status":"advisory_only","certified":false}),
    }
}

fn expire_pending(dir: &Path) {
    let path = dir.join(PENDING);
    let Ok(raw) = state::read(&path, 65536) else {
        return;
    };
    let Ok(pending) = serde_json::from_str::<PendingOperation>(&raw) else {
        return;
    };
    if now_ms().map_or(true, |now| pending.expires_at_ms <= now) {
        let _ = std::fs::remove_file(path);
    }
}

fn installed_claude_version() -> Result<String, String> {
    let result = std::process::Command::new("claude")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Claude Code unavailable")?;
    if !result.status.success() || result.stdout.len() > 1024 {
        return Err("Claude Code version unavailable".into());
    }
    let version = String::from_utf8(result.stdout)
        .map_err(|_| "Claude Code version unavailable")?
        .split_whitespace()
        .next()
        .ok_or("Claude Code version unavailable")?
        .to_string();
    if !change_guard::supports_fail_closed(&version) {
        return Err("Claude Code 2.1.295 or later is required for fail-closed hooks".into());
    }
    Ok(version)
}
