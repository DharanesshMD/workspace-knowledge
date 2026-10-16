//! Agent history readers, exercised only against synthetic fixtures written to temp dirs.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use wk_adapters::cursor_history::{within, Conversation, History};

fn write_jsonl(path: &Path, records: &[Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body: Vec<String> = records.iter().map(Value::to_string).collect();
    std::fs::write(path, body.join("\n") + "\n").unwrap();
}

fn find<'a>(history: &'a History, id: &str) -> &'a Conversation {
    history
        .conversations
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("missing {id}"))
}

fn claude(kind: &str, uuid: &str, ts: &str, cwd: &str, content: Value) -> Value {
    json!({"type":kind,"sessionId":"s","uuid":uuid,"timestamp":ts,"cwd":cwd,"isSidechain":false,
           "message":{"role":kind,"content":content}})
}

const LONG: &str = "Fix the tenant rate limiter so that bursts from one tenant never starve the other tenants in the pool\nsecond line";

/// `projects/` with sessions for `/work/app` and a sibling `/work/app2`.
fn claude_fixture(root: &Path) -> PathBuf {
    let projects = root.join("projects");
    let app = projects.join("-work-app");
    let t = "2026-10-15T10:00:01.000Z";
    let mut sidechain = claude("user", "side1", t, "/work/app", json!("sidechain secret"));
    sidechain["isSidechain"] = json!(true);
    let mut side_reply = claude(
        "assistant",
        "side2",
        t,
        "/work/app",
        json!([{"type":"text","text":"sidechain secret"}]),
    );
    side_reply["isSidechain"] = json!(true);
    let mut meta = claude("user", "meta", t, "/work/app", json!("Caveat: meta secret"));
    meta["isMeta"] = json!(true);
    let mut tool_result_text = claude("user", "tr2", t, "/work/app", json!("tool output secret"));
    tool_result_text["toolUseResult"] = json!({"stdout":"tool output secret"});
    write_jsonl(
        &app.join("s1.jsonl"),
        &[
            json!({"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-15T10:00:00.000Z","sessionId":"s1"}),
            claude("user", "u1", t, "/work/app", json!(LONG)),
            meta,
            claude(
                "assistant",
                "a1",
                t,
                "/work/app",
                json!([
                    {"type":"thinking","thinking":"thinking secret"},
                    {"type":"text","text":"Reading the limiter."}
                ]),
            ),
            claude(
                "assistant",
                "a2",
                t,
                "/work/app",
                json!([
                    {"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/work/app/secret.rs"}}
                ]),
            ),
            claude(
                "user",
                "tr1",
                t,
                "/work/app",
                json!([
                    {"type":"tool_result","tool_use_id":"t1","content":"tool output secret"}
                ]),
            ),
            tool_result_text,
            sidechain,
            side_reply,
            claude(
                "assistant",
                "a3",
                t,
                "/work/app",
                json!([{"type":"text","text":"Done: limiter fixed."}]),
            ),
            claude(
                "user",
                "u2",
                t,
                "/work/app",
                json!([{"type":"text","text":"Now add tests"}]),
            ),
            claude(
                "assistant",
                "a4",
                t,
                "/work/app",
                json!([{"type":"text","text":"Tests added."}]),
            ),
        ],
    );
    // Interrupted session: malformed line, unknown message-like record, no reply.
    let s2 = app.join("s2.jsonl");
    write_jsonl(
        &s2,
        &[
            json!({"type":"summary","summary":"Earlier work","leafUuid":"x"}),
            claude(
                "user",
                "p1",
                "2026-10-15T09:00:00Z",
                "/work/app",
                json!("Partial"),
            ),
            json!({"type":"mystery","message":{"role":"user","content":"unknown secret"}}),
        ],
    );
    let mut text = std::fs::read_to_string(&s2).unwrap();
    text.push_str("{not json\n");
    std::fs::write(&s2, text).unwrap();
    // Only injected context: a conversation with no readable text.
    let mut only_meta = claude("user", "m", t, "/work/app", json!("meta only"));
    only_meta["isMeta"] = json!(true);
    write_jsonl(&app.join("s5.jsonl"), &[only_meta]);
    // Only bookkeeping records: not a conversation at all.
    write_jsonl(
        &app.join("s6.jsonl"),
        &[json!({"type":"summary","summary":"x","leafUuid":"y"})],
    );
    // Subagent transcripts and non-jsonl files are not sessions.
    write_jsonl(
        &app.join("s1/subagents/agent-1.jsonl"),
        &[
            claude("user", "x1", t, "/work/app", json!("subagent secret")),
            claude(
                "assistant",
                "x2",
                t,
                "/work/app",
                json!([{"type":"text","text":"subagent secret"}]),
            ),
        ],
    );
    std::fs::write(app.join("notes.txt"), "not a session").unwrap();
    // Oversized (sparse) file is skipped without being read.
    let big = std::fs::File::create(app.join("big.jsonl")).unwrap();
    big.set_len(33 * 1024 * 1024).unwrap();
    // A sibling folder whose name shares a prefix.
    write_jsonl(
        &projects.join("-work-app2/s3.jsonl"),
        &[
            claude(
                "user",
                "v1",
                "2026-10-15T15:30:00.250+05:30",
                "/work/app2",
                json!("Sibling prompt"),
            ),
            claude(
                "assistant",
                "v2",
                t,
                "/work/app2",
                json!([{"type":"text","text":"Sibling reply"}]),
            ),
        ],
    );
    projects
}

