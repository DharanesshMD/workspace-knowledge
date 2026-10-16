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

fn codex(ts: &str, kind: &str, payload: Value) -> Value {
    json!({"timestamp":ts,"type":kind,"payload":payload})
}

fn codex_msg(role: &str, part: &str, text: &str) -> Value {
    json!({"type":"message","role":role,"content":[{"type":part,"text":text}]})
}

fn codex_meta(ts: &str, id: &str, cwd: &str) -> Value {
    codex(
        ts,
        "session_meta",
        json!({"id":id,"timestamp":ts,"cwd":cwd,"originator":"codex_cli_rs","cli_version":"0.0.0",
               "instructions":"base instructions secret"}),
    )
}

/// `sessions/YYYY/MM/DD/*.jsonl` with sessions for `/work/app` and a sibling `/work/app2`.
fn codex_fixture(root: &Path) -> PathBuf {
    let sessions = root.join("sessions");
    let t = "2026-10-15T10:00:01.000Z";
    let user = |text: &str| codex(t, "response_item", codex_msg("user", "input_text", text));
    let reply = |text: &str| {
        codex(
            t,
            "response_item",
            codex_msg("assistant", "output_text", text),
        )
    };
    write_jsonl(
        &sessions.join("2026/10/15/rollout-2026-10-15T10-00-00-s1.jsonl"),
        &[
            codex_meta("2026-10-15T10:00:00.000Z", "s1", "/work/app"),
            codex(
                t,
                "response_item",
                codex_msg("developer", "input_text", "<permissions instructions>developer secret"),
            ),
            codex(t, "response_item", codex_msg("system", "input_text", "system secret")),
            user("<user_instructions>agents secret</user_instructions>"),
            user("# AGENTS.md instructions for /work/app\n\n<INSTRUCTIONS>agents md secret</INSTRUCTIONS>"),
            user("  <environment_context>\n<cwd>/work/app</cwd> env secret\n</environment_context>"),
            user("<permissions instructions>permissions secret</permissions instructions>"),
            user("<ENVIRONMENT_CONTEXT>upper-case env secret</ENVIRONMENT_CONTEXT>"),
            codex(t, "turn_context", json!({"cwd":"/work/app","model":"m","summary":"turn secret"})),
            codex(t, "event_msg", json!({"type":"user_message","message":"event secret"})),
            user(LONG),
            codex(
                t,
                "response_item",
                json!({"type":"reasoning","summary":[{"type":"summary_text","text":"reasoning secret"}],
                       "encrypted_content":"secret"}),
            ),
            reply("Reading the limiter."),
            codex(
                t,
                "response_item",
                json!({"type":"function_call","name":"shell","arguments":"{\"cmd\":\"cat secret\"}","call_id":"c1"}),
            ),
            codex(
                t,
                "response_item",
                json!({"type":"function_call_output","call_id":"c1","output":"tool output secret"}),
            ),
            codex(
                t,
                "response_item",
                json!({"type":"custom_tool_call","name":"apply_patch","input":"patch secret","call_id":"c2"}),
            ),
            codex(
                t,
                "response_item",
                json!({"type":"custom_tool_call_output","call_id":"c2","output":"patch output secret"}),
            ),
            reply("Done: limiter fixed."),
            codex(t, "event_msg", json!({"type":"agent_message","message":"event secret"})),
            user("<skill>\n<name>x</name>skill secret</skill>"),
            user("<user_shell_command>\nls\nshell output secret\n</user_shell_command>"),
            user("<turn_aborted>aborted secret</turn_aborted>"),
            user("<subagent_notification>subagent secret</subagent_notification>"),
            user("Now add tests"),
            json!({"timestamp":t,"type":"response_item","payload":{"type":"message","role":"user",
                   "content":[{"type":"input_image","image_url":"data:image/png;base64,secret"},
                              {"type":"input_text","text":"See <environment_context> above"}]}}),
            reply("Tests added."),
        ],
    );
    // No session_meta, nested under another date: interrupted, malformed and unknown records.
    let s2 = sessions.join("2026/10/14/rollout-s2.jsonl");
    write_jsonl(
        &s2,
        &[
            codex(
                "2026-10-14T09:00:00Z",
                "response_item",
                codex_msg("user", "input_text", "Partial"),
            ),
            codex(
                t,
                "response_item",
                codex_msg("critic", "output_text", "unknown secret"),
            ),
        ],
    );
    let mut text = std::fs::read_to_string(&s2).unwrap();
    text.push_str("{not json\n");
    std::fs::write(&s2, text).unwrap();
    // Only injected context: a conversation with no readable text.
    write_jsonl(
        &sessions.join("2026/10/15/rollout-s5.jsonl"),
        &[
            codex_meta(t, "s5", "/work/app"),
            user("<environment_context>env only</environment_context>"),
        ],
    );
    // Only bookkeeping records: not a conversation at all.
    write_jsonl(
        &sessions.join("2026/10/15/rollout-s6.jsonl"),
        &[
            codex_meta(t, "s6", "/work/app"),
            codex(t, "turn_context", json!({"cwd":"/work/app"})),
        ],
    );
    // Deeper than YYYY/MM/DD and non-jsonl files are not sessions.
    write_jsonl(
        &sessions.join("2026/10/15/extra/deep.jsonl"),
        &[
            codex_meta(t, "deep", "/work/app"),
            user("deep secret"),
            reply("deep secret"),
        ],
    );
    std::fs::write(sessions.join("2026/10/15/notes.txt"), "not a session").unwrap();
    // Oversized (sparse) file is skipped without being read.
    let big = std::fs::File::create(sessions.join("2026/10/15/big.jsonl")).unwrap();
    big.set_len(33 * 1024 * 1024).unwrap();
    // A sibling folder whose name shares a prefix.
    write_jsonl(
        &sessions.join("2026/10/16/rollout-s3.jsonl"),
        &[
            codex_meta("2026-10-16T15:30:00.250+05:30", "s3", "/work/app2"),
            user("Sibling prompt"),
            reply("Sibling reply"),
        ],
    );
    sessions
}

