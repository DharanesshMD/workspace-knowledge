//! Read-only importer for VS Code Copilot Chat's local session history.
//!
//! VS Code's storage is undocumented and changes between versions. Observed layout:
//! `<User>/workspaceStorage/<hash>/chatSessions/<sessionId>.json`, one JSON document per
//! session with `sessionId`, optional `customTitle`, `creationDate` (ms) and `requests[]`, and
//! `<hash>/workspace.json` naming the folder (`{"folder":"file:///abs/path"}`). Each request
//! carries the prompt in `message.text` and the reply as `response[]` parts.
//!
//! Newer versions store a session as `<sessionId>.jsonl`, a mutation log: a `{"kind":0,"v":…}`
//! snapshot of the same document, then `{"kind":1,"k":path,"v":value}` (set the value at
//! `path`) and `{"kind":2,"k":path,"v":[items]|null,"i":n}` (truncate the array at `path` to
//! `n` when `i` is present, then append the items). Path components are object keys or array
//! indexes. The log is replayed in memory and the result read like a `.json` session; malformed
//! lines, unknown kinds and paths that do not resolve are counted and skipped. When a session
//! has both files, only the log is read.
//!
//! Only visible text is read: the prompt's `message.text` and markdown reply parts (parts
//! without a `kind`, `markdownContent` or `markdownVuln`). Prompt `parts` and `variableData`
//! (attached variables and files), the unsent draft in `inputState`, thinking, tool
//! invocations, edits, code-block URIs, inline references, progress, confirmations,
//! elicitations, commands, MCP notices, vulnerability details and error messages are ignored. Prompts made entirely of
//! `<tag ...>…</tag>` blocks are injected context, not typed text, and are dropped. Unknown part
//! kinds are counted, never guessed. A failed or cancelled request keeps only the reply text it
//! produced. Symlinks are never followed out of the workspace storage directory.
//!
//! Session files are read in place rather than snapshotted: VS Code rewrites a whole `.json`
//! document and appends whole lines to a log, so a read racing a write at worst sees malformed
//! JSON, which is counted as unreadable and never yields partial content.
//!
//! Known limits: only folder workspaces on the local disk are attributed (`file://` URIs);
//! remote, multi-root and empty-window chats have no workspace.
use crate::cursor_history::{Conversation, History, Message};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_WORKSPACE_BYTES: u64 = 64 * 1024;
const MAX_MESSAGES: usize = 5_000;
const MAX_LOG_LINES: usize = 200_000;

/// Response part kinds that never carry visible reply text.
const HIDDEN_KINDS: [&str; 15] = [
    "thinking",
    "toolInvocationSerialized",
    "prepareToolInvocation",
    "textEditGroup",
    "codeblockUri",
    "inlineReference",
    "undoStop",
    "progressTaskSerialized",
    "mcpServersStarting",
    "notebookEditGroup",
    "confirmation",
    "progressMessage",
    "elicitation",
    "elicitationSerialized",
    "command",
];

/// The VS Code `User` directory for the current OS.
pub fn default_dir() -> Option<PathBuf> {
    let home = || {
        std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .filter(|a| !a.is_empty())
            .map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|c| c.is_absolute())
            .or_else(|| home().map(|h| h.join(".config")))
    }
    .map(|base| base.join("Code").join("User"))
}

