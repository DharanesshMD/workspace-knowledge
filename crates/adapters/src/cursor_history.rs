//! Read-only importer for Cursor's local chat history.
//!
//! Cursor's storage is undocumented and changes between versions. Supported layouts:
//! - global `state.vscdb` table `cursorDiskKV`: `composerData:<id>` with either an inline
//!   `conversation` array or `fullConversationHeadersOnly` plus `bubbleId:<id>:<bubble>` rows;
//! - per-workspace `state.vscdb` table `ItemTable`: `composer.composerData` (which composers
//!   belong to the workspace) and legacy `workbench.panel.aichat.view.aichat.chatdata` tabs.
//!
//! Databases are copied to a private temporary directory and opened read-only, so a running
//! Cursor is never locked or modified. Only visible `text` is read; thinking/tool payloads are
//! ignored. Unrecognised records are counted, never guessed.
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    pub role: String,
    pub text: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub workspace: Option<PathBuf>,
    pub created_at_ms: Option<i64>,
    pub source: &'static str,
    pub messages: Vec<Message>,
    /// Workspace-relative files the agent changed (edit/write/delete tool calls).
    pub files_changed: Vec<String>,
    /// Workspace-relative files or folders the agent read or searched.
    pub files_read: Vec<String>,
    /// The agent's final reply in the session: usually its closing report.
    pub outcome: Option<String>,
    /// Coverage is complete within the source boundary: every record of Cursor's session file
    /// was read and the session ends with the agent's reply. Not a claim about model context.
    pub complete: bool,
}

#[derive(Serialize, Debug, Default)]
pub struct History {
    pub conversations: Vec<Conversation>,
    pub unreadable_records: usize,
    /// Conversations found but with no readable visible text (likely an unknown layout).
    pub empty_conversations: usize,
    /// Sessions deliberately not read because they exceed the reader's size limits.
    pub skipped: usize,
    /// Prompt text parts dropped because the client injected them (IDE context, reminders,
    /// command output, environment context…) rather than the developer typing them.
    pub dropped_injected: usize,
}

/// Default Cursor user-data directory for the current OS.
pub fn default_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/Cursor/User"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Cursor/User"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/Cursor/User"))
    }
}

/// Copy a database (and WAL/SHM, so recent writes are visible) into `scratch`, open read-only.
fn snapshot(db: &Path, scratch: &Path, name: &str) -> Result<Option<Connection>, String> {
    if !db.is_file() {
        return Ok(None);
    }
    let target = scratch.join(name);
    std::fs::copy(db, &target).map_err(|_| "cannot snapshot Cursor database")?;
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", db.display()));
        if side.is_file() {
            std::fs::copy(&side, format!("{}{suffix}", target.display()))
                .map_err(|_| "cannot snapshot Cursor database")?;
        }
    }
    Connection::open_with_flags(&target, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map(Some)
        .map_err(|_| "cannot open Cursor database snapshot".into())
}

fn value(conn: &Connection, sql: &str, key: &str) -> Option<Value> {
    let raw: Option<Vec<u8>> = conn
        .query_row(sql, [key], |r| {
            let v = r.get_ref(0)?;
            Ok(v.as_bytes()
                .or_else(|_| v.as_str().map(str::as_bytes))
                .map(<[u8]>::to_vec)
                .ok())
        })
        .optional()
        .ok()
        .flatten()
        .flatten();
    raw.and_then(|b| serde_json::from_slice(&b).ok())
}

fn decode_file_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(out).ok()?;
    // Windows URIs look like file:///c:/dir.
    let path = if cfg!(windows) {
        path.trim_start_matches('/').to_string()
    } else {
        path
    };
    Some(PathBuf::from(path))
}

fn text_of(v: &Value) -> Option<String> {
    let t = v["text"].as_str().or_else(|| v["rawText"].as_str())?.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn role(v: &Value) -> Option<&'static str> {
    match (&v["type"], v["type"].as_str()) {
        (Value::Number(n), _) if n.as_i64() == Some(1) => Some("user"),
        (Value::Number(n), _) if n.as_i64() == Some(2) => Some("assistant"),
        (_, Some("user")) => Some("user"),
        (_, Some("ai" | "assistant")) => Some("assistant"),
        _ => None,
    }
}

/// Read chats from Cursor's user-data directory and `~/.cursor` agent transcripts.
pub fn read(user_dir: &Path) -> Result<History, String> {
    let dot = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cursor"));
    read_with(user_dir, dot.as_deref())
}