#[test]
fn codex_reads_visible_prompts_and_replies_only() {
    let d = tempfile::tempdir().unwrap();
    let sessions = codex_fixture(d.path());
    let history = wk_adapters::codex_history::read(&sessions).unwrap();
    let ids: Vec<_> = history
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["codex:s3", "codex:s1", "codex:rollout-s2"],
        "newest first; too-deep, bookkeeping-only and non-jsonl files are not sessions"
    );
    let c = find(&history, "codex:s1");
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
            (
                "u2",
                "user",
                "Now add tests\n\nSee <environment_context> above"
            ),
            ("a2", "assistant", "Tests added."),
        ]
    );
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(
        !all.contains("secret"),
        "hidden, injected or tool content leaked: {all}"
    );
    assert_eq!(c.title, LONG.lines().next().unwrap()[..80]);
    assert_eq!(c.workspace.as_deref(), Some(Path::new("/work/app")));
    assert_eq!(c.created_at_ms, Some(1_792_058_400_000));
    assert_eq!(c.source, "codex");
    assert_eq!(c.outcome.as_deref(), Some("Tests added."));
    assert!(c.complete);
    assert!(c.files_changed.is_empty() && c.files_read.is_empty());

    let partial = find(&history, "codex:rollout-s2");
    assert_eq!(partial.workspace, None, "no session_meta: never selected");
    assert_eq!(partial.messages.len(), 1);
    assert_eq!(partial.title, "Partial");
    assert!(!partial.complete, "no reply and unreadable records");
    assert_eq!(partial.outcome, None);
    assert_eq!(
        history.unreadable_records, 2,
        "malformed line and unknown role are counted, never guessed"
    );
    assert_eq!(history.empty_conversations, 1);
    assert_eq!(history.skipped, 1, "oversized file skipped");

    let sibling = find(&history, "codex:s3");
    assert_eq!(sibling.created_at_ms, Some(1_792_144_800_250));
}

#[test]
fn codex_sessions_scope_to_the_folder_not_its_prefix_siblings() {
    let d = tempfile::tempdir().unwrap();
    let history = wk_adapters::codex_history::read(&codex_fixture(d.path())).unwrap();
    let ids = |root: &str| -> Vec<String> {
        within(&history, Path::new(root))
            .into_iter()
            .map(|c| c.id)
            .collect()
    };
    assert_eq!(ids("/work/app"), ["codex:s1"]);
    assert_eq!(ids("/work/app2"), ["codex:s3"]);
}

