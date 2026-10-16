//! Read-only importer for Claude Code's local session history.
//!
//! Claude Code's storage is undocumented and changes between versions. Observed layout:
//! `<projects>/<encoded-folder>/<sessionId>.jsonl`, one JSON record per line. Records with
//! `type` `user` or `assistant` carry `uuid`, `timestamp`, `cwd`, `isSidechain` and
//! `message: {role, content}`.
//!
//! Only text the developer typed and the model's real replies are read: prompt text parts
//! (string content or `text` parts) and reply `text` parts. Thinking, tool calls, tool results,
//! meta and compact-summary records, API-error and synthetic replies, interrupt markers,
//! subagent sidechains and injected parts (slash commands, bash mode, IDE context, system
//! reminders, task notifications, and parts made only of tool-style `<tag_name>…</tag_name>`
//! blocks) are ignored; dropped injected parts are counted in `dropped_injected`. Parts made only
//! of plain-named tags such as `<task>…</task>` are prompts the developer wrote and are kept.
//! Records that look like messages but have an unknown type are counted, never guessed. The
//! workspace is the first record's `cwd` only when it is absolute. Symlinks are never followed
//! out of the projects directory.
//!
//! Session files are read in place rather than snapshotted: they are append-only JSONL, so a
//! concurrent write can at worst leave a partial last line, which is counted as a malformed
//! record and marks the session incomplete. Copying would add I/O without a consistency gain.
use crate::cursor_history::{Conversation, History, Message};
use crate::history_common::{
    absolute_dir, injected_tag_blocks, rfc3339_ms, session_id, title, Lines, Refused,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_MESSAGES: usize = 5_000;

/// `$CLAUDE_CONFIG_DIR/projects`, else `~/.claude/projects`.
pub fn default_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("projects"));
    }
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    };
    home.map(|h| PathBuf::from(h).join(".claude").join("projects"))
}