/// Read every `workspaceStorage/*/chatSessions/*.{json,jsonl}` session below the `User`
/// directory.
pub fn read(user_dir: &Path) -> Result<History, String> {
    let entries = std::fs::read_dir(user_dir.join("workspaceStorage"))
        .map_err(|_| "VS Code workspace storage not found or unreadable")?;
    let mut history = History::default();
    let mut files = Vec::new();
    for storage in entries {
        let Ok(storage) = storage else {
            history.unreadable_records += 1;
            continue;
        };
        if !storage.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let chats = storage.path().join("chatSessions");
        if !std::fs::symlink_metadata(&chats).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let Ok(dir) = std::fs::read_dir(&chats) else {
            history.unreadable_records += 1;
            continue;
        };
        let mut sessions = Vec::new();
        for file in dir {
            let Ok(file) = file else {
                history.unreadable_records += 1;
                continue;
            };
            let path = file.path();
            if !file.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            if path
                .extension()
                .is_some_and(|e| e == "json" || e == "jsonl")
            {
                sessions.push(path);
            }
        }
        let logs: Vec<PathBuf> = sessions
            .iter()
            .filter(|p| is_log(p))
            .map(|p| p.with_extension(""))
            .collect();
        sessions.retain(|p| is_log(p) || !logs.contains(&p.with_extension("")));
        if !sessions.is_empty() {
            let workspace = workspace(&storage.path());
            files.extend(sessions.into_iter().map(|s| (s, workspace.clone())));
        }
    }
    files.sort();
    for (file, workspace) in files {
        read_session(&file, workspace, &mut history);
    }
    history
        .conversations
        .sort_by_key(|c| std::cmp::Reverse(c.created_at_ms));
    Ok(history)
}

/// Read one session file into `history`. Oversized sessions count only as `skipped`;
/// unreadable records (malformed files, requests without prompt text, unknown part kinds) are
/// counted otherwise.
fn read_session(file: &Path, workspace: Option<PathBuf>, history: &mut History) {
    let Some(bytes) = read_regular(file, MAX_FILE_BYTES, history) else {
        return;
    };
    let (session, mut gaps) = if is_log(file) {
        let Some(replayed) = replay(&bytes, history) else {
            return;
        };
        replayed
    } else {
        let Ok(session) = serde_json::from_slice::<Value>(&bytes) else {
            history.unreadable_records += 1;
            return;
        };
        (session, 0)
    };
    let Some(requests) = session["requests"].as_array() else {
        history.unreadable_records += 1;
        return;
    };
    let id = session["sessionId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map_or_else(
            || {
                file.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into()
            },
            str::to_owned,
        );
    let mut messages: Vec<Message> = Vec::new();
    let mut outcome = None;
    for (index, request) in requests.iter().enumerate() {
        let request_id = request["requestId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map_or_else(|| format!("request-{index}"), str::to_owned);
        let prompt = match request["message"]["text"].as_str() {
            Some(text) => Some(text.trim())
                .filter(|t| !t.is_empty() && !tag_wrapped(t))
                .map(str::to_owned),
            None => {
                gaps += 1;
                None
            }
        };
        let reply = match &request["response"] {
            Value::Null => None,
            Value::Array(parts) => reply_text(parts, &mut gaps),
            _ => {
                gaps += 1;
                None
            }
        };
        if let Some(reply) = &reply {
            outcome = Some(reply.clone());
        }
        for (role, suffix, text) in [("user", "u", prompt), ("assistant", "a", reply)] {
            let Some(text) = text else { continue };
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
                    messages.push(Message {
                        id: format!("{request_id}:{suffix}"),
                        role: role.into(),
                        text,
                    });
                }
            }
        }
    }
    history.unreadable_records += gaps;
    if messages.is_empty() {
        history.empty_conversations += 1;
        return;
    }
    let title = session["customTitle"]
        .as_str()
        .and_then(first_line)
        .or_else(|| {
            messages
                .iter()
                .find(|m| m.role == "user")
                .and_then(|m| first_line(&m.text))
        })
        .unwrap_or_else(|| "Untitled chat".into());
    let complete = gaps == 0 && messages.last().is_some_and(|m| m.role == "assistant");
    history.conversations.push(Conversation {
        id: format!("copilot:{id}"),
        title,
        workspace,
        created_at_ms: session["creationDate"].as_i64(),
        source: "copilot",
        messages,
        files_changed: vec![],
        files_read: vec![],
        outcome,
        complete,
    });
}

