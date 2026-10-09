//! Direct Codex hooks for one explicitly identified visible turn.
use crate::native_evidence::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
fn id(p: &Value, key: &str) -> Result<String, String> {
    let s = p[key].as_str().ok_or("native_identity_missing")?;
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err("native_identity_invalid".into());
    }
    Ok(s.into())
}
fn hold(e: &mut Evidence, reason: &str) {
    e.hold_reason.get_or_insert_with(|| reason.into());
    e.messages_complete = false;
    e.scope = Scope::Unknown;
    if e.terminal != Terminal::Cancelled {
        e.terminal = Terminal::Held;
    }
    e.prompt.clear();
    e.response.clear();
}
pub fn apply(previous: Option<&Evidence>, p: &Value, workspace: &Path) -> Result<Evidence, String> {
    if serde_json::to_vec(p).map_err(|_| "invalid_hook")?.len() > 256 * 1024 {
        return Err("hook_exceeds_limit".into());
    }
    let session = id(p, "session_id")?;
    let turn = id(p, "turn_id")?;
    let agent = p
        .get("agent_id")
        .map(|_| id(p, "agent_id"))
        .transpose()?
        .unwrap_or_else(|| "main".into());
    if agent != "main" {
        return Err("subagent_unsupported".into());
    }
    let cwd = Path::new(p["cwd"].as_str().ok_or("workspace_missing")?);
    if cwd != workspace || cwd.canonicalize().map_err(|_| "workspace_unavailable")? != workspace {
        return Err("workspace_mismatch".into());
    }
    if previous.is_some_and(|e| {
        e.client != Client::Codex || e.session != session || e.turn != turn || e.agent != agent
    }) {
        return Err("native_identity_mismatch".into());
    }
    let mut e = previous.cloned().unwrap_or(Evidence {
        client: Client::Codex,
        client_version: "0.162.0-alpha.2".into(),
        parser_version: 1,
        session,
        agent,
        turn,
        prompt: String::new(),
        response: String::new(),
        messages_complete: false,
        scope: Scope::Verified,
        terminal: Terminal::Open,
        digest: String::new(),
        files_read: vec![],
        files_changed: vec![],
        hold_reason: None,
    });
    let event = p["hook_event_name"].as_str().ok_or("event_missing")?;
    if event == "Interrupt" {
        e.terminal = Terminal::Cancelled;
        hold(&mut e, "cancelled");
    }
    if e.hold_reason.is_none() {
        let allowed = [
            "session_id",
            "turn_id",
            "agent_id",
            "cwd",
            "hook_event_name",
            "model",
            "permission_mode",
            "transcript_path",
            "prompt",
            "last_assistant_message",
            "stop_hook_active",
        ];
        if p.as_object()
            .ok_or("invalid_hook")?
            .keys()
            .any(|k| !allowed.contains(&k.as_str()))
        {
            hold(&mut e, "unsupported_context");
        } else {
            match event {
                "UserPromptSubmit" => {
                    let text = p["prompt"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or("prompt_missing")?;
                    if previous.is_none() {
                        e.prompt = text.into();
                    } else if e.prompt != text {
                        hold(&mut e, "conflicting_prompt");
                    }
                }
                "Stop" => {
                    let text = p["last_assistant_message"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or("response_missing")?;
                    if p["stop_hook_active"] != false {
                        hold(&mut e, "continued_stop");
                    } else if e.prompt.is_empty() {
                        hold(&mut e, "orphan_stop");
                    } else if !e.response.is_empty() && e.response != text {
                        hold(&mut e, "conflicting_response");
                    } else {
                        e.response = text.into();
                        e.messages_complete = true;
                        e.terminal = Terminal::Complete;
                    }
                }
                _ => hold(&mut e, "unsupported_event"),
            }
        }
    }
    e.digest.clear();
    e.digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&e).map_err(|_| "invalid_evidence")?)
    );
    Ok(e)
}