/// Read every session file directly inside each project folder of `projects_dir`.
pub fn read(projects_dir: &Path) -> Result<History, String> {
    let entries = std::fs::read_dir(projects_dir)
        .map_err(|_| "Claude Code projects directory not found or unreadable")?;
    let mut history = History::default();
    let mut files = Vec::new();
    for project in entries {
        let Ok(project) = project else {
            history.unreadable_records += 1;
            continue;
        };
        let Ok(kind) = project.file_type() else {
            history.unreadable_records += 1;
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let Ok(dir) = std::fs::read_dir(project.path()) else {
            history.unreadable_records += 1;
            continue;
        };
        for file in dir {
            let Ok(file) = file else {
                history.unreadable_records += 1;
                continue;
            };
            let path = file.path();
            if file.file_type().is_ok_and(|t| t.is_file())
                && path.extension().is_some_and(|e| e == "jsonl")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    for file in files {
        read_session(&file, &mut history);
    }
    history
        .conversations
        .sort_by_key(|c| std::cmp::Reverse(c.created_at_ms));
    Ok(history)
}

/// Read one session file into `history`. Oversized sessions count only as `skipped`;
/// unreadable records (malformed lines, unknown message types) are counted otherwise.
fn read_session(file: &Path, history: &mut History) {
    let mut lines = match Lines::open(file, MAX_FILE_BYTES) {
        Ok(lines) => lines,
        Err(refused) => {
            refused.count(history);
            return;
        }
    };
    let session = session_id(None, file);
    let mut messages: Vec<Message> = Vec::new();
    let mut workspace = None;
    let mut created_at_ms = None;
    let mut outcome = None;
    let mut saw_message_record = false;
    let mut gaps = 0;
    loop {
        let line = match lines.next_line() {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(Refused::TooLarge) => {
                history.skipped += 1;
                return;
            }
            Err(_) => {
                gaps += 1;
                break;
            }
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            gaps += 1;
            continue;
        };
        if v["isSidechain"] == true {
            continue;
        }
        if created_at_ms.is_none() {
            created_at_ms = v["timestamp"].as_str().and_then(rfc3339_ms);
        }
        let role = match v["type"].as_str() {
            Some("user") => "user",
            Some("assistant") => "assistant",
            _ => {
                if !v["message"].is_null() {
                    gaps += 1;
                }
                continue;
            }
        };
        saw_message_record = true;
        if workspace.is_none() {
            workspace = absolute_dir(v["cwd"].as_str());
        }
        let text = match role {
            "user" => prompt_text(&v, &mut history.dropped_injected),
            _ => reply_text(&v),
        };
        let Some(text) = text else { continue };
        if role == "assistant" {
            outcome = Some(text.clone());
        }
        match messages.last_mut() {
            Some(last) if last.role == role => {
                last.text.push_str("\n\n");
                last.text.push_str(&text);
            }
            _ => {
                if messages.len() == MAX_MESSAGES {
                    history.skipped += 1;
                    return;
                }
                let id = v["uuid"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map_or_else(|| format!("{session}-{}", messages.len()), str::to_owned);
                messages.push(Message {
                    id,
                    role: role.into(),
                    text,
                });
            }
        }
    }
    history.unreadable_records += gaps;
    if messages.is_empty() {
        if saw_message_record {
            history.empty_conversations += 1;
        }
        return;
    }
    let title = title(&messages);
    let complete = gaps == 0 && messages.last().is_some_and(|m| m.role == "assistant");
    history.conversations.push(Conversation {
        id: format!("claude-code:{session}"),
        title,
        workspace,
        created_at_ms,
        source: "claude-code",
        messages,
        files_changed: vec![],
        files_read: vec![],
        outcome,
        complete,
    });
}

/// A typed prompt: string content or `text` parts, each kept only if the developer wrote it;
/// `dropped` counts the injected parts. Meta, compact-summary and transcript-only records and
/// tool results are not prompts.
fn prompt_text(v: &Value, dropped: &mut usize) -> Option<String> {
    if v["isMeta"] == true
        || v["isCompactSummary"] == true
        || v["isVisibleInTranscriptOnly"] == true
        || v.get("toolUseResult").is_some()
    {
        return None;
    }
    let content = &v["message"]["content"];
    if let Some(parts) = content.as_array() {
        if parts.iter().any(|p| p["type"] == "tool_result") {
            return None;
        }
    }
    joined(text_parts(content).filter(|t| {
        let keep = t.is_empty() || typed(t);
        *dropped += usize::from(!keep);
        keep
    }))
}

/// Reply `text` parts; thinking, tool calls, API errors and synthetic replies are ignored.
fn reply_text(v: &Value) -> Option<String> {
    if v["isApiErrorMessage"] == true || v["message"]["model"] == "<synthetic>" {
        return None;
    }
    joined(text_parts(&v["message"]["content"]))
}

/// String content, or each `text` part of array content, trimmed.
fn text_parts(content: &Value) -> impl Iterator<Item = &str> {
    let (single, parts) = match content {
        Value::String(s) => (Some(s.as_str()), None),
        Value::Array(parts) => (None, Some(parts)),
        _ => (None, None),
    };
    let parts = parts
        .into_iter()
        .flatten()
        .filter(|p| p["type"] == "text")
        .filter_map(|p| p["text"].as_str());
    single.into_iter().chain(parts).map(str::trim)
}

fn joined<'a>(parts: impl Iterator<Item = &'a str>) -> Option<String> {
    let text = parts
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

/// Openings of blocks Claude Code injects into user records, dropped even when text follows the
/// block (the general all-tag-blocks rule alone would fail open on trailing or nested text).
const INJECTED_PREFIXES: [&str; 8] = [
    "<command-",
    "<local-command-",
    "<task-notification",
    "<system-reminder",
    "<ide_",
    "<user-prompt-submit-hook",
    "<user-memory-input",
    "<bash-",
];

/// Whether a trimmed prompt part is text the developer typed. Client-injected parts are dropped:
/// those made only of tool-style `<tag_name>…</tag_name>` blocks, those opening with a known
/// injected block (slash commands, bash mode, IDE context, reminders, notifications, hooks),
/// bash-mode output anywhere in the part, and interrupt markers. Parts made only of plain-named
/// tags such as `<task>…</task>` are the developer's own and kept.
fn typed(part: &str) -> bool {
    !(injected_tag_blocks(part)
        || INJECTED_PREFIXES.iter().any(|p| part.starts_with(p))
        || part.contains("<bash-stdout>")
        || part.contains("<bash-stderr>")
        || part.starts_with("[Request interrupted by user"))
}
