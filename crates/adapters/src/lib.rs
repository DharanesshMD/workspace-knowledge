//! Version-observed hook normalization. Unknown coverage never becomes complete.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
#[derive(Serialize, Deserialize)]
pub struct Observation {
    pub session: String,
    pub agent: String,
    pub turn: Option<String>,
    pub kind: String,
    pub text: String,
    pub paths: Vec<PathBuf>,
    pub complete_coverage: bool,
}
fn string(v: &Value, name: &str) -> Result<String, String> {
    let s = v[name].as_str().ok_or("missing client field")?;
    if s.is_empty() || s.len() > 256 {
        return Err("invalid client identity".into());
    }
    Ok(s.into())
}
pub fn normalize(client: &str, v: &Value) -> Result<Option<Observation>, String> {
    if serde_json::to_vec(v).map_err(|_| "invalid hook")?.len() > 256 * 1024 {
        return Err("hook exceeds limit".into());
    }
    let event = v["hook_event_name"].as_str().ok_or("missing event type")?;
    if matches!(event, "afterAgentThought" | "reasoning" | "thinking") {
        return Ok(None);
    }
    let (session, turn, kind, field) = match (client, event) {
        ("cursor", "beforeSubmitPrompt") => (
            string(v, "conversation_id")?,
            v["generation_id"].as_str(),
            "user_prompt",
            Some("prompt"),
        ),
        ("cursor", "afterAgentResponse") => (
            string(v, "conversation_id")?,
            v["generation_id"].as_str(),
            "assistant_response",
            Some("text"),
        ),
        ("cursor", "stop") => (
            string(v, "conversation_id")?,
            v["generation_id"].as_str(),
            if v["status"] == "completed" {
                "turn_end"
            } else {
                "capture_gap"
            },
            None,
        ),
        ("claude-code", "UserPromptSubmit") => (
            string(v, "session_id")?,
            v["turn_id"].as_str(),
            "user_prompt",
            Some("prompt"),
        ),
        ("claude-code", "Stop") => (
            string(v, "session_id")?,
            v["turn_id"].as_str(),
            "assistant_response",
            Some("last_assistant_message"),
        ),
        ("gemini", "BeforeAgent") => (
            string(v, "session_id")?,
            v["turn_id"].as_str(),
            "user_prompt",
            Some("prompt"),
        ),
        ("gemini", "AfterAgent") => (
            string(v, "session_id")?,
            v["turn_id"].as_str(),
            "assistant_response",
            Some("prompt_response"),
        ),
        ("cursor", "beforeReadFile" | "afterFileEdit") => (
            string(v, "conversation_id")?,
            v["generation_id"].as_str(),
            "tool_result",
            None,
        ),
        ("claude-code", "PreToolUse" | "PostToolUse") | ("gemini", "BeforeTool" | "AfterTool") => (
            string(v, "session_id")?,
            v["turn_id"].as_str(),
            "tool_result",
            None,
        ),
        ("claude-code", "PreCompact" | "StopFailure") | ("gemini", "PreCompress") => {
            (string(v, "session_id")?, None, "capture_gap", None)
        }
        ("cursor", "preCompact") => (string(v, "conversation_id")?, None, "capture_gap", None),
        ("codex" | "antigravity", _) => {
            return Err("native capture not certified; use managed chat or explicit import".into())
        }
        _ => return Ok(None),
    };
    let mut paths = Vec::new();
    if let Some(cwd) = v["cwd"].as_str() {
        paths.push(PathBuf::from(cwd));
    }
    if let Some(roots) = v["workspace_roots"].as_array() {
        for p in roots {
            paths.push(PathBuf::from(p.as_str().ok_or("invalid workspace roots")?));
        }
    }
    for key in ["file_path", "path"] {
        if let Some(path) = v[key].as_str() {
            paths.push(path.into());
        }
        if let Some(path) = v["tool_input"][key].as_str() {
            paths.push(path.into());
        }
    }
    if let Some(attachments) = v["attachments"].as_array() {
        for a in attachments {
            if let Some(path) = a["file_path"].as_str() {
                paths.push(path.into());
            }
        }
    }
    if paths.is_empty() {
        return Err("hook has no attributable workspace".into());
    }
    Ok(Some(Observation {
        session: format!("{client}:{session}"),
        agent: v["agent_id"].as_str().unwrap_or("main").into(),
        turn: turn.map(str::to_owned),
        kind: kind.into(),
        text: match field {
            Some(f) => v[f].as_str().ok_or("missing visible message")?.into(),
            None => String::new(),
        },
        paths,
        complete_coverage: false,
    }))
}
pub fn capabilities() -> Value {
    json!({"cursor":{"status":"partial","prompt":true,"response":true,"live_certified":false},"claude-code":{"status":"partial","prompt":true,"response":true,"live_certified":false},"gemini":{"status":"partial","prompt":true,"response":true,"live_certified":false},"codex":{"status":"unsupported-native","alternative":"managed-chat"},"antigravity":{"status":"unsupported-native","alternative":"managed-chat"},"claude-desktop":{"status":"enterprise-bound-chat-or-managed-chat","mcp_is_capture":false}})
}
/// Generate a merge fragment only; caller must explicitly choose a config file.
pub fn config(client: &str, executable: &Path, state: &Path) -> Result<Value, String> {
    let quote = |p: &Path| -> Result<String, String> {
        let s = p.to_str().ok_or("non-UTF8 config path")?;
        if s.contains(['\'', '"', '\n', '\r', '`', '$', '%']) {
            return Err("unsafe hook command path".into());
        }
        Ok(format!("\"{s}\""))
    };
    let command = format!(
        "{} --state-dir {} hook --client {client}",
        quote(executable)?,
        quote(state)?
    );
    let events: &[&str] = match client {
        "cursor" => &[
            "beforeSubmitPrompt",
            "afterAgentResponse",
            "stop",
            "beforeReadFile",
            "afterFileEdit",
            "postToolUse",
            "preCompact",
        ],
        "claude-code" => &[
            "UserPromptSubmit",
            "Stop",
            "PreToolUse",
            "PostToolUse",
            "PreCompact",
            "StopFailure",
        ],
        "gemini" => &[
            "BeforeAgent",
            "AfterAgent",
            "BeforeTool",
            "AfterTool",
            "PreCompress",
        ],
        _ => return Err("client has no supported hook configuration".into()),
    };
    let mut hooks = serde_json::Map::new();
    for event in events {
        let command = format!("{command} --event {event}");
        hooks.insert(
            (*event).into(),
            if client == "cursor" {
                json!([{"command":command,"timeout":2}])
            } else {
                json!([{"hooks":[{"type":"command","command":command,"timeout":2}]}])
            },
        );
    }
    Ok(if client == "cursor" {
        json!({"version":1,"hooks":hooks})
    } else {
        json!({"hooks":hooks})
    })
}

pub mod cursor_history;
pub mod install;
pub mod team_context;

/// Observational hooks never veto development or request another model turn.
pub fn success(client: &str, event: &str) -> Value {
    match (client, event) {
        ("cursor", "beforeReadFile") => json!({"permission":"allow"}),
        ("cursor", "beforeSubmitPrompt") => json!({"continue":true}),
        _ => json!({}),
    }
}

pub mod cursor_session;