#[test]
fn claude_reads_visible_prompts_and_replies_only() {
    let d = tempfile::tempdir().unwrap();
    let projects = claude_fixture(d.path());
    let history = wk_adapters::claude_history::read(&projects).unwrap();
    let ids: Vec<_> = history
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["claude-code:s3", "claude-code:s1", "claude-code:s2"],
        "newest first; subagent, bookkeeping-only and non-jsonl files are not sessions"
    );
    let c = find(&history, "claude-code:s1");
    let got: Vec<_> = c
        .messages
        .iter()
        .map(|m| (m.id.as_str(), m.role.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("u1", "user", LONG),
            (
                "a1",
                "assistant",
                "Reading the limiter.\n\nDone: limiter fixed."
            ),
            ("u2", "user", "Now add tests"),
            ("a4", "assistant", "Tests added."),
        ]
    );
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(
        !all.contains("secret"),
        "hidden or tool content leaked: {all}"
    );
    assert_eq!(c.title, LONG.lines().next().unwrap()[..80]);
    assert_eq!(c.title.chars().count(), 80);
    assert_eq!(c.workspace.as_deref(), Some(Path::new("/work/app")));
    assert_eq!(c.created_at_ms, Some(1_792_058_400_000));
    assert_eq!(c.source, "claude-code");
    assert_eq!(c.outcome.as_deref(), Some("Tests added."));
    assert!(c.complete);
    assert!(c.files_changed.is_empty() && c.files_read.is_empty());

    let partial = find(&history, "claude-code:s2");
    assert_eq!(partial.messages.len(), 1);
    assert_eq!(partial.title, "Partial");
    assert!(!partial.complete, "no reply and unreadable records");
    assert_eq!(partial.outcome, None);
    assert_eq!(
        history.unreadable_records, 2,
        "malformed line and unknown message-like record are counted, never guessed"
    );
    assert_eq!(history.empty_conversations, 1);
    assert_eq!(history.skipped, 1, "oversized file skipped");

    let sibling = find(&history, "claude-code:s3");
    assert_eq!(sibling.created_at_ms, Some(1_792_058_400_250));
}

#[test]
fn claude_sessions_scope_to_the_folder_not_its_prefix_siblings() {
    let d = tempfile::tempdir().unwrap();
    let history = wk_adapters::claude_history::read(&claude_fixture(d.path())).unwrap();
    let ids = |root: &str| -> Vec<String> {
        within(&history, Path::new(root))
            .into_iter()
            .map(|c| c.id)
            .collect()
    };
    assert_eq!(ids("/work/app"), ["claude-code:s1", "claude-code:s2"]);
    assert_eq!(ids("/work/app2"), ["claude-code:s3"]);
}