#[test]
fn codex_skips_sessions_with_too_many_messages() {
    let d = tempfile::tempdir().unwrap();
    let t = "2026-10-15T10:00:00Z";
    let mut records = vec![codex_meta(t, "huge", "/work/app")];
    records.extend((0..5001).map(|i| {
        if i % 2 == 0 {
            codex(t, "response_item", codex_msg("user", "input_text", "q"))
        } else {
            codex(
                t,
                "response_item",
                codex_msg("assistant", "output_text", "a"),
            )
        }
    }));
    write_jsonl(&d.path().join("2026/10/15/huge.jsonl"), &records);
    records[0] = codex_meta(t, "ok", "/work/app");
    write_jsonl(&d.path().join("2026/10/15/ok.jsonl"), &records[..5001]);
    let history = wk_adapters::codex_history::read(d.path()).unwrap();
    let ids: Vec<_> = history
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["codex:ok"]);
    assert_eq!(history.conversations[0].messages.len(), 5000);
    assert_eq!(history.skipped, 1);
}

#[cfg(unix)]
#[test]
fn codex_never_follows_symlinks_out_of_the_sessions_dir() {
    let d = tempfile::tempdir().unwrap();
    let outside = d.path().join("outside");
    let t = "2026-10-15T10:00:00Z";
    write_jsonl(
        &outside.join("leak.jsonl"),
        &[
            codex_meta(t, "leak", "/work/app"),
            codex(
                t,
                "response_item",
                codex_msg("user", "input_text", "outside secret"),
            ),
        ],
    );
    let sessions = d.path().join("sessions");
    std::fs::create_dir_all(sessions.join("2026/10/15")).unwrap();
    std::os::unix::fs::symlink(&outside, sessions.join("2026/linked")).unwrap();
    std::os::unix::fs::symlink(
        outside.join("leak.jsonl"),
        sessions.join("2026/10/15/leak.jsonl"),
    )
    .unwrap();
    let history = wk_adapters::codex_history::read(&sessions).unwrap();
    assert!(history.conversations.is_empty());
    assert_eq!(history.empty_conversations, 0);
}

#[test]
fn codex_missing_sessions_dir_is_an_error() {
    let d = tempfile::tempdir().unwrap();
    assert!(wk_adapters::codex_history::read(&d.path().join("absent")).is_err());
}

#[test]
fn codex_drops_tag_wrapped_injected_parts_and_keeps_real_prompts() {
    let d = tempfile::tempdir().unwrap();
    let t = "2026-10-15T10:00:00Z";
    let parts = |texts: &[&str]| {
        let content: Vec<Value> = texts
            .iter()
            .map(|text| json!({"type":"input_text","text":text}))
            .collect();
        codex(
            t,
            "response_item",
            json!({"type":"message","role":"user","content":content}),
        )
    };
    let reply = |text: &str| {
        codex(
            t,
            "response_item",
            codex_msg("assistant", "output_text", text),
        )
    };
    write_jsonl(
        &d.path().join("2026/10/15/tags.jsonl"),
        &[
            codex_meta(t, "tags", "/work/app"),
            parts(&["<some_new_context>\nnew context secret\n</some_new_context>"]),
            parts(&["  <a-b k=\"v\">attr secret</a-b>\n\n<C9>second block secret</C9>  "]),
            parts(&[
                "<user_instructions>instructions secret</user_instructions>",
                "Real prompt first line\nmore",
                "# AGENTS.md instructions for /work/app\n\n<INSTRUCTIONS>agents secret</INSTRUCTIONS>",
            ]),
            reply("Done."),
            parts(&["Please wrap the output in <tag> like <b>this</b> ok"]),
            parts(&["<note>context</note> and then real text"]),
            reply("Wrapped."),
        ],
    );
    let history = wk_adapters::codex_history::read(d.path()).unwrap();
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(!all.contains("secret"), "injected context leaked: {all}");
    let c = find(&history, "codex:tags");
    assert_eq!(c.title, "Real prompt first line");
    let got: Vec<_> = c
        .messages
        .iter()
        .map(|m| (m.id.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("u1", "Real prompt first line\nmore"),
            ("a1", "Done."),
            (
                "u2",
                "Please wrap the output in <tag> like <b>this</b> ok\n\n<note>context</note> and then real text"
            ),
            ("a2", "Wrapped."),
        ]
    );
    assert_eq!(history.unreadable_records, 0);
    assert!(c.complete);
}