/// `read` with an explicit `~/.cursor` location (tests must never read the real one).
pub fn read_with(user_dir: &Path, dot_cursor: Option<&Path>) -> Result<History, String> {
    let scratch = tempfile::tempdir().map_err(|_| "cannot create private scratch directory")?;
    let mut history = History::default();
    let mut owner: BTreeMap<String, PathBuf> = BTreeMap::new();

    // Workspace storage: composer ownership and legacy chat tabs.
    if let Ok(entries) = std::fs::read_dir(user_dir.join("workspaceStorage")) {
        for (n, entry) in entries.flatten().enumerate() {
            let dir = entry.path();
            let folder = std::fs::read_to_string(dir.join("workspace.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| v["folder"].as_str().and_then(decode_file_uri));
            let Some(conn) = snapshot(
                &dir.join("state.vscdb"),
                scratch.path(),
                &format!("ws{n}.db"),
            )?
            else {
                continue;
            };
            const ITEM: &str = "SELECT value FROM ItemTable WHERE key=?";
            if let (Some(folder), Some(v)) = (&folder, value(&conn, ITEM, "composer.composerData"))
            {
                for c in v["allComposers"].as_array().into_iter().flatten() {
                    if let Some(id) = c["composerId"].as_str() {
                        owner.insert(id.to_string(), folder.clone());
                    }
                }
            }
            if let Some(v) = value(&conn, ITEM, "workbench.panel.aichat.view.aichat.chatdata") {
                for tab in v["tabs"].as_array().into_iter().flatten() {
                    let id = tab["tabId"].as_str().unwrap_or_default().to_string();
                    let mut messages = Vec::new();
                    for (i, b) in tab["bubbles"].as_array().into_iter().flatten().enumerate() {
                        if let (Some(role), Some(text)) = (role(b), text_of(b)) {
                            let mid = b["id"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| format!("{id}-{i}"));
                            messages.push(Message {
                                id: mid,
                                role: role.into(),
                                text,
                            });
                        }
                    }
                    if id.is_empty() {
                        history.unreadable_records += 1;
                    } else if messages.is_empty() {
                        history.empty_conversations += 1;
                    } else {
                        history.conversations.push(Conversation {
                            title: tab["chatTitle"].as_str().unwrap_or("Untitled chat").into(),
                            id: format!("cursor-chat:{id}"),
                            workspace: folder.clone(),
                            created_at_ms: tab["lastSendTime"].as_i64(),
                            source: "cursor-legacy-chat",
                            messages,
                            files_changed: vec![],
                            files_read: vec![],
                            outcome: None,
                            complete: false,
                        });
                    }
                }
            }
        }
    }

    // Global composer (chat/agent) storage.
    if let Some(conn) = snapshot(
        &user_dir.join("globalStorage/state.vscdb"),
        scratch.path(),
        "global.db",
    )? {
        let has_kv: bool = conn
            .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='cursorDiskKV')", [], |r| r.get(0))
            .unwrap_or(false);
        if has_kv {
            const KV: &str = "SELECT value FROM cursorDiskKV WHERE key=?";
            let keys: Vec<String> = conn
                .prepare(
                    "SELECT key FROM cursorDiskKV WHERE key LIKE 'composerData:%' ORDER BY key",
                )
                .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
                .map_err(|_| "unrecognised Cursor global database layout")?;
            for key in keys {
                let Some(c) = value(&conn, KV, &key) else {
                    history.unreadable_records += 1;
                    continue;
                };
                let id = key.trim_start_matches("composerData:").to_string();
                let mut messages = Vec::new();
                if let Some(inline) = c["conversation"].as_array().filter(|a| !a.is_empty()) {
                    for (i, b) in inline.iter().enumerate() {
                        if let (Some(role), Some(text)) = (role(b), text_of(b)) {
                            let mid = b["bubbleId"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| format!("{id}-{i}"));
                            messages.push(Message {
                                id: mid,
                                role: role.into(),
                                text,
                            });
                        }
                    }
                } else {
                    for h in c["fullConversationHeadersOnly"]
                        .as_array()
                        .into_iter()
                        .flatten()
                    {
                        let Some(bid) = h["bubbleId"].as_str() else {
                            continue;
                        };
                        match value(&conn, KV, &format!("bubbleId:{id}:{bid}")) {
                            Some(b) => {
                                if let (Some(role), Some(text)) =
                                    (role(&b).or_else(|| role(h)), text_of(&b))
                                {
                                    messages.push(Message {
                                        id: bid.into(),
                                        role: role.into(),
                                        text,
                                    });
                                }
                            }
                            None => history.unreadable_records += 1,
                        }
                    }
                }
                if messages.is_empty() {
                    history.empty_conversations += 1;
                } else {
                    history.conversations.push(Conversation {
                        title: c["name"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("Untitled chat")
                            .into(),
                        workspace: owner.get(&id).cloned(),
                        created_at_ms: c["createdAt"].as_i64(),
                        source: "cursor-composer",
                        files_changed: vec![],
                        files_read: vec![],
                        outcome: None,
                        complete: false,
                        id: format!("cursor-composer:{id}"),
                        messages,
                    });
                }
            }
        }
    }
    if let Some(dot) = dot_cursor {
        read_transcripts(&dot.join("projects"), &mut history);
    }
    history
        .conversations
        .sort_by_key(|c| std::cmp::Reverse(c.created_at_ms));
    Ok(history)
}

/// Cursor agent transcripts: `projects/<path-slug>/agent-transcripts/**/*.jsonl`, one JSON
/// record per line: `{"role":"user"|"assistant","message":{"content":[{"type":"text","text":..}]}}`.
/// Only `text` items are read. Consecutive same-role records are merged into one message.
fn read_transcripts(projects: &Path, history: &mut History) {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(projects)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for project in dirs {
        let slug = project
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let workspace = resolve_slug(&slug);
        let root = project.join("agent-transcripts");
        let mut files: Vec<PathBuf> = walk_files(&root, 0)
            .into_iter()
            .filter(|f| f.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        files.sort();
        for file in files {
            if file.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > 32 * 1024 * 1024 {
                history.unreadable_records += 1;
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file) else {
                history.unreadable_records += 1;
                continue;
            };
            let rel = file.strip_prefix(&root).unwrap_or(&file).with_extension("");
            let id = rel.to_string_lossy().replace(['/', '\\'], "_");
            let mut messages: Vec<Message> = Vec::new();
            let mut changed = std::collections::BTreeSet::new();
            let mut read = std::collections::BTreeSet::new();
            let mut outcome = None;
            let mut gaps = 0;
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let Ok(v) = serde_json::from_str::<Value>(line) else {
                    history.unreadable_records += 1;
                    gaps += 1;
                    continue;
                };
                let role = match v["role"].as_str() {
                    Some("user") => "user",
                    Some("assistant") => "assistant",
                    _ => continue,
                };
                if role == "assistant" {
                    collect_tool_paths(&v, workspace.as_deref(), &mut changed, &mut read);
                }
                let body: Vec<&str> = v["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["type"] == "text")
                    .filter_map(|c| c["text"].as_str())
                    .collect();
                let mut body = body.join("\n").trim().to_string();
                if role == "user" {
                    body = user_query(&body);
                }
                if body.is_empty() {
                    continue;
                }
                if role == "assistant" {
                    outcome = Some(body.clone());
                }
                match messages.last_mut() {
                    Some(last) if last.role == role => {
                        last.text.push_str("\n\n");
                        last.text.push_str(&body);
                    }
                    _ => messages.push(Message {
                        id: format!("{id}-{}", messages.len()),
                        role: role.into(),
                        text: body,
                    }),
                }
            }
            if messages.is_empty() {
                history.empty_conversations += 1;
                continue;
            }
            let title = messages
                .iter()
                .find(|m| m.role == "user")
                .and_then(|m| m.text.lines().find(|l| !l.trim().is_empty()))
                .map(|l| {
                    let t: String = l.trim().chars().take(80).collect();
                    t
                })
                .unwrap_or_else(|| "Untitled chat".into());
            let modified = file
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            let complete = gaps == 0
                && messages.last().is_some_and(|m| m.role == "assistant")
                && outcome.is_some();
            history.conversations.push(Conversation {
                id: format!("cursor-transcript:{id}"),
                title,
                workspace: workspace.clone(),
                created_at_ms: modified,
                source: "cursor-agent-transcript",
                messages,
                files_read: read.difference(&changed).cloned().collect(),
                files_changed: changed.into_iter().collect(),
                complete,
                outcome,
            });
        }
    }
}

/// Prefer the explicit `<user_query>` section Cursor wraps around the typed request.
/// Without that section, remove Cursor-injected `<tag>…</tag>` context blocks; if nothing the
/// developer wrote remains, the record is not a user message.
fn user_query(text: &str) -> String {
    match (text.find("<user_query>"), text.find("</user_query>")) {
        (Some(a), Some(b)) if b > a => text[a + "<user_query>".len()..b].trim().to_string(),
        _ => strip_context_blocks(text).trim().to_string(),
    }
}

fn strip_context_blocks(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        let tail = &rest[start + 1..];
        let name: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        // Cursor context tags use snake_case names; ordinary HTML such as <div> is kept.
        let closes = name.contains('_') && tail[name.len()..].starts_with('>');
        let end = format!("</{name}>");
        match (closes, tail.find(&end)) {
            (true, Some(e)) => {
                out.push_str(&rest[..start]);
                rest = &tail[e + end.len()..];
            }
            _ => {
                out.push_str(&rest[..=start]);
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// File paths from tool calls, kept only when inside the workspace, as relative paths.
fn collect_tool_paths(
    record: &Value,
    workspace: Option<&Path>,
    changed: &mut std::collections::BTreeSet<String>,
    read: &mut std::collections::BTreeSet<String>,
) {
    let Some(root) = workspace else { return };
    let relative = |raw: &str| crate::team_context::relative(root, raw);
    for item in record["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if item["type"] != "tool_use" {
            continue;
        }
        let input = &item["input"];
        match item["name"].as_str().unwrap_or_default() {
            "StrReplace" | "Write" | "Delete" | "Edit" | "MultiEdit" | "EditNotebook" => {
                if let Some(p) = input["path"]
                    .as_str()
                    .or_else(|| input["file_path"].as_str())
                    .and_then(relative)
                {
                    changed.insert(p);
                }
            }
            "ApplyPatch" => {
                let patch = input
                    .as_str()
                    .or_else(|| input["patch"].as_str())
                    .unwrap_or_default();
                for line in patch.lines() {
                    for marker in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
                        if let Some(p) = line.strip_prefix(marker).and_then(relative) {
                            changed.insert(p);
                        }
                    }
                }
            }
            "Read" | "ReadFile" | "Grep" => {
                if let Some(p) = input["path"].as_str().and_then(relative) {
                    read.insert(p);
                }
            }
            "ReadLints" => {
                for p in input["paths"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_str())
                    .filter_map(relative)
                {
                    read.insert(p);
                }
            }
            "Glob" => {
                if let Some(p) = input["target_directory"].as_str().and_then(relative) {
                    read.insert(p);
                }
            }
            _ => {}
        }
    }
}

/// Resolve a Cursor project slug (path with separators replaced by `-`) back to an existing
/// directory by walking the real filesystem. Ambiguous or missing paths stay unattributed.
pub fn resolve_slug(slug: &str) -> Option<PathBuf> {
    fn norm(s: &str) -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect()
    }
    fn walk(dir: &Path, rest: &str, depth: usize) -> Option<PathBuf> {
        if depth > 40 || rest.is_empty() {
            return None;
        }
        let mut children: Vec<String> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        children.sort();
        for child in children {
            let n = norm(&child);
            if n.is_empty() {
                continue;
            }
            if rest == n {
                return Some(dir.join(&child));
            }
            if let Some(tail) = rest.strip_prefix(&format!("{n}-")) {
                if let Some(found) = walk(&dir.join(&child), tail, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    let slug = norm(slug);
    if cfg!(windows) {
        // `C:\Users\x` becomes `c--users-x`: drive letter, then the rest.
        let (drive, rest) = slug.trim_start_matches('-').split_once('-')?;
        walk(
            Path::new(&format!("{drive}:\\")),
            rest.trim_start_matches('-'),
            0,
        )
    } else {
        walk(Path::new("/"), slug.trim_start_matches('-'), 0)
    }
}

/// Keep conversations attributed to `root` or a folder inside it. Unattributed conversations
/// are never assumed to belong to a workspace.
pub fn within(history: &History, root: &Path) -> Vec<Conversation> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    history
        .conversations
        .iter()
        .filter(|c| {
            c.workspace.as_ref().is_some_and(|w| {
                let w = w.canonicalize().unwrap_or_else(|_| w.clone());
                w.starts_with(&root)
            })
        })
        .cloned()
        .collect()
}

/// Mask likely credentials before display or export. Conservative and not exhaustive.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_key_block = false;
    for line in text.split_inclusive('\n') {
        if line.contains("-----BEGIN") {
            in_key_block = true;
        }
        if in_key_block {
            if line.contains("-----END") {
                in_key_block = false;
            }
            out.push_str("[REDACTED KEY MATERIAL]\n");
            continue;
        }
        let mut rebuilt = String::new();
        for (i, word) in line.split(' ').enumerate() {
            if i > 0 {
                rebuilt.push(' ');
            }
            rebuilt.push_str(if secret_like(word) {
                "[REDACTED]"
            } else {
                word
            });
        }
        out.push_str(&mask_assignments(&rebuilt));
    }
    out
}

/// Names whose assigned value is a credential, matched case-insensitively as a whole name or
/// as its last `_`, `-` or `.` separated part (`DB_PASSWORD`, `config.token`).
const CREDENTIAL_KEYS: [&str; 14] = [
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "private_key",
    "client_secret",
    "auth",
    "bearer",
    "authorization",
];

/// Mask the value of every credential assignment in `line`, whatever its length, keeping the
/// key: `key=value`, `key: value`, `"key": "value"` and `Authorization: Bearer value`. A value
/// ends at whitespace, a quote, a comma or the end of the line.
fn mask_assignments(line: &str) -> String {
    let b = line.as_bytes();
    let name_byte = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.');
    let mut out = String::with_capacity(line.len());
    let (mut copied, mut i) = (0, 0);
    while i < b.len() {
        if !name_byte(b[i]) || (i > 0 && name_byte(b[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && name_byte(b[i]) {
            i += 1;
        }
        if credential_key(&line[start..i]) {
            if let Some((from, to)) = assigned_value(b, i) {
                out.push_str(&line[copied..from]);
                out.push_str("[REDACTED]");
                (copied, i) = (to, to);
            }
        }
    }
    out.push_str(&line[copied..]);
    out
}

fn credential_key(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    CREDENTIAL_KEYS.iter().any(|key| {
        name.strip_suffix(key)
            .is_some_and(|rest| rest.is_empty() || rest.ends_with(['_', '-', '.']))
    })
}

/// The byte range of the value assigned after a key ending at `i`, if there is one.
fn assigned_value(b: &[u8], mut i: usize) -> Option<(usize, usize)> {
    let skip_spaces = |mut i: usize| {
        while i < b.len() && matches!(b[i], b' ' | b'\t') {
            i += 1;
        }
        i
    };
    let is_quote = |c: u8| matches!(c, b'"' | b'\'' | b'`');
    if b.get(i).copied().is_some_and(is_quote) {
        i += 1;
    }
    i = skip_spaces(i);
    if !matches!(b.get(i), Some(b'=' | b':')) || matches!(b.get(i + 1), Some(b'=' | b'/')) {
        return None;
    }
    i = skip_spaces(i + 1);
    if b.get(i).copied().is_some_and(is_quote) {
        i += 1;
    }
    let end = |i: usize| {
        (i..b.len())
            .find(|&j| b[j].is_ascii_whitespace() || is_quote(b[j]) || b[j] == b',')
            .unwrap_or(b.len())
    };
    let mut value_end = end(i);
    // `Authorization: Bearer <value>`: keep the scheme, mask the credential after it.
    let scheme = &b[i..value_end];
    if ["bearer", "basic", "token"]
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s.as_bytes()))
        && b.get(value_end) == Some(&b' ')
    {
        i = skip_spaces(value_end);
        value_end = end(i);
    }
    (value_end > i).then_some((i, value_end))
}

fn secret_like(word: &str) -> bool {
    let w = word.trim_matches(|c: char| "\"'`,;:()[]{}<>\n\r".contains(c));
    let w = w.rsplit_once('=').map_or(w, |(_, v)| v);
    let prefixed = [
        "sk-",
        "sk_",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AKIA",
        "eyJ",
        "AIza",
    ]
    .iter()
    .any(|p| w.starts_with(p) && w.len() >= 16);
    let opaque = w.len() >= 32
        && !w.contains('/')
        && w.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_+=.".contains(c))
        && w.chars().any(|c| c.is_ascii_digit())
        && w.chars().any(|c| c.is_ascii_alphabetic());
    prefixed || opaque
}

/// Structure-only report for adapting to new Cursor versions: file/table names, key-name
/// prefixes with counts, and JSON field names with types. Never includes values or text.
pub fn diagnose(user_dir: &Path) -> Result<Value, String> {
    let dot = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cursor"));
    diagnose_with(user_dir, dot.as_deref())
}

/// `diagnose` with an explicit `~/.cursor` location (used by tests).
pub fn diagnose_with(user_dir: &Path, dot_cursor: Option<&Path>) -> Result<Value, String> {
    use serde_json::json;
    let scratch = tempfile::tempdir().map_err(|_| "cannot create private scratch directory")?;
    let names = |dir: &Path| -> Vec<Value> {
        let mut v: Vec<Value> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| {
                json!({"name": e.file_name().to_string_lossy(), "dir": e.path().is_dir(),
                "bytes": e.metadata().map(|m| m.len()).unwrap_or(0)})
            })
            .collect();
        v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        v.truncate(60);
        v
    };
    let ws = user_dir.join("workspaceStorage");
    let ws_dirs: Vec<PathBuf> = std::fs::read_dir(&ws)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    let mut report = json!({
        "user_dir_entries": names(user_dir),
        "global_storage_entries": names(&user_dir.join("globalStorage")),
        "workspace_dirs": ws_dirs.len(),
        "workspace_dirs_with_state_db": ws_dirs.iter().filter(|d| d.join("state.vscdb").is_file()).count(),
        "workspace_dirs_with_folder": ws_dirs.iter().filter(|d| std::fs::read_to_string(d.join("workspace.json")).ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok()).is_some_and(|v| v["folder"].is_string())).count(),
    });
    if let Some(dot) = dot_cursor {
        report["dot_cursor_entries"] = json!(names(dot));
    }
    if let Some(conn) = snapshot(
        &user_dir.join("globalStorage/state.vscdb"),
        scratch.path(),
        "global.db",
    )? {
        report["global_db"] = db_shape(&conn);
    }
    // Merge key prefixes from every workspace database.
    let mut ws_keys: BTreeMap<String, usize> = BTreeMap::new();
    let mut ws_shapes = serde_json::Map::new();
    for (n, d) in ws_dirs.iter().enumerate() {
        if let Some(conn) = snapshot(&d.join("state.vscdb"), scratch.path(), &format!("ws{n}.db"))?
        {
            let shape = db_shape(&conn);
            for t in shape["tables"].as_array().into_iter().flatten() {
                for (k, c) in t["key_prefixes"].as_object().into_iter().flatten() {
                    *ws_keys.entry(k.clone()).or_default() += c.as_u64().unwrap_or(0) as usize;
                }
                for (k, v) in t["chat_like_value_fields"]
                    .as_object()
                    .into_iter()
                    .flatten()
                {
                    ws_shapes.entry(k.clone()).or_insert(v.clone());
                }
            }
        }
    }
    report["workspace_db_key_prefixes"] = json!(ws_keys
        .into_iter()
        .filter(|(k, _)| chat_like(k))
        .collect::<BTreeMap<_, _>>());
    report["workspace_chat_like_value_fields"] = Value::Object(ws_shapes);
    let h = read_with(user_dir, dot_cursor)?;
    let mut dbs = Vec::new();
    let mut n = 0;
    let mut roots = vec![user_dir.join("globalStorage")];
    if let Some(dot) = dot_cursor {
        roots.push(dot.to_path_buf());
    }
    for root in &roots {
        find_databases(root, root, 0, &mut |path, shown| {
            n += 1;
            if let Ok(Some(conn)) = snapshot(path, scratch.path(), &format!("probe{n}.db")) {
                let mut shape = db_shape(&conn);
                shape["path"] = json!(shown);
                dbs.push(shape);
            }
        });
    }
    report["sqlite_databases"] = json!(dbs);
    if let Some(dot) = dot_cursor {
        report["projects"] = projects_shape(&dot.join("projects"));
    }
    report["reader_result"] = json!({"conversations": h.conversations.len(), "attributed": h.conversations.iter().filter(|c| c.workspace.is_some()).count(), "empty_conversations": h.empty_conversations, "unreadable_records": h.unreadable_records});
    Ok(report)
}

fn chat_like(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "composer",
        "chat",
        "bubble",
        "aiservice",
        "conversation",
        "agent",
        "prompt",
        "message",
    ]
    .iter()
    .any(|w| k.contains(w))
}

/// Field names and JSON types only, recursing two levels; never values.
fn field_types(v: &Value, depth: usize) -> Value {
    use serde_json::json;
    match v {
        Value::Object(m) if depth > 0 => {
            // Only schema-like names are reported; ID/path/free-text keys collapse to one entry.
            let schema = |k: &str| {
                k.len() <= 40
                    && k.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            };
            let mut out = serde_json::Map::new();
            let mut dynamic = 0;
            for (k, x) in m.iter().take(200) {
                if schema(k) {
                    out.insert(k.clone(), field_types(x, depth - 1));
                } else {
                    dynamic += 1;
                    out.entry("<dynamic-key>")
                        .or_insert_with(|| field_types(x, depth - 1));
                }
            }
            if dynamic > 0 {
                out.insert("<dynamic-key-count>".into(), json!(dynamic));
            }
            Value::Object(out)
        }
        Value::Object(_) => json!("object"),
        Value::Array(a) if depth > 0 => {
            json!({"array_len": a.len(), "first_item": a.first().map(|x| field_types(x, depth - 1))})
        }
        Value::Array(a) => json!(format!("array[{}]", a.len())),
        Value::String(_) => json!("string"),
        Value::Number(_) => json!("number"),
        Value::Bool(_) => json!("bool"),
        Value::Null => json!("null"),
    }
}

fn db_shape(conn: &Connection) -> Value {
    use serde_json::json;
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for t in tables {
        if !t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
            .unwrap_or(-1);
        let columns: Vec<Value> = conn.prepare(&format!("PRAGMA table_info({t})"))
            .and_then(|mut st| st.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.collect::<Result<Vec<_>, _>>())
            .unwrap_or_default().into_iter()
            .map(|(name, ty)| json!({"name": if name.len() <= 40 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { name } else { "<masked>".into() }, "type": ty}))
            .collect();
        let mut entry = json!({"table": t, "rows": rows, "columns": columns});
        let keyed = conn
            .prepare(&format!("SELECT key FROM {t} LIMIT 0"))
            .is_ok();
        if keyed {
            let keys: Vec<String> = conn
                .prepare(&format!("SELECT key FROM {t}"))
                .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
                .unwrap_or_default();
            let mut prefixes: BTreeMap<String, usize> = BTreeMap::new();
            for k in &keys {
                // Prefix up to the first ':' only; IDs after it are not reported.
                // Non-schema-like prefixes (IDs, paths, text) are grouped as <other>.
                let prefix = k.split(':').next().unwrap_or_default();
                let schema = prefix.len() <= 80
                    && prefix
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
                    && !(prefix.len() >= 16
                        && prefix.chars().filter(|c| c.is_ascii_digit()).count() >= 4);
                *prefixes
                    .entry(if schema {
                        prefix.to_string()
                    } else {
                        "<other>".into()
                    })
                    .or_default() += 1;
            }
            let mut samples = serde_json::Map::new();
            for (prefix, _) in prefixes.iter().filter(|(p, _)| chat_like(p)).take(20) {
                let matching: Vec<&String> = keys
                    .iter()
                    .filter(|k| k.split(':').next() == Some(prefix.as_str()))
                    .collect();
                // Up to eight records spread across the range, so old and new layouts both appear.
                let step = matching.len().div_ceil(8).max(1);
                let sample = matching
                    .iter()
                    .step_by(step)
                    .take(8)
                    .filter_map(|k| value(conn, &format!("SELECT value FROM {t} WHERE key=?"), k))
                    .map(|v| field_types(&v, 3))
                    .collect::<Vec<_>>();
                samples.insert(prefix.clone(), json!(sample));
            }
            entry["key_prefixes"] = json!(prefixes
                .into_iter()
                .filter(|(p, _)| t != "ItemTable" || chat_like(p))
                .take(60)
                .collect::<BTreeMap<_, _>>());
            entry["chat_like_value_fields"] = Value::Object(samples);
        }
        out.push(entry);
    }
    json!({"tables": out})
}

/// A path segment safe to display: project folders and ID-like names are masked.
fn display_segment(segment: &str, parent: Option<&str>) -> String {
    let id_like = segment.len() >= 16 && segment.chars().any(|c| c.is_ascii_digit());
    let plain = segment.len() <= 40
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if parent == Some("projects") {
        "<project>".into()
    } else if id_like || !plain {
        "<id>".into()
    } else {
        segment.into()
    }
}

fn find_databases(root: &Path, dir: &Path, depth: usize, found: &mut dyn FnMut(&Path, String)) {
    if depth > 4 {
        return;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if [
            "extensions",
            "node_modules",
            "browser-logs",
            "workspaceStorage",
            "History",
        ]
        .contains(&name.as_str())
        {
            continue;
        }
        if path.is_dir() && !entry.file_type().is_ok_and(|t| t.is_symlink()) {
            find_databases(root, &path, depth + 1, found);
        } else if [".db", ".vscdb", ".sqlite", ".sqlite3"]
            .iter()
            .any(|e| name.ends_with(e))
        {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            let shown = parts
                .iter()
                .enumerate()
                .map(|(i, seg)| display_segment(seg, i.checked_sub(1).map(|j| parts[j].as_str())))
                .collect::<Vec<_>>()
                .join("/");
            found(
                &path,
                format!(
                    "{}/{shown}",
                    root.file_name().unwrap_or_default().to_string_lossy()
                ),
            );
        }
    }
}

/// Aggregate layout of ~/.cursor/projects without project names or file contents.
fn projects_shape(dir: &Path) -> Value {
    use serde_json::json;
    let projects: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    let mut children: BTreeMap<String, BTreeMap<String, (usize, u64)>> = BTreeMap::new();
    let mut samples: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let markers = [
        "user:",
        "assistant:",
        "human:",
        "ai:",
        "a:",
        "u:",
        "<user>",
        "<assistant>",
        "<user_query>",
        "## user",
        "## assistant",
        "**user**",
        "**assistant**",
    ];
    for project in &projects {
        for child in std::fs::read_dir(project).into_iter().flatten().flatten() {
            let cname = display_segment(&child.file_name().to_string_lossy(), None);
            let mut files: Vec<PathBuf> = if child.path().is_dir() {
                walk_files(&child.path(), 0)
            } else {
                vec![child.path()]
            };
            files.sort();
            let entry = children.entry(cname.clone()).or_default();
            for f in &files {
                let ext = f
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_else(|| "<none>".into());
                let e = entry.entry(ext).or_default();
                e.0 += 1;
                e.1 += f.metadata().map(|m| m.len()).unwrap_or(0);
            }
            let bucket = samples.entry(cname).or_default();
            for f in files.iter().rev().take(3) {
                if bucket.len() >= 6 {
                    break;
                }
                let Ok(text) = std::fs::read_to_string(f) else {
                    continue;
                };
                let ext = f
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default();
                if ext == "jsonl" || ext == "json" {
                    let first = if ext == "jsonl" {
                        text.lines()
                            .find(|l| !l.trim().is_empty())
                            .unwrap_or_default()
                    } else {
                        text.as_str()
                    };
                    let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
                    let shape = serde_json::from_str::<Value>(first)
                        .map(|v| field_types(&v, 5))
                        .unwrap_or_else(|_| json!("unparseable"));
                    let roles: BTreeMap<String, usize> = text
                        .lines()
                        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                        .filter_map(|v| {
                            v["role"]
                                .as_str()
                                .or_else(|| v["type"].as_str())
                                .map(str::to_owned)
                        })
                        .filter(|r| {
                            r.len() <= 24
                                && r.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        })
                        .fold(BTreeMap::new(), |mut m, r| {
                            *m.entry(r).or_default() += 1;
                            m
                        });
                    // Content item types and the field names of non-text items (e.g. tool calls).
                    let mut item_types: BTreeMap<String, usize> = BTreeMap::new();
                    let mut item_fields = serde_json::Map::new();
                    for v in text
                        .lines()
                        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                    {
                        for item in v["message"]["content"].as_array().into_iter().flatten() {
                            let kind = item["type"]
                                .as_str()
                                .filter(|k| {
                                    k.len() <= 32
                                        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                                })
                                .unwrap_or("<other>");
                            *item_types.entry(kind.to_string()).or_default() += 1;
                            if kind != "text" {
                                item_fields
                                    .entry(kind.to_string())
                                    .or_insert_with(|| field_types(item, 3));
                            }
                        }
                    }
                    bucket.push(json!({"ext": ext, "records": lines, "first_record_fields": shape, "role_or_type_counts": roles, "content_item_types": item_types, "non_text_item_fields": item_fields}));
                } else if matches!(ext.as_str(), "txt" | "md" | "log") {
                    let mut counts = BTreeMap::new();
                    for line in text.lines() {
                        let l = line.trim_start().to_ascii_lowercase();
                        if let Some(m) = markers.iter().find(|m| l.starts_with(**m)) {
                            *counts.entry(m.to_string()).or_insert(0usize) += 1;
                        }
                    }
                    bucket.push(json!({"ext": ext, "lines": text.lines().count(), "role_marker_line_counts": counts}));
                }
            }
        }
    }
    let children: BTreeMap<String, Value> = children
        .into_iter()
        .map(|(k, exts)| {
            (
                k,
                json!(exts
                    .into_iter()
                    .map(|(e, (n, b))| (e, json!({"files": n, "bytes": b})))
                    .collect::<BTreeMap<_, _>>()),
            )
        })
        .collect();
    json!({"project_count": projects.len(), "children": children, "samples": samples})
}

fn walk_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if depth > 3 {
        return out;
    }
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if e.file_type().is_ok_and(|t| t.is_symlink()) {
            continue;
        }
        if p.is_dir() {
            out.extend(walk_files(&p, depth + 1));
        } else {
            out.push(p);
        }
    }
    out
}