#[test]
fn claude_skips_sessions_with_too_many_messages() {
    let d = tempfile::tempdir().unwrap();
    let t = "2026-10-15T10:00:00Z";
    let records: Vec<Value> = (0..5001)
        .map(|i| {
            if i % 2 == 0 {
                claude("user", &format!("u{i}"), t, "/work/app", json!("q"))
            } else {
                claude(
                    "assistant",
                    &format!("a{i}"),
                    t,
                    "/work/app",
                    json!([{"type":"text","text":"a"}]),
                )
            }
        })
        .collect();
    write_jsonl(&d.path().join("-work-app/huge.jsonl"), &records);
    write_jsonl(&d.path().join("-work-app/ok.jsonl"), &records[..5000]);
    let history = wk_adapters::claude_history::read(d.path()).unwrap();
    let ids: Vec<_> = history
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["claude-code:ok"]);
    assert_eq!(history.conversations[0].messages.len(), 5000);
    assert_eq!(history.skipped, 1);
}

#[cfg(unix)]
#[test]
fn claude_never_follows_symlinks_out_of_the_projects_dir() {
    let d = tempfile::tempdir().unwrap();
    let outside = d.path().join("outside");
    let t = "2026-10-15T10:00:00Z";
    write_jsonl(
        &outside.join("leak.jsonl"),
        &[claude("user", "l", t, "/work/app", json!("outside secret"))],
    );
    let projects = d.path().join("projects");
    std::fs::create_dir_all(projects.join("-work-app")).unwrap();
    std::os::unix::fs::symlink(&outside, projects.join("-linked")).unwrap();
    std::os::unix::fs::symlink(
        outside.join("leak.jsonl"),
        projects.join("-work-app/leak.jsonl"),
    )
    .unwrap();
    let history = wk_adapters::claude_history::read(&projects).unwrap();
    assert!(history.conversations.is_empty());
    assert_eq!(history.empty_conversations, 0);
}

#[test]
fn claude_missing_projects_dir_is_an_error() {
    let d = tempfile::tempdir().unwrap();
    assert!(wk_adapters::claude_history::read(&d.path().join("absent")).is_err());
}

#[test]
fn claude_skips_slash_command_and_local_command_records() {
    let d = tempfile::tempdir().unwrap();
    let t = "2026-10-15T10:00:00Z";
    let user = |uuid: &str, content: Value| claude("user", uuid, t, "/work/app", content);
    write_jsonl(
        &d.path().join("-work-app/cmd.jsonl"),
        &[
            user("c1", json!("<command-name>/clear</command-name>\n<command-message>clear secret</command-message>\n<command-args></command-args>")),
            user("c2", json!("<command-message>review secret</command-message>")),
            user("c3", json!("<command-args>args secret</command-args>")),
            user("c4", json!("<local-command-stdout>API_KEY=sk-test-secret-123</local-command-stdout>")),
            user("c5", json!("  \n<local-command-stderr>stderr secret</local-command-stderr>")),
            user("c6", json!([{"type":"text","text":"<local-command-caveat>Caveat: caveat secret</local-command-caveat>"}])),
            user("u1", json!("Explain <command-name> handling")),
            claude("assistant", "a1", t, "/work/app", json!([{"type":"text","text":"It is skipped."}])),
        ],
    );
    let history = wk_adapters::claude_history::read(d.path()).unwrap();
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(!all.contains("secret"), "command content leaked: {all}");
    let c = find(&history, "claude-code:cmd");
    let got: Vec<_> = c
        .messages
        .iter()
        .map(|m| (m.id.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("u1", "Explain <command-name> handling"),
            ("a1", "It is skipped.")
        ]
    );
    assert_eq!(c.title, "Explain <command-name> handling");
    assert_eq!(
        history.unreadable_records, 0,
        "command records are not unreadable"
    );
    assert!(c.complete);
}