/// Markdown reply parts concatenated as VS Code streams them. Where a hidden part separated two
/// pieces of text with no whitespace between them, a line break keeps their words apart.
fn reply_text(parts: &[Value], gaps: &mut usize) -> Option<String> {
    let mut text = String::new();
    let mut separated = false;
    for part in parts {
        let piece = match part["kind"].as_str() {
            None if part.get("kind").is_none() => markdown(&part["value"]),
            Some("markdownContent" | "markdownVuln") => {
                markdown(&part["content"]).or_else(|| markdown(&part["value"]))
            }
            Some(kind) if HIDDEN_KINDS.contains(&kind) => {
                separated = !text.is_empty();
                continue;
            }
            _ => None,
        };
        let Some(piece) = piece else {
            // Unknown kinds and markdown parts without a readable value are never guessed.
            *gaps += 1;
            separated = !text.is_empty();
            continue;
        };
        if separated
            && !text.ends_with(char::is_whitespace)
            && !piece.starts_with(char::is_whitespace)
        {
            text.push('\n');
        }
        separated = false;
        text.push_str(piece);
    }
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn is_log(file: &Path) -> bool {
    file.extension().is_some_and(|e| e == "jsonl")
}

/// The session a `.jsonl` mutation log describes, and how many of its lines could not be
/// applied. Logs without a leading snapshot count as unreadable; overlong logs as `skipped`.
fn replay(bytes: &[u8], history: &mut History) -> Option<(Value, usize)> {
    let lines = || {
        bytes
            .split(|b| *b == b'\n')
            .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
    };
    if lines().count() > MAX_LOG_LINES {
        history.skipped += 1;
        return None;
    }
    let mut lines = lines().map(serde_json::from_slice::<Value>);
    let snapshot = match lines.next() {
        Some(Ok(mut first)) if first["kind"] == 0 => first.get_mut("v").map(Value::take),
        _ => None,
    };
    let Some(mut session) = snapshot else {
        history.unreadable_records += 1;
        return None;
    };
    let mut gaps = 0;
    for op in lines {
        if !op.is_ok_and(|op| apply(&mut session, op)) {
            gaps += 1;
        }
    }
    Some((session, gaps))
}

/// Apply one log operation to `session`; `false` (and no change) if it cannot be applied.
fn apply(session: &mut Value, mut op: Value) -> bool {
    let value = op.get_mut("v").map(Value::take);
    let path = op["k"].as_array().map(Vec::as_slice);
    match (op["kind"].as_u64(), path, value) {
        (Some(0), _, Some(value)) => *session = value,
        (Some(1), Some([parent @ .., key]), Some(value)) => match (resolve(session, parent), key) {
            (Some(Value::Object(map)), Value::String(key)) => {
                map.insert(key.clone(), value);
            }
            (Some(Value::Array(items)), Value::Number(n)) => {
                let Some(slot) = n
                    .as_u64()
                    .and_then(|i| items.get_mut(usize::try_from(i).ok()?))
                else {
                    return false;
                };
                *slot = value;
            }
            _ => return false,
        },
        (Some(2), Some(path), Some(value)) => {
            let Some(Value::Array(items)) = resolve(session, path) else {
                return false;
            };
            let added = match value {
                Value::Array(added) => added,
                Value::Null => Vec::new(),
                _ => return false,
            };
            match &op["i"] {
                Value::Null => {}
                i => match i.as_u64().and_then(|i| usize::try_from(i).ok()) {
                    Some(i) if i <= items.len() => items.truncate(i),
                    _ => return false,
                },
            }
            items.extend(added);
        }
        _ => return false,
    }
    true
}

/// The value at `path` (object keys and array indexes) inside `value`.
fn resolve<'a>(mut value: &'a mut Value, path: &[Value]) -> Option<&'a mut Value> {
    for key in path {
        value = match (value, key) {
            (Value::Object(map), Value::String(key)) => map.get_mut(key)?,
            (Value::Array(items), Value::Number(n)) => {
                items.get_mut(usize::try_from(n.as_u64()?).ok()?)?
            }
            _ => return None,
        };
    }
    Some(value)
}

