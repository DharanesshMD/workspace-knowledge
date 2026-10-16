//! Read-only importer for VS Code Copilot Chat's local session history.
//!
//! VS Code's storage is undocumented and changes between versions. Observed layout:
//! `<User>/workspaceStorage/<hash>/chatSessions/<sessionId>.json`, one JSON document per
//! session with `sessionId`, optional `customTitle`, `creationDate` (ms, an integer or a float;
//! the first request's `timestamp` when absent) and `requests[]`, and
//! `<hash>/workspace.json` naming the folder (`{"folder":"file:///abs/path"}`). Each request
//! carries the prompt in `message.text` and the reply as `response[]` parts.
//!
//! Newer versions store a session as `<sessionId>.jsonl`, a mutation log: a `{"kind":0,"v":…}`
//! snapshot of the same document, then `{"kind":1,"k":path,"v":value}` (set the value at
//! `path`) and `{"kind":2,"k":path,"v":[items]|null,"i":n}` (truncate the array at `path` to
//! `n` when `i` is present, then append the items). Path components are object keys or array
//! indexes. The log is replayed in memory and the result read like a `.json` session; malformed
//! lines, unknown kinds, paths that do not resolve and paths longer than 16 components (which
//! would let repeated operations nest values without bound) are counted and skipped. When a
//! session has both files the log is read; only if the log cannot be read at all (oversized,
//! unreadable, no leading snapshot) is the `.json` read instead, marked incomplete. A session
//! copied into several storage folders is imported once, from the most recently modified file.
//!
//! Only visible text is read: the prompt's `message.text` and markdown reply parts (parts
//! without a `kind`, `markdownContent` or `markdownVuln`). Prompt `parts` and `variableData`
//! (attached variables and files), the unsent draft in `inputState`, thinking, tool
//! invocations, edits, code-block URIs, progress, confirmations, elicitations, commands, MCP
//! notices, vulnerability details and error messages are ignored. Inline references read as the
//! referenced file's base name, never its path, so replies stay readable.
//! Copilot keeps injected context in `variableData`, never in `message.text`, so a prompt is
//! kept whole even when it is made of `<tag>…</tag>` blocks. Unknown part kinds are counted,
//! never guessed. A failed or cancelled request keeps only the reply text it
//! produced. Symlinks are never followed out of the workspace storage directory.
//!
//! Session files are read in place rather than snapshotted: VS Code rewrites a whole `.json`
//! document and appends whole lines to a log, so a read racing a write at worst sees malformed
//! JSON, which is counted as unreadable and never yields partial content.
//!
//! Known limits: only folder workspaces on the local disk are attributed (`file://` URIs without
//! `.` or `..` components); remote, multi-root and empty-window chats have no workspace.
use crate::cursor_history::{Conversation, History, Message};
use crate::history_common::{file_uri_path, first_line, read_regular, session_id, title};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_WORKSPACE_BYTES: u64 = 64 * 1024;
const MAX_MESSAGES: usize = 5_000;
const MAX_LOG_LINES: usize = 200_000;
/// Longest log operation path applied. Paths only ever address a few levels into a session;
/// capping them bounds how deep repeated operations can nest values, so a crafted log cannot
/// build a tree deep enough to overflow the stack when it is dropped.
const MAX_LOG_PATH: usize = 16;

/// Response part kinds that never carry visible reply text.
const HIDDEN_KINDS: [&str; 14] = [
    "thinking",
    "toolInvocationSerialized",
    "prepareToolInvocation",
    "textEditGroup",
    "codeblockUri",
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
                let modified = file.metadata().and_then(|m| m.modified()).ok();
                sessions.push((path, modified));
            }
        }
        // A session stored both ways is read from its log; the `.json` is only a fallback.
        let logged: HashSet<PathBuf> = sessions
            .iter()
            .filter(|(p, _)| is_log(p))
            .map(|(p, _)| p.with_extension(""))
            .collect();
        sessions.retain(|(p, _)| is_log(p) || !logged.contains(&p.with_extension("")));
        if !sessions.is_empty() {
            let workspace = workspace(&storage.path());
            files.extend(sessions.into_iter().map(|(file, modified)| {
                let fallback = Some(file.with_extension("json"))
                    .filter(|json| is_log(&file) && json.is_file());
                (file, fallback, modified, workspace.clone())
            }));
        }
    }
    files.sort();
    // The same session can be copied into several storage folders; keep the newest copy.
    let mut newest: HashMap<String, (Option<SystemTime>, usize)> = HashMap::new();
    for (file, fallback, modified, workspace) in files {
        let Some(conversation) = read_session(&file, fallback.as_deref(), workspace, &mut history)
        else {
            continue;
        };
        match newest.get_mut(&conversation.id) {
            Some((kept, index)) => {
                if modified > *kept {
                    *kept = modified;
                    history.conversations[*index] = conversation;
                }
            }
            None => {
                newest.insert(
                    conversation.id.clone(),
                    (modified, history.conversations.len()),
                );
                history.conversations.push(conversation);
            }
        }
    }
    history
        .conversations
        .sort_by_key(|c| std::cmp::Reverse(c.created_at_ms));
    Ok(history)
}

