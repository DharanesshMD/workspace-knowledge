//! Read-only importer for Codex's local session history.
//!
//! Codex's storage is undocumented and changes between versions. Observed layout:
//! `<sessions>/YYYY/MM/DD/*.jsonl`, one `{timestamp, type, payload}` record per line. A
//! `session_meta` record carries the session `id` and `cwd`; `response_item` records whose
//! `payload.type` is `message` carry `role` and `content[]` text parts.
//!
//! Only visible text is read: typed prompts and reply text. Developer and system messages,
//! context Codex injects as user messages, reasoning, tool calls and their output, events and
//! turn context are ignored. Injected context is dropped per content part: parts starting with
//! a known marker, and parts made up entirely of `<tag ...>…</tag>` blocks. Messages with an
//! unknown role are counted, never guessed. Symlinks are never followed out of the sessions
//! directory.
//!
//! Known limits: `archived_sessions` is not read. Sub-agent threads Codex spawns are stored as
//! their own session files and are imported as ordinary sessions (how `session_meta` marks
//! them is not confirmed). A forked session copies its parent's history into a new file, so
//! the same exchanges can appear in two conversations.
use crate::claude_history::rfc3339_ms;
use crate::cursor_history::{Conversation, History, Message};
use serde_json::Value;
use std::io::BufRead;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_MESSAGES: usize = 5_000;
/// Directory levels searched for session files: `YYYY/MM/DD/`.
const MAX_DEPTH: usize = 4;

/// Context Codex sends as user messages (instructions, environment, skills, shell output,
/// notices). Matched case-insensitively at the start of any text part, as Codex itself does.
const INJECTED: [&str; 8] = [
    "<environment_context>",
    "<user_instructions>",
    "<permissions instructions>",
    "# AGENTS.md instructions for ",
    "<skill>",
    "<user_shell_command>",
    "<turn_aborted>",
    "<subagent_notification>",
];

/// `$CODEX_HOME/sessions`, else `~/.codex/sessions`.
pub fn default_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("sessions"));
    }
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    };
    home.map(|h| PathBuf::from(h).join(".codex/sessions"))
}

/// Read every session file up to `YYYY/MM/DD/` below `sessions_dir`.
pub fn read(sessions_dir: &Path) -> Result<History, String> {
    let entries = std::fs::read_dir(sessions_dir)
        .map_err(|_| "Codex sessions directory not found or unreadable")?;
    let mut files = Vec::new();
    collect(entries, MAX_DEPTH, &mut files);
    files.sort();
    let mut history = History::default();
    for file in files {
        read_session(&file, &mut history);
    }
    history
        .conversations
        .sort_by_key(|c| std::cmp::Reverse(c.created_at_ms));
    Ok(history)
}

/// `*.jsonl` files in `entries` and, while `depth` allows, in real (non-symlink) subfolders.
fn collect(entries: std::fs::ReadDir, depth: usize, files: &mut Vec<PathBuf>) {
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() && depth > 1 {
            if let Ok(inner) = std::fs::read_dir(&path) {
                collect(inner, depth - 1, files);
            }
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
            files.push(path);
        }
    }
}

/// Read one session file into `history`. Oversized sessions count only as `skipped`;
/// unreadable records (malformed lines, unknown roles) are counted otherwise.
fn read_session(file: &Path, history: &mut History) {
    if std::fs::symlink_metadata(file).map_or(u64::MAX, |m| m.len()) > MAX_FILE_BYTES {
        history.skipped += 1;
        return;
    }
    let Ok(handle) = std::fs::File::open(file) else {
        history.unreadable_records += 1;
        return;
    };
    let mut session = file
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut messages: Vec<Message> = Vec::new();
    let mut counts = (0, 0);
    let mut workspace = None;
    let mut created_at_ms = None;
    let mut outcome = None;
    let mut saw_meta = false;
    let mut saw_message_record = false;
    let mut gaps = 0;
    for line in std::io::BufReader::new(handle).split(b'\n') {
        let Ok(line) = line else {
            gaps += 1;
            break;
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
            gaps += 1;
            continue;
        };
        if created_at_ms.is_none() {
            created_at_ms = v["timestamp"].as_str().and_then(rfc3339_ms);
        }
        let payload = &v["payload"];
        match v["type"].as_str() {
            Some("session_meta") if !saw_meta => {
                saw_meta = true;
                if let Some(id) = payload["id"].as_str().filter(|s| !s.is_empty()) {
                    session = id.to_owned();
                }
                workspace = payload["cwd"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from);
                continue;
            }
            Some("response_item") if payload["type"] == "message" => {}
            _ => continue,
        }
        let role = match payload["role"].as_str() {
            Some("user") => "user",
            Some("assistant") => "assistant",
            Some("developer" | "system") => continue,
            _ => {
                gaps += 1;
                continue;
            }
        };
        let Some(parts) = payload["content"].as_array() else {
            gaps += 1;
            continue;
        };
        saw_message_record = true;
        let texts: Vec<&str> = parts
            .iter()
            .filter(|p| p["type"] == "input_text" || p["type"] == "output_text")
            .filter_map(|p| p["text"].as_str())
            .filter(|t| role == "assistant" || !injected(t))
            .collect();
        let text = texts.join("\n").trim().to_string();
        if text.is_empty() {
            continue;
        }
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
                let id = if role == "user" {
                    counts.0 += 1;
                    format!("u{}", counts.0)
                } else {
                    counts.1 += 1;
                    format!("a{}", counts.1)
                };
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
    let title = messages
        .iter()
        .find(|m| m.role == "user")
        .and_then(|m| m.text.lines().find(|l| !l.trim().is_empty()))
        .map(|l| l.trim().chars().take(80).collect())
        .unwrap_or_else(|| "Untitled chat".into());
    let complete = gaps == 0 && messages.last().is_some_and(|m| m.role == "assistant");
    history.conversations.push(Conversation {
        id: format!("codex:{session}"),
        title,
        workspace,
        created_at_ms,
        source: "codex",
        messages,
        files_changed: vec![],
        files_read: vec![],
        outcome,
        complete,
    });
}

/// Whether a user text part is context Codex injected rather than a typed prompt: it starts
/// with a known marker or consists only of tag-wrapped blocks.
fn injected(text: &str) -> bool {
    let start = text.trim_start();
    INJECTED.iter().any(|marker| {
        start
            .get(..marker.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(marker))
    }) || tag_wrapped(text)
}

/// Whether `text`, trimmed, is one or more `<tag ...>…</tag>` blocks separated only by
/// whitespace. Tag names match `[A-Za-z][A-Za-z0-9_-]*`; any other text keeps the part.
fn tag_wrapped(text: &str) -> bool {
    let mut rest = text.trim();
    if rest.is_empty() {
        return false;
    }
    while !rest.is_empty() {
        let Some(open) = rest.strip_prefix('<') else {
            return false;
        };
        let len = open
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(open.len());
        let (name, after) = open.split_at(len);
        if !name.starts_with(|c: char| c.is_ascii_alphabetic())
            || !after.starts_with(|c: char| c == '>' || c.is_whitespace())
        {
            return false;
        }
        let Some(gt) = after.find('>') else {
            return false;
        };
        let body = &after[gt + 1..];
        let close = format!("</{name}>");
        let Some(end) = body.find(&close) else {
            return false;
        };
        rest = body[end + close.len()..].trim_start();
    }
    true
}
