//! Shared synthetic Cursor storage fixture (no real data).
#![allow(dead_code)]
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;

fn kv(db: &Path, table: &str, rows: &[(String, serde_json::Value)]) {
    let c = Connection::open(db).unwrap();
    c.execute_batch(&format!(
        "CREATE TABLE {table}(key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)"
    ))
    .unwrap();
    for (k, v) in rows {
        c.execute(
            &format!("INSERT INTO {table}(key,value) VALUES(?,?)"),
            rusqlite::params![k, v.to_string().into_bytes()],
        )
        .unwrap();
    }
}
fn workspace(user: &Path, hash: &str, folder: &Path, rows: &[(String, serde_json::Value)]) {
    let dir = user.join("workspaceStorage").join(hash);
    std::fs::create_dir_all(&dir).unwrap();
    let uri = format!("file://{}", folder.display()).replace(' ', "%20");
    std::fs::write(
        dir.join("workspace.json"),
        json!({"folder":uri}).to_string(),
    )
    .unwrap();
    kv(&dir.join("state.vscdb"), "ItemTable", rows);
}
/// Synthetic layout modelled on Cursor's composer and legacy chat storage. No real data.
pub fn fixture(base: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let user = base.join("Cursor/User");
    let product = base.join("my product");
    let personal = base.join("personal");
    std::fs::create_dir_all(product.join("backend")).unwrap();
    std::fs::create_dir_all(&personal).unwrap();
    std::fs::create_dir_all(user.join("globalStorage")).unwrap();
    workspace(
        &user,
        "aaa",
        &product,
        &[
            (
                "composer.composerData".into(),
                json!({"allComposers":[{"composerId":"c-new"},{"composerId":"c-inline"}]}),
            ),
            (
                "workbench.panel.aichat.view.aichat.chatdata".into(),
                json!({"tabs":[{"tabId":"t1","chatTitle":"Legacy limiter chat","lastSendTime":5,"bubbles":[{"type":"user","text":"How do we cap requests?"},{"type":"ai","rawText":"Use the shared middleware."}]}]}),
            ),
        ],
    );
    workspace(
        &user,
        "bbb",
        &personal,
        &[(
            "composer.composerData".into(),
            json!({"allComposers":[{"composerId":"c-personal"}]}),
        )],
    );
    kv(
        &user.join("globalStorage/state.vscdb"),
        "cursorDiskKV",
        &[
            (
                "composerData:c-new".into(),
                json!({"composerId":"c-new","name":"Tenant rate limiter","createdAt":30,"fullConversationHeadersOnly":[{"bubbleId":"b1","type":1},{"bubbleId":"b2","type":2},{"bubbleId":"b3","type":2},{"bubbleId":"missing","type":2}]}),
            ),
            (
                "bubbleId:c-new:b1".into(),
                json!({"type":1,"text":"Add tenant limits. key=sk-SYNTHETICabcdef1234567890"}),
            ),
            (
                "bubbleId:c-new:b2".into(),
                json!({"type":2,"text":"","toolFormerData":{"name":"read_file"}}),
            ),
            (
                "bubbleId:c-new:b3".into(),
                json!({"type":2,"text":"Added middleware in backend.","thinking":{"text":"HIDDEN_REASONING"}}),
            ),
            (
                "composerData:c-inline".into(),
                json!({"composerId":"c-inline","name":"","createdAt":20,"conversation":[{"bubbleId":"i1","type":1,"text":"Inline question"},{"bubbleId":"i2","type":2,"text":"-----BEGIN PRIVATE KEY-----\nABC\n-----END PRIVATE KEY-----\nDone"}]}),
            ),
            (
                "composerData:c-personal".into(),
                json!({"composerId":"c-personal","name":"Personal","createdAt":10,"conversation":[{"bubbleId":"p1","type":1,"text":"PERSONAL_CANARY"}]}),
            ),
            (
                "composerData:c-orphan".into(),
                json!({"composerId":"c-orphan","name":"Orphan","createdAt":40,"conversation":[{"bubbleId":"o1","type":1,"text":"ORPHAN_CANARY"}]}),
            ),
            ("composerData:c-broken".into(), json!("not an object")),
        ],
    );
    (user, product)
}

fn slug(path: &Path) -> String {
    let p = path.canonicalize().unwrap();
    p.to_string_lossy()
        .trim_start_matches('/')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Synthetic `~/.cursor` with agent transcripts for the product, a personal folder and a
/// project whose path no longer exists. Returns the fake `.cursor` directory.
pub fn transcripts(base: &Path, product: &Path) -> std::path::PathBuf {
    let dot = base.join("dot-cursor");
    let write = |slug: &str, name: &str, lines: &[serde_json::Value], extra: &str| {
        let dir = dot.join("projects").join(slug).join("agent-transcripts");
        std::fs::create_dir_all(&dir).unwrap();
        let mut body: String = lines.iter().map(|l| l.to_string() + "\n").collect();
        body.push_str(extra);
        std::fs::write(dir.join(name), body).unwrap();
    };
    let msg =
        |role: &str, items: serde_json::Value| json!({"role": role, "message": {"content": items}});
    write(
        &slug(product),
        "11111111-aaaa.jsonl",
        &[
            msg(
                "user",
                json!([{"type":"text","text":"<attached_files>ATTACHED_CANARY</attached_files>\n<user_query>\nReuse the limiter for billing\n</user_query>"}]),
            ),
            msg(
                "assistant",
                json!([{"type":"thinking","text":"THINKING_CANARY"},{"type":"text","text":"Checking the middleware."}]),
            ),
            msg(
                "assistant",
                json!([{"type":"tool_use","name":"read_file","input":{"path":"TOOL_CANARY"}}]),
            ),
            msg(
                "assistant",
                json!([{"type":"text","text":"Billing now uses the shared limiter."}]),
            ),
        ],
        "not json\n",
    );
    let personal = base.join("personal");
    write(
        &slug(&personal),
        "22222222-bbbb.jsonl",
        &[msg(
            "user",
            json!([{"type":"text","text":"PERSONAL_TRANSCRIPT_CANARY"}]),
        )],
        "",
    );
    write(
        "Users-nobody-deleted-project",
        "33333333-cccc.jsonl",
        &[msg("user", json!([{"type":"text","text":"Gone project"}]))],
        "",
    );
    dot
}
