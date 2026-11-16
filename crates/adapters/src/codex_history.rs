//! Read-only importer for Codex's local session history.
//!
//! Codex's storage is undocumented and changes between versions. Observed layout:
//! `<sessions>/YYYY/MM/DD/*.jsonl`, one `{timestamp, type, payload}` record per line. A
//! `session_meta` record carries the session `id` and `cwd`; `response_item` records whose
//! `payload.type` is `message` carry `role` and `content[]` text parts.
//!
//! Only visible text is read: typed prompts and reply text. Developer and system messages,
//! context Codex injects as user messages, reasoning, tool calls and their output, events and
//! turn context are ignored. Injected context is dropped per content part, and counted in
//! `dropped_injected`: parts starting with a known marker, and parts made up entirely of
//! `<tag ...>…</tag>` blocks at least one of which has a tool-style name containing `_` or `-`
//! (parts made only of plain-named tags such as `<task>` are typed prompts and kept). A new
//! injected wrapper with a plain name (like `<skill>`) is therefore only dropped once it is
//! added to the markers below; the general tag rule cannot see it. Messages
//! with an unknown role and content parts of an unknown type (anything but `input_text`,
//! `output_text` and text-less `input_image`) are counted, never guessed. An `input_image` is
//! never read; the prompt it came with gains one `[developer attached N image(s)]` line after
//! its text. A user message made only of injected context leaves no prompt, so the replies
//! around it read as one assistant turn.
//! The workspace is `session_meta.cwd` only when it is absolute and has no `.` or `..`
//! component. A session id found in two files is imported once, from the most recently
//! modified file; the dropped copy counts as an unreadable record. Session files are read through a
//! bounded, symlink-refusing open; symlinks are never followed out of the sessions directory.
//!
//! Compaction: in the observed data compaction writes top-level `compacted` records, which are
//! skipped like other non-message records, and no compaction-bridge user messages were found;
//! if Codex later replays a summary as a user message it would need a marker here.
//!
//! Known limits: `archived_sessions` is not read. Sub-agent threads Codex spawns are stored as
//! their own session files and are imported as ordinary sessions (how `session_meta` marks
//! them is not confirmed). A forked session copies its parent's history into a new file, so
//! the same exchanges can appear in two conversations.
use crate::cursor_history::{Conversation, History, Message};
use crate::history_common::{
    absolute_dir, image_marker, injected_tag_blocks, rfc3339_ms, session_id, title, Lines, Newest,
    Refused,
};
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_MESSAGES: usize = 5_000;
/// Directory levels searched for session files: `YYYY/MM/DD/`.
const MAX_DEPTH: usize = 4;

/// Wrapper tags of context Codex sends as user messages (environment, instructions, skills,
/// shell output, notices). A part opening with one, with or without attributes, is injected.
const INJECTED_TAGS: [&str; 6] = [
    "environment_context",
    "user_instructions",
    "skill",
    "user_shell_command",
    "turn_aborted",
    "subagent_notification",
];

/// Other openings of injected context.
const INJECTED_PREFIXES: [&str; 2] = [
    "<permissions instructions>",
    "# AGENTS.md instructions for ",
];

/// `$CODEX_HOME/sessions`, else `~/.codex/sessions`.
pub fn default_dir() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    };
    sessions_dir(std::env::var_os("CODEX_HOME"), home)
}

/// The sessions directory for a `CODEX_HOME` value and a home directory; empty values are unset.
fn sessions_dir(codex_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let set = |v: Option<OsString>| v.filter(|v| !v.is_empty()).map(PathBuf::from);
    set(codex_home)
        .or_else(|| set(home).map(|h| h.join(".codex")))
        .map(|dir| dir.join("sessions"))
}

/// Read every session file up to `YYYY/MM/DD/` below `sessions_dir`.
pub fn read(sessions_dir: &Path) -> Result<History, String> {
    let entries = std::fs::read_dir(sessions_dir)
        .map_err(|_| "Codex sessions directory not found or unreadable")?;
    let mut files = Vec::new();
    collect(entries, MAX_DEPTH, &mut files);
    files.sort();
    let mut history = History::default();
    let mut newest = Newest::default();
    for (file, modified) in files {
        if let Some(conversation) = read_session(&file, sessions_dir, &mut history) {
            newest.push(&mut history, conversation, modified);
        }
    }
    history
        .conversations
        .sort_by_key(|c| std::cmp::Reverse(c.created_at_ms));
    Ok(history)
}

/// A session file and when it was last modified.
type SessionFile = (PathBuf, Option<SystemTime>);

