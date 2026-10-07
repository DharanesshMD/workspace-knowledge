//! Cursor 3.23.23 visible conversation parser. Never discovers or opens transcripts.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use wk_scope::enrollment::Binding;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScopeState {
    Verified,
    Unknown,
    Violated,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageState {
    Partial,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CursorTurn {
    pub index: u64,
    pub prompt: String,
    pub response: String,
    pub files_read: Vec<String>,
    pub files_changed: Vec<String>,
    pub messages: MessageState,
    pub scope: ScopeState,
    pub digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CursorSnapshot {
    pub consumed: usize,
    pub prefix_digest: String,
    pub turns: Vec<CursorTurn>,
    pub held_reason: Option<String>,
}
fn visible_user(text: &str) -> Result<String, ()> {
    // Only known injected wrappers are discarded. Unknown wrappers/attachments are held.
    let mut rest = text.to_string();
    for tag in [
        "timestamp",
        "system_reminder",
        "environment_details",
        "user_info",
        "git_status",
        "rules",
    ] {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        while let Some(a) = rest.find(&open) {
            let z = rest[a + open.len()..].find(&close).ok_or(())? + a + open.len() + close.len();
            rest.replace_range(a..z, "");
        }
    }
    if rest.contains("<user_query>") {
        let a = rest.find("<user_query>").ok_or(())?;
        let z = rest.find("</user_query>").ok_or(())?;
        if z < a || !rest[..a].trim().is_empty() || !rest[z + 13..].trim().is_empty() {
            return Err(());
        }
        rest = rest[a + 12..z].to_string();
    }
    if rest.contains('<') || rest.contains("@file") {
        return Err(());
    }
    Ok(rest.trim().to_string())
}
fn new_turn(index: u64, prompt: String) -> CursorTurn {
    CursorTurn {
        index,
        prompt,
        response: String::new(),
        files_read: vec![],
        files_changed: vec![],
        messages: MessageState::Partial,
        scope: ScopeState::Verified,
        digest: String::new(),
    }
}
pub fn parse_transcript(
    bytes: &[u8],
    binding: &Binding,
    service: &str,
) -> Result<CursorSnapshot, String> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("source_too_large".into());
    }
    if !binding.services.iter().any(|s| s == service)
        || !binding.eligible(&binding.root.join(service))
    {
        return Err("scope_unavailable".into());
    }
    let consumed = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let mut snapshot = CursorSnapshot {
        consumed,
        prefix_digest: format!("{:x}", Sha256::digest(&bytes[..consumed])),
        turns: vec![],
        held_reason: None,
    };
    let mut scope = ScopeState::Verified;
    let mut current: Option<CursorTurn> = None;
    let hold = |s: &mut CursorSnapshot, reason: &str| {
        if s.held_reason.is_none() {
            s.held_reason = Some(reason.into());
        }
    };
    for (index, line) in bytes[..consumed].split(|b| *b == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        if line.len() > 256 * 1024 {
            hold(&mut snapshot, "record_too_large");
            break;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            hold(&mut snapshot, "invalid_record");
            break;
        };
        if v["type"] == "turn_ended" {
            if let Some(mut turn) = current.take() {
                if v["status"] == "success"
                    && !turn.prompt.is_empty()
                    && !turn.response.trim().is_empty()
                {
                    turn.messages = MessageState::Complete;
                } else if !matches!(
                    v["status"].as_str(),
                    Some("error" | "cancelled" | "success")
                ) {
                    hold(&mut snapshot, "unknown_terminal");
                }
                snapshot.turns.push(turn);
            } else {
                hold(&mut snapshot, "orphan_terminal");
            }
            continue;
        }
        let Some(role) = v["role"]
            .as_str()
            .filter(|r| matches!(*r, "user" | "assistant"))
        else {
            hold(&mut snapshot, "unknown_record");
            scope = scope.max(ScopeState::Unknown);
            continue;
        };
        let Some(content) = v["message"]["content"].as_array() else {
            hold(&mut snapshot, "unknown_content");
            continue;
        };
        let mut text = String::new();
        for block in content {
            match block["type"].as_str() {
                Some("text") => match block["text"].as_str() {
                    Some(t) => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                    None => hold(&mut snapshot, "unknown_content"),
                },
                Some("thinking" | "reasoning") if role == "assistant" => {}
                Some("tool_use") if role == "assistant" => {
                    let name = block["name"].as_str().unwrap_or("");
                    if !matches!(
                        name,
                        "Read" | "ReadFile" | "StrReplace" | "Write" | "Edit" | "Delete"
                    ) {
                        scope = scope.max(ScopeState::Unknown);
                        continue;
                    }
                    let Some(raw) = block["input"]["path"]
                        .as_str()
                        .or_else(|| block["input"]["file_path"].as_str())
                    else {
                        scope = scope.max(ScopeState::Unknown);
                        continue;
                    };
                    let path = std::path::Path::new(raw);
                    let path = if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        binding.root.join(path)
                    };
                    if !path.starts_with(binding.root.join(service)) || !binding.eligible(&path) {
                        scope = ScopeState::Violated;
                        continue;
                    }
                    if let Some(turn) = current.as_mut() {
                        let relative = path
                            .strip_prefix(&binding.root)
                            .map_err(|_| "invalid_scope")?
                            .to_string_lossy()
                            .into_owned();
                        if matches!(name, "Read" | "ReadFile") {
                            turn.files_read.push(relative);
                        } else {
                            turn.files_changed.push(relative);
                        }
                    } else {
                        hold(&mut snapshot, "orphan_tool");
                    }
                }
                _ => {
                    scope = scope.max(ScopeState::Unknown);
                    hold(&mut snapshot, "unknown_content");
                }
            }
        }
        if role == "user" {
            if let Some(turn) = current.take() {
                snapshot.turns.push(turn);
            }
            match visible_user(&text) {
                Ok(prompt) => current = Some(new_turn(index as u64, prompt)),
                Err(()) => {
                    hold(&mut snapshot, "unknown_user_context");
                    scope = scope.max(ScopeState::Unknown);
                }
            }
        } else if let Some(turn) = current.as_mut() {
            if !turn.response.is_empty() && !text.is_empty() {
                turn.response.push('\n');
            }
            turn.response.push_str(&text);
        } else {
            hold(&mut snapshot, "orphan_response");
        }
        if snapshot.turns.len() >= 100 {
            hold(&mut snapshot, "turn_limit");
            break;
        }
    }
    if let Some(turn) = current {
        snapshot.turns.push(turn);
    }
    if snapshot.turns.len() > 100 {
        hold(&mut snapshot, "turn_limit");
    }
    for turn in &mut snapshot.turns {
        turn.scope = scope;
        turn.files_read.sort();
        turn.files_read.dedup();
        turn.files_changed.sort();
        turn.files_changed.dedup();
        turn.digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&turn).map_err(|_| "invalid_turn")?)
        );
    }
    if serde_json::to_vec(&snapshot)
        .map_err(|_| "invalid_snapshot")?
        .len()
        > 768 * 1024
    {
        hold(&mut snapshot, "retained_limit");
    }
    if snapshot.held_reason.is_some() {
        snapshot.turns.clear();
    }
    Ok(snapshot)
}
