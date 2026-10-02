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
}

#[derive(Serialize, Debug, Default)]
pub struct History {
    pub conversations: Vec<Conversation>,
    pub unreadable_records: usize,
    /// Conversations found but with no readable visible text (likely an unknown layout).
    pub empty_conversations: usize,
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

pub fn read(user_dir: &Path) -> Result<History, String> {
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
                        id: format!("cursor-composer:{id}"),
                        messages,
                    });
                }
            }
        }
    }
    history
        .conversations
        .sort_by_key(|c| std::cmp::Reverse(c.created_at_ms));
    Ok(history)
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
        out.push_str(&rebuilt);
    }
    out
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
    if let Some(home) = std::env::var_os("HOME") {
        let dot = PathBuf::from(home).join(".cursor");
        report["dot_cursor_entries"] = json!(names(&dot));
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
    let h = read(user_dir)?;
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
        let mut entry = json!({"table": t, "rows": rows});
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
                *prefixes
                    .entry(
                        k.split(':')
                            .next()
                            .unwrap_or_default()
                            .chars()
                            .take(80)
                            .collect(),
                    )
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
