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
//! subagent sidechains and parts made only of `<tag>…</tag>` blocks (slash commands, bash mode,
//! IDE context, system reminders, task notifications) are ignored. Records that look like
//! messages but have an unknown type are counted, never guessed. Symlinks are never followed out
//! of the projects directory.
//!
//! Session files are read in place rather than snapshotted: they are append-only JSONL, so a
//! concurrent write can at worst leave a partial last line, which is counted as a malformed
//! record and marks the session incomplete. Copying would add I/O without a consistency gain.
use crate::cursor_history::{Conversation, History, Message};
use serde_json::Value;
use std::io::{BufRead, Read};
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
    let Some((handle, len)) = open_regular(file) else {
        history.unreadable_records += 1;
        return;
    };
    if len > MAX_FILE_BYTES {
        history.skipped += 1;
        return;
    }
    let session = file
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut messages: Vec<Message> = Vec::new();
    let mut workspace = None;
    let mut created_at_ms = None;
    let mut outcome = None;
    let mut saw_message_record = false;
    let mut gaps = 0;
    // The file may grow after the size check; never read past the limit.
    let mut reader = std::io::BufReader::new(handle.take(MAX_FILE_BYTES + 1));
    let mut total = 0;
    let mut line = Vec::new();
    loop {
        line.clear();
        let Ok(n) = reader.read_until(b'\n', &mut line) else {
            gaps += 1;
            break;
        };
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_FILE_BYTES {
            history.skipped += 1;
            return;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
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
            workspace = v["cwd"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
        }
        let text = match role {
            "user" => prompt_text(&v),
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

/// Open `file` only if it is a regular file, not a symlink, returning the handle and its size.
/// On Unix the opened handle must be the same inode that was checked, so a symlink swapped in
/// between the check and the open is refused. Elsewhere a swap in that window is a residual
/// race (no `O_NOFOLLOW` without a new dependency).
fn open_regular(file: &Path) -> Option<(std::fs::File, u64)> {
    let checked = std::fs::symlink_metadata(file).ok()?;
    if !checked.file_type().is_file() {
        return None;
    }
    let handle = std::fs::File::open(file).ok()?;
    let opened = handle.metadata().ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (opened.dev(), opened.ino()) != (checked.dev(), checked.ino()) {
            return None;
        }
    }
    opened.is_file().then_some((handle, opened.len()))
}

/// The first non-blank line of the first prompt, at most 80 characters.
pub(crate) fn title(messages: &[Message]) -> String {
    messages
        .iter()
        .find(|m| m.role == "user")
        .and_then(|m| m.text.lines().find(|l| !l.trim().is_empty()))
        .map(|l| l.trim().chars().take(80).collect())
        .unwrap_or_else(|| "Untitled chat".into())
}

/// A typed prompt: string content or `text` parts, each kept only if the developer wrote it.
/// Meta, compact-summary and transcript-only records and tool results are not prompts.
fn prompt_text(v: &Value) -> Option<String> {
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
    joined(text_parts(content).filter(|t| typed(t)))
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
/// those made only of `<tag>…</tag>` blocks, those opening with a known injected block
/// (slash commands, bash mode, IDE context, reminders, notifications, hooks), bash-mode output
/// anywhere in the part, and interrupt markers.
fn typed(part: &str) -> bool {
    !(only_tag_blocks(part)
        || INJECTED_PREFIXES.iter().any(|p| part.starts_with(p))
        || part.contains("<bash-stdout>")
        || part.contains("<bash-stderr>")
        || part.starts_with("[Request interrupted by user"))
}

/// `s` is one or more `<name attrs>…</name>` blocks separated only by whitespace.
fn only_tag_blocks(s: &str) -> bool {
    let mut rest = s.trim();
    if rest.is_empty() {
        return false;
    }
    while !rest.is_empty() {
        let Some(after) = rest.strip_prefix('<') else {
            return false;
        };
        let name_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(after.len());
        let name = &after[..name_len];
        if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
            return false;
        }
        let tail = &after[name_len..];
        if !(tail.starts_with('>') || tail.starts_with(char::is_whitespace)) {
            return false;
        }
        let Some(open_end) = tail.find('>') else {
            return false;
        };
        let close = format!("</{name}>");
        let Some(at) = tail[open_end..].find(&close) else {
            return false;
        };
        rest = tail[open_end + at + close.len()..].trim_start();
    }
    true
}

/// Milliseconds since the Unix epoch for an RFC 3339 timestamp such as
/// `2026-10-15T10:00:00.250Z` or `2026-10-15T15:30:00+05:30`.
pub(crate) fn rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => return None,
    };
    if !(1..=days_in_month).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    if second > 60 {
        return None;
    }
    let mut i = 19;
    let mut millis = 0;
    if b[i] == b'.' {
        let start = i + 1;
        i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
        let digits: String = s[start..i].chars().chain("000".chars()).take(3).collect();
        millis = digits.parse().ok()?;
    }
    let offset_minutes = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let (h, m) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
            if h > 23 || m > 59 {
                return None;
            }
            let total = h * 60 + m;
            if *sign == b'-' {
                -total
            } else {
                total
            }
        }
        _ => return None,
    };
    // Days from civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1_000 + millis)
}

#[cfg(test)]
mod tests {
    use super::rfc3339_ms;

    #[test]
    fn recognises_injected_tag_blocks() {
        use super::only_tag_blocks as tags;
        assert!(tags("<a>x</a>"));
        assert!(tags(
            "<ide_selection path=\"p\">x <b>y</b></ide_selection>\n <c-d></c-d>"
        ));
        for not in [
            "",
            "plain",
            "<a>x</a> tail",
            "lead <a>x</a>",
            "<a>unclosed",
            "<1a>x</1a>",
            "<a>x</b>",
            "< a>x</a>",
        ] {
            assert!(!tags(not), "{not}");
        }
    }

    #[test]
    fn parses_rfc3339() {
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_ms("2026-10-15T10:00:00.000Z"),
            Some(1_792_058_400_000)
        );
        assert_eq!(
            rfc3339_ms("2026-10-15T15:30:00.250123+05:30"),
            Some(1_792_058_400_250)
        );
        assert_eq!(
            rfc3339_ms("2024-02-29T00:00:00-01:00"),
            Some(1_709_168_400_000)
        );
        for bad in [
            "",
            "2026-10-15",
            "2026-13-01T00:00:00Z",
            "2026-10-15T10:00:00",
            "x026-10-15T10:00:00Z",
            "2026-02-31T10:00:00Z",
            "2025-02-29T10:00:00Z",
            "2026-04-31T10:00:00Z",
        ] {
            assert_eq!(rfc3339_ms(bad), None, "{bad}");
        }
    }
}