/// A markdown string: either a string or an object with a string `value`.
fn markdown(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value["value"].as_str())
}

/// The first non-blank line of `text`, trimmed, at most 80 characters.
fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(80).collect())
}

/// The folder a storage directory belongs to: `workspace.json`'s `folder` when it is a local
/// `file://` URI. Remote folders, multi-root workspaces and anything unreadable are `None`.
fn workspace(storage: &Path) -> Option<PathBuf> {
    let mut ignored = History::default();
    let bytes = read_regular(
        &storage.join("workspace.json"),
        MAX_WORKSPACE_BYTES,
        &mut ignored,
    )?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    file_uri_path(v["folder"].as_str()?)
}

/// The absolute local path of a `file:///…` URI with an empty or `localhost` authority,
/// percent-decoded. Anything else (other schemes, hosts, queries, bad escapes) is `None`.
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') || rest.contains(['?', '#']) {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(out).ok()?;
    if path.contains('\0') {
        return None;
    }
    // Windows folders look like `file:///c%3A/dir`.
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if cfg!(windows) && drive.is_ascii_alphabetic() => &path[1..],
        _ => &path,
    };
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

/// The contents of `file` if it is a regular file (not a symlink) of at most `limit` bytes.
/// Larger files count as `skipped`; files that cannot be read count as unreadable.
fn read_regular(file: &Path, limit: u64, history: &mut History) -> Option<Vec<u8>> {
    let checked = std::fs::symlink_metadata(file).ok()?;
    if !checked.file_type().is_file() {
        return None;
    }
    let unreadable = |history: &mut History| {
        history.unreadable_records += 1;
        None
    };
    let Ok(handle) = std::fs::File::open(file) else {
        return unreadable(history);
    };
    let Ok(opened) = handle.metadata() else {
        return unreadable(history);
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // The opened file must be the one checked, so a symlink swapped in is refused.
        if (opened.dev(), opened.ino()) != (checked.dev(), checked.ino()) {
            return None;
        }
    }
    if opened.len() > limit {
        history.skipped += 1;
        return None;
    }
    // The file may grow after the size check; never read past the limit.
    let mut bytes = Vec::new();
    if handle.take(limit + 1).read_to_end(&mut bytes).is_err() {
        return unreadable(history);
    }
    if bytes.len() as u64 > limit {
        history.skipped += 1;
        return None;
    }
    Some(bytes)
}

/// Whether `text`, trimmed, is one or more `<tag ...>…</tag>` blocks separated only by
/// whitespace. Tag names match `[A-Za-z][A-Za-z0-9_-]*`; any other text keeps the prompt.
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

#[cfg(test)]
mod tests {
    use super::file_uri_path;
    use std::path::PathBuf;

    #[test]
    fn decodes_local_file_uris_only() {
        #[cfg(unix)]
        {
            let ok = |uri: &str| file_uri_path(uri).map(|p| p.to_string_lossy().into_owned());
            assert_eq!(ok("file:///work/my%20app").as_deref(), Some("/work/my app"));
            assert_eq!(
                ok("file://localhost/work/app").as_deref(),
                Some("/work/app")
            );
            assert_eq!(ok("file:///w/%E2%9C%93").as_deref(), Some("/w/\u{2713}"));
        }
        #[cfg(windows)]
        assert_eq!(
            file_uri_path("file:///c%3A/Users/me/app"),
            Some(PathBuf::from("c:/Users/me/app"))
        );
        for bad in [
            "",
            "/work/app",
            "file:/work/app",
            "file://server/share/app",
            "vscode-remote://ssh-remote%2Bbox/work/app",
            "file:///work/bad%2",
            "file:///work/bad%zz",
            "file:///work/%FF",
            "file:///work/nul%00",
            "file:///work/app?x=1",
            "file:///work/app#frag",
        ] {
            assert_eq!(file_uri_path(bad), None::<PathBuf>, "{bad}");
        }
    }
}