/// Read one session file. Oversized sessions count only as `skipped`; unreadable records
/// (malformed files, requests without prompt text, unknown part kinds) are counted otherwise.
/// When a log cannot be read at all, its `fallback` `.json` copy is read instead and the
/// conversation is marked incomplete, since the log holds the newer state.
fn read_session(
    file: &Path,
    fallback: Option<&Path>,
    workspace: Option<PathBuf>,
    history: &mut History,
) -> Option<Conversation> {
    let ((session, mut gaps), fell_back) = match load(file, history) {
        Some(loaded) => (loaded, false),
        None => (load(fallback?, history)?, true),
    };
    let requests = session["requests"].as_array()?;
    let id = session_id(session["sessionId"].as_str(), file);
    let mut messages: Vec<Message> = Vec::new();
    let mut outcome = None;
    for (index, request) in requests.iter().enumerate() {
        let request_id = request["requestId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map_or_else(|| format!("request-{index}"), str::to_owned);
        let prompt = match request["message"]["text"].as_str() {
            Some(text) => Some(text.trim())
                .filter(|t| !t.is_empty())
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
                        return None;
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
        return None;
    }
    let title = session["customTitle"]
        .as_str()
        .and_then(first_line)
        .unwrap_or_else(|| title(&messages));
    let complete =
        !fell_back && gaps == 0 && messages.last().is_some_and(|m| m.role == "assistant");
    // Older sessions store `creationDate` as a float; without one, the first request's time.
    let created_at_ms =
        millis(&session["creationDate"]).or_else(|| millis(&requests.first()?["timestamp"]));
    Some(Conversation {
        id: format!("copilot:{id}"),
        title,
        workspace,
        created_at_ms,
        source: "copilot",
        messages,
        files_changed: vec![],
        files_read: vec![],
        outcome,
        complete,
    })
}

/// The session in `file` and how many of its log lines could not be applied, or `None` (counted)
/// when the file cannot be read or holds no `requests` list.
fn load(file: &Path, history: &mut History) -> Option<(Value, usize)> {
    let bytes = match read_regular(file, MAX_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(refused) => {
            refused.count(history);
            return None;
        }
    };
    let loaded = if is_log(file) {
        replay(&bytes, history)?
    } else {
        let Ok(session) = serde_json::from_slice::<Value>(&bytes) else {
            history.unreadable_records += 1;
            return None;
        };
        (session, 0)
    };
    if !loaded.0["requests"].is_array() {
        history.unreadable_records += 1;
        return None;
    }
    Some(loaded)
}

/// Epoch milliseconds stored as an integer or a finite float.
fn millis(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        let f = value.as_f64().filter(|f| f.is_finite())?;
        // Saturating, so out-of-range values clamp rather than wrap.
        Some(f as i64)
    })
}

/// Markdown reply parts concatenated as VS Code streams them. Where a hidden part separated two
/// pieces of text with no whitespace between them, a line break keeps their words apart.
fn reply_text(parts: &[Value], gaps: &mut usize) -> Option<String> {
    let mut text = String::new();
    let mut separated = false;
    for part in parts {
        let piece = match part["kind"].as_str() {
            None if part.get("kind").is_none() => markdown(&part["value"]).map(Cow::Borrowed),
            Some("markdownContent" | "markdownVuln") => markdown(&part["content"])
                .or_else(|| markdown(&part["value"]))
                .map(Cow::Borrowed),
            Some("inlineReference") => match reference_name(&part["inlineReference"]) {
                Some(name) => Some(Cow::Owned(name)),
                None => {
                    separated = !text.is_empty();
                    continue;
                }
            },
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
        text.push_str(&piece);
    }
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The base name of the file an inline reference points at (a URI, a location or a symbol's
/// location), never its full path or a display name. `None` when there is no usable name.
fn reference_name(reference: &Value) -> Option<String> {
    let uri = [reference, &reference["uri"], &reference["location"]["uri"]]
        .into_iter()
        .find_map(|uri| uri["path"].as_str().or_else(|| uri["fsPath"].as_str()))?;
    let name = uri.rsplit(['/', '\\']).next()?;
    (!matches!(name, "" | "." | "..")).then(|| name.to_owned())
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
    let path = op["k"]
        .as_array()
        .map(Vec::as_slice)
        .filter(|path| path.len() <= MAX_LOG_PATH);
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

/// The folder a storage directory belongs to: `workspace.json`'s `folder` when it is a local
/// `file://` URI. Remote folders, multi-root workspaces and anything unreadable are `None`.
fn workspace(storage: &Path) -> Option<PathBuf> {
    let bytes = read_regular(&storage.join("workspace.json"), MAX_WORKSPACE_BYTES).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    file_uri_path(v["folder"].as_str()?)
}