/// `*.jsonl` files in `entries` and, while `depth` allows, in real (non-symlink) subfolders.
fn collect(entries: std::fs::ReadDir, depth: usize, files: &mut Vec<SessionFile>) {
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
            let modified = entry.metadata().and_then(|m| m.modified()).ok();
            files.push((path, modified));
        }
    }
}

/// Read one session file. Oversized sessions count only as `skipped`; unreadable records
/// (malformed lines, unknown roles and content parts) and dropped injected parts are counted in
/// `history` only for sessions that were read to the end.
fn read_session(file: &Path, root: &Path, history: &mut History) -> Option<Conversation> {
    let mut lines = match Lines::open(file, MAX_FILE_BYTES) {
        Ok(lines) => lines,
        Err(refused) => {
            refused.count(history);
            return None;
        }
    };
    let mut session = None;
    let mut messages: Vec<Message> = Vec::new();
    let mut counts = (0, 0);
    let mut workspace = None;
    let mut created_at_ms = None;
    let mut outcome = None;
    let mut saw_meta = false;
    let mut saw_message_record = false;
    let mut gaps = 0;
    let mut dropped = 0;
    loop {
        let line = match lines.next_line() {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(Refused::TooLarge) => {
                history.skipped += 1;
                return None;
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
        if created_at_ms.is_none() {
            created_at_ms = v["timestamp"].as_str().and_then(rfc3339_ms);
        }
        let payload = &v["payload"];
        match v["type"].as_str() {
            Some("session_meta") if !saw_meta => {
                saw_meta = true;
                session = payload["id"].as_str().map(str::to_owned);
                workspace = absolute_dir(payload["cwd"].as_str());
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
        let mut images = 0;
        let texts: Vec<&str> = parts
            .iter()
            .filter(|p| match p["type"].as_str() {
                Some("input_text" | "output_text") => true,
                // Attached images carry no text; anything else is an unknown part.
                Some("input_image") => {
                    images += 1;
                    false
                }
                _ => {
                    gaps += 1;
                    false
                }
            })
            .filter_map(|p| p["text"].as_str())
            .filter(|t| {
                let keep = role == "assistant" || !injected(t);
                dropped += usize::from(!keep);
                keep
            })
            .collect();
        let mut text = texts.join("\n").trim().to_string();
        if let Some(marker) = image_marker(images).filter(|_| role == "user") {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&marker);
        }
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
                    return None;
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
    history.dropped_injected += dropped;
    if messages.is_empty() {
        if saw_message_record {
            history.empty_conversations += 1;
        }
        return None;
    }
    let title = title(&messages);
    let complete = gaps == 0 && messages.last().is_some_and(|m| m.role == "assistant");
    Some(Conversation {
        id: format!("codex:{}", session_id(session.as_deref(), file, root)),
        title,
        workspace,
        created_at_ms,
        source: "codex",
        messages,
        files_changed: vec![],
        files_read: vec![],
        outcome,
        complete,
    })
}

/// Whether a user text part is context Codex injected rather than a typed prompt: it starts
/// with a known marker or consists only of tag blocks, one with a tool-style name.
fn injected(text: &str) -> bool {
    let start = text.trim_start();
    // Matched case-insensitively at the start of any text part, as Codex itself does.
    let opens = |prefix: &str| {
        start
            .get(..prefix.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
    };
    let tag = |name: &str| {
        opens(&format!("<{name}"))
            && start[name.len() + 1..].starts_with(|c: char| c == '>' || c.is_whitespace())
    };
    INJECTED_TAGS.iter().any(|name| tag(name))
        || INJECTED_PREFIXES.iter().any(|p| opens(p))
        || injected_tag_blocks(text)
}

#[cfg(test)]
mod tests {
    use super::sessions_dir;
    use std::path::PathBuf;

    #[test]
    fn sessions_dir_prefers_codex_home_then_home() {
        let some = |s: &str| Some(s.into());
        assert_eq!(
            sessions_dir(some("/codex"), some("/home/me")),
            Some(PathBuf::from("/codex").join("sessions"))
        );
        assert_eq!(
            sessions_dir(some(""), some("/home/me")),
            Some(PathBuf::from("/home/me").join(".codex").join("sessions"))
        );
        assert_eq!(
            sessions_dir(None, some("/home/me")),
            Some(PathBuf::from("/home/me").join(".codex").join("sessions"))
        );
        assert_eq!(sessions_dir(None, some("")), None);
        assert_eq!(sessions_dir(None, None), None);
    }
}
