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
    assert_eq!(
        history.dropped_injected, 10,
        "nine injected parts in s1 and the environment context in s5"
    );

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
    assert_eq!(history.dropped_injected, 4);
    assert!(c.complete);
}

#[test]
fn codex_keeps_developer_xml_prompts_and_counts_dropped_context() {
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
    write_jsonl(
        &d.path().join("2026/10/15/xml.jsonl"),
        &[
            codex_meta(t, "xml", "/work/app"),
            parts(&["<task>Fix the limiter</task>"]),
            parts(&["<env_x>\nenv secret\n</env_x>"]),
            parts(&[
                "<environment_context>secret</environment_context>",
                "<context>Keep me</context>",
            ]),
            parts(&["<env_x attr=\"a>b\">attr secret</env_x>"]),
            parts(&["<env_x>Unclosed is typed"]),
            codex(
                t,
                "response_item",
                codex_msg("assistant", "output_text", "Done."),
            ),
        ],
    );
    let history = wk_adapters::codex_history::read(d.path()).unwrap();
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(!all.contains("secret"), "injected context leaked: {all}");
    let c = find(&history, "codex:xml");
    let got: Vec<_> = c
        .messages
        .iter()
        .map(|m| (m.id.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (
                "u1",
                "<task>Fix the limiter</task>\n\n<context>Keep me</context>\n\n<env_x>Unclosed is typed"
            ),
            ("a1", "Done."),
        ]
    );
    assert_eq!(c.title, "<task>Fix the limiter</task>");
    assert_eq!(history.dropped_injected, 3);
    assert_eq!(history.unreadable_records, 0);
}

fn write_json(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

/// A Copilot request whose prompt is `text` and whose reply is made of `response` parts.
fn copilot_request(id: &str, text: &str, response: Value) -> Value {
    json!({"requestId":id,"timestamp":1_792_058_401_000_i64,
           "message":{"text":text,"parts":[{"kind":"text","text":"parts secret"}]},
           "variableData":{"variables":[{"id":"file","name":"secret.rs","value":"variable secret"}]},
           "response":response,
           "result":{"metadata":{"toolCallRounds":"metadata secret"}}})
}

fn copilot_session(id: &str, created: i64, requests: Value) -> Value {
    json!({"version":3,"sessionId":id,"creationDate":created,"requests":requests})
}

/// `User/` with Copilot sessions in workspace storage folders of every kind.
fn copilot_fixture(root: &Path) -> PathBuf {
    let user = root.join("User");
    let storage = user.join("workspaceStorage");
    let app = storage.join("h1");
    write_json(
        &app.join("workspace.json"),
        &json!({"folder":"file:///work/my%20app"}),
    );
    let chats = app.join("chatSessions");
    write_json(
        &chats.join("s1.json"),
        &copilot_session(
            "s1",
            1_792_058_400_000,
            json!([
                copilot_request(
                    "r1",
                    LONG,
                    json!([
                        {"kind":"thinking","value":"thinking secret"},
                        {"value":"Reading the ","supportThemeIcons":false},
                        {"kind":"inlineReference","inlineReference":{"path":"/work/my app/secret.rs"},"name":"secret"},
                        {"value":{"value":"limiter."}},
                        {"kind":"prepareToolInvocation","toolName":"read secret"},
                        {"kind":"toolInvocationSerialized","invocationMessage":"tool secret","pastTenseMessage":"tool secret"},
                        {"kind":"textEditGroup","uri":{"path":"/secret"},"edits":[[{"text":"edit secret"}]]},
                        {"kind":"codeblockUri","uri":{"path":"/codeblock/secret"}},
                        {"kind":"undoStop","id":"undo secret"},
                        {"kind":"progressTaskSerialized","content":{"value":"progress secret"}},
                        {"kind":"mcpServersStarting","didStartServerIds":["mcp secret"]},
                        {"kind":"markdownContent","content":{"value":"Done: limiter fixed."}}
                    ]),
                ),
                copilot_request("r2", "Now add tests", json!([{"value":"Tests added."}])),
            ]),
        ),
    );
    // An empty session, and a session log whose snapshot is not a session.
    let mut empty = copilot_session("empty", 1_792_058_400_000, json!([]));
    empty["customTitle"] = json!("Empty secret");
    write_json(&chats.join("empty.json"), &empty);
    std::fs::write(
        chats.join("future.jsonl"),
        "{\"kind\":0,\"v\":\"jsonl secret\"}\n",
    )
    .unwrap();
    std::fs::write(chats.join("broken.json"), "{\"requests\":[").unwrap();
    std::fs::write(chats.join("notes.txt"), "not a session").unwrap();
    // Oversized (sparse) file is skipped without being read.
    let big = std::fs::File::create(chats.join("big.json")).unwrap();
    big.set_len(33 * 1024 * 1024).unwrap();

    // Custom title, an unknown part kind, and a failed request without reply text.
    let mut partial = copilot_session(
        "s2",
        1_792_058_300_000,
        json!([
            copilot_request(
                "r1",
                "Partial prompt",
                json!([{"kind":"someFutureKind","value":"future secret"},{"value":"Half."}]),
            ),
            copilot_request("r2", "Will fail", json!([])),
        ]),
    );
    partial["customTitle"] = json!("  Custom title\nsecond line");
    partial["requests"][1]["result"] = json!({"errorDetails":{"message":"error secret"}});
    write_json(&storage.join("h2/chatSessions/s2.json"), &partial);
    write_json(
        &storage.join("h2/workspace.json"),
        &json!({"folder":"vscode-remote://ssh-remote%2Bbox/work/my%20app"}),
    );

    // Multi-root workspace, and a storage folder without workspace.json.
    let one = |id: &str| {
        copilot_session(
            id,
            1_792_058_200_000,
            json!([copilot_request("r1", "Hi", json!([{"value":"Hello."}]))]),
        )
    };
    write_json(
        &storage.join("h3/workspace.json"),
        &json!({"workspace":"file:///work/my%20app/app.code-workspace"}),
    );
    write_json(&storage.join("h3/chatSessions/s3.json"), &one("s3"));
    write_json(&storage.join("h4/chatSessions/s4.json"), &one("s4"));

    // A sibling folder whose name shares a prefix; cancelled with partial reply text.
    let mut sibling = copilot_session(
        "s5",
        1_792_144_800_250,
        json!([copilot_request(
            "r1",
            "Sibling prompt",
            json!([{"value":"Sibling reply"}])
        )]),
    );
    sibling["requests"][0]["isCanceled"] = json!(true);
    write_json(&storage.join("h5/chatSessions/s5.json"), &sibling);
    write_json(
        &storage.join("h5/workspace.json"),
        &json!({"folder":"file:///work/my%20app2"}),
    );
    user
}

#[test]
fn copilot_reads_visible_prompts_and_replies_only() {
    let d = tempfile::tempdir().unwrap();
    let user = copilot_fixture(d.path());
    let history = wk_adapters::copilot_history::read(&user).unwrap();
    let ids: Vec<_> = history
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "copilot:s5",
            "copilot:s1",
            "copilot:s2",
            "copilot:s3",
            "copilot:s4"
        ],
        "newest first; empty, broken, oversized, .jsonl and other files are not sessions"
    );
    let c = find(&history, "copilot:s1");
    let got: Vec<_> = c
        .messages
        .iter()
        .map(|m| (m.id.as_str(), m.role.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("r1:u", "user", LONG),
            (
                "r1:a",
                "assistant",
                "Reading the limiter.\nDone: limiter fixed."
            ),
            ("r2:u", "user", "Now add tests"),
            ("r2:a", "assistant", "Tests added."),
        ]
    );
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(
        !all.contains("secret"),
        "hidden, tool, reference or variable content leaked: {all}"
    );
    assert_eq!(c.title, LONG.lines().next().unwrap()[..80]);
    assert_eq!(c.workspace.as_deref(), Some(Path::new("/work/my app")));
    assert_eq!(c.created_at_ms, Some(1_792_058_400_000));
    assert_eq!(c.source, "copilot");
    assert_eq!(c.outcome.as_deref(), Some("Tests added."));
    assert!(c.complete);
    assert!(c.files_changed.is_empty() && c.files_read.is_empty());

    let partial = find(&history, "copilot:s2");
    assert_eq!(partial.title, "Custom title");
    assert_eq!(partial.workspace, None, "remote folders are never selected");
    let got: Vec<_> = partial
        .messages
        .iter()
        .map(|m| (m.id.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("r1:u", "Partial prompt"),
            ("r1:a", "Half."),
            ("r2:u", "Will fail")
        ]
    );
    assert_eq!(partial.outcome.as_deref(), Some("Half."));
    assert!(!partial.complete, "unknown part and failed last request");
    assert_eq!(
        history.unreadable_records, 3,
        "unknown part kind, malformed file and bad log snapshot are counted, never guessed"
    );
    assert_eq!(history.empty_conversations, 1);
    assert_eq!(history.skipped, 1, "oversized file skipped");

    for id in ["copilot:s3", "copilot:s4"] {
        assert_eq!(find(&history, id).workspace, None, "{id}");
    }
    let sibling = find(&history, "copilot:s5");
    assert_eq!(
        sibling.workspace.as_deref(),
        Some(Path::new("/work/my app2"))
    );
    assert_eq!(sibling.created_at_ms, Some(1_792_144_800_250));
    assert!(
        sibling.complete,
        "a cancelled reply with visible text is kept"
    );
}

#[test]
fn copilot_sessions_scope_to_the_folder_not_its_prefix_siblings() {
    let d = tempfile::tempdir().unwrap();
    let history = wk_adapters::copilot_history::read(&copilot_fixture(d.path())).unwrap();
    let ids = |root: &str| -> Vec<String> {
        within(&history, Path::new(root))
            .into_iter()
            .map(|c| c.id)
            .collect()
    };
    assert_eq!(ids("/work/my app"), ["copilot:s1"]);
    assert_eq!(ids("/work/my app2"), ["copilot:s5"]);
}

#[test]
fn copilot_skips_sessions_with_too_many_messages() {
    let d = tempfile::tempdir().unwrap();
    let chats = d.path().join("workspaceStorage/h/chatSessions");
    let requests: Vec<Value> = (0..2501)
        .map(|i| copilot_request(&format!("r{i}"), "q", json!([{"value":"a"}])))
        .collect();
    write_json(
        &chats.join("huge.json"),
        &copilot_session("huge", 1, json!(requests)),
    );
    write_json(
        &chats.join("ok.json"),
        &copilot_session("ok", 1, json!(requests[..2500])),
    );
    let history = wk_adapters::copilot_history::read(d.path()).unwrap();
    let ids: Vec<_> = history
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["copilot:ok"]);
    assert_eq!(history.conversations[0].messages.len(), 5000);
    assert_eq!(history.skipped, 1);
}

#[cfg(unix)]
#[test]
fn copilot_never_follows_symlinks_out_of_workspace_storage() {
    let d = tempfile::tempdir().unwrap();
    let outside = d.path().join("outside");
    let leak = copilot_session(
        "leak",
        1,
        json!([copilot_request("r1", "outside secret", json!([]))]),
    );
    write_json(&outside.join("chatSessions/leak.json"), &leak);
    write_json(
        &outside.join("workspace.json"),
        &json!({"folder":"file:///work/app"}),
    );
    let storage = d.path().join("User/workspaceStorage");
    std::fs::create_dir_all(storage.join("h2")).unwrap();
    std::os::unix::fs::symlink(&outside, storage.join("linked")).unwrap();
    std::os::unix::fs::symlink(
        outside.join("chatSessions"),
        storage.join("h2/chatSessions"),
    )
    .unwrap();
    let real = storage.join("h1");
    std::fs::create_dir_all(real.join("chatSessions")).unwrap();
    std::os::unix::fs::symlink(
        outside.join("chatSessions/leak.json"),
        real.join("chatSessions/leak.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.join("workspace.json"), real.join("workspace.json"))
        .unwrap();
    write_json(
        &real.join("chatSessions/own.json"),
        &copilot_session(
            "own",
            1,
            json!([copilot_request("r1", "Mine", json!([{"value":"Ok."}]))]),
        ),
    );
    let history = wk_adapters::copilot_history::read(&d.path().join("User")).unwrap();
    let ids: Vec<_> = history
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["copilot:own"]);
    assert_eq!(
        history.conversations[0].workspace, None,
        "a symlinked workspace.json is never read"
    );
    assert_eq!(history.empty_conversations, 0);
}

#[test]
fn copilot_missing_user_dir_is_an_error() {
    let d = tempfile::tempdir().unwrap();
    assert!(wk_adapters::copilot_history::read(&d.path().join("absent")).is_err());
}

#[test]
fn copilot_keeps_tag_prompts_and_merges_unanswered_requests() {
    let d = tempfile::tempdir().unwrap();
    let request = |id: &str, text: &str, reply: &str| {
        let response = if reply.is_empty() {
            json!([])
        } else {
            json!([{ "value": reply }])
        };
        copilot_request(id, text, response)
    };
    let mut no_text = request("r5", "", "");
    no_text["message"] = json!({"parts":[{"text":"parts secret"}]});
    write_json(
        &d.path().join("workspaceStorage/h/chatSessions/tags.json"),
        &copilot_session(
            "tags",
            1,
            json!([
                request("r1", "<env_x>\ntyped context\n</env_x>", ""),
                request("r2", "  First real prompt\nmore  ", ""),
                request("r3", "Wrap it in <b>bold</b> ok", "Done."),
                request("r4", "Use <tag>x</tag> and then this", "Wrapped."),
                no_text,
                {"requestId":"r6","message":{"text":"No response yet"}},
            ]),
        ),
    );
    let history = wk_adapters::copilot_history::read(d.path()).unwrap();
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(!all.contains("secret"), "injected context leaked: {all}");
    let c = find(&history, "copilot:tags");
    assert_eq!(c.title, "<env_x>");
    let got: Vec<_> = c
        .messages
        .iter()
        .map(|m| (m.id.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (
                "r1:u",
                "<env_x>\ntyped context\n</env_x>\n\nFirst real prompt\nmore\n\nWrap it in <b>bold</b> ok"
            ),
            ("r3:a", "Done."),
            ("r4:u", "Use <tag>x</tag> and then this"),
            ("r4:a", "Wrapped."),
            ("r6:u", "No response yet"),
        ]
    );
    assert_eq!(
        history.unreadable_records, 1,
        "request without message text"
    );
    assert_eq!(
        history.dropped_injected, 0,
        "Copilot keeps injected context out of message.text"
    );
    assert!(!c.complete);
    assert_eq!(c.outcome.as_deref(), Some("Wrapped."));
    assert_eq!(c.workspace, None);
}

fn write_lines(path: &Path, lines: &[String]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

#[test]
fn copilot_replays_session_logs_and_never_reads_input_state() {
    let d = tempfile::tempdir().unwrap();
    let chats = d.path().join("workspaceStorage/h/chatSessions");
    let mut snapshot = copilot_session(
        "log",
        1_792_058_400_000,
        json!([copilot_request(
            "r1",
            "First prompt",
            json!([
                {"kind":"notebookEditGroup","uri":{"path":"/nb/secret"},"edits":[["notebook secret"]]},
                {"kind":"confirmation","title":"confirm secret","message":"confirm secret"},
                {"kind":"progressMessage","content":{"value":"progress secret"}},
                {"kind":"elicitation","title":"elicit secret"},
                {"kind":"elicitationSerialized","title":"elicit secret"},
                {"kind":"command","command":{"title":"command secret"}},
                {"value":"First reply"}
            ]),
        )]),
    );
    snapshot["inputState"] = json!({"inputText":"draft secret","attachments":["attach secret"]});
    let second = copilot_request("r2", "Second prompt", json!([]));
    let ops = [
        json!({"kind":0,"v":snapshot}),
        json!({"kind":2,"k":["requests"],"v":[second]}),
        json!({"kind":2,"k":["requests",1,"response"],"v":[{"value":"Streaming draft"}]}),
        // Streaming replaces the response from index 0.
        json!({"kind":2,"k":["requests",1,"response"],"i":0,
               "v":[{"value":"Final "},{"kind":"thinking","value":"thinking secret"}]}),
        json!({"kind":2,"k":["requests",1,"response"],"v":[
            {"kind":"markdownVuln","content":{"value":"answer."},
             "vulnerabilities":[{"title":"vuln secret","description":"vuln secret"}]}
        ]}),
        json!({"kind":2,"k":["requests",1,"response"],"v":null}),
        json!({"kind":1,"k":["requests",1,"result"],"v":{"metadata":{"x":"metadata secret"}}}),
        json!({"kind":1,"k":["inputState","inputText"],"v":"draft secret two"}),
        json!({"kind":1,"k":["customTitle"],"v":"Logged title"}),
    ];
    let lines: Vec<String> = ops.iter().map(Value::to_string).collect();
    write_lines(&chats.join("log.jsonl"), &lines);
    // A session stored both ways is read once, from the log.
    write_json(
        &chats.join("log.json"),
        &copilot_session(
            "log",
            1,
            json!([copilot_request("r1", "stale secret", json!([]))]),
        ),
    );
    let history = wk_adapters::copilot_history::read(d.path()).unwrap();
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(
        !all.contains("secret"),
        "hidden or draft content leaked: {all}"
    );
    assert!(!all.contains("Streaming draft"), "spliced parts must go");
    assert_eq!(history.conversations.len(), 1);
    let c = find(&history, "copilot:log");
    let got: Vec<_> = c
        .messages
        .iter()
        .map(|m| (m.id.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("r1:u", "First prompt"),
            ("r1:a", "First reply"),
            ("r2:u", "Second prompt"),
            ("r2:a", "Final answer."),
        ]
    );
    assert_eq!(c.title, "Logged title");
    assert_eq!(c.created_at_ms, Some(1_792_058_400_000));
    assert_eq!(c.outcome.as_deref(), Some("Final answer."));
    assert!(c.complete);
    assert_eq!(
        history.unreadable_records, 0,
        "known kinds are not unreadable"
    );
    assert_eq!(history.skipped, 0);
}

#[test]
fn copilot_counts_bad_log_lines_and_bounds_logs() {
    let d = tempfile::tempdir().unwrap();
    let chats = d.path().join("workspaceStorage/h/chatSessions");
    let snapshot = copilot_session(
        "bad",
        1,
        json!([copilot_request("r1", "Prompt", json!([{"value":"Reply"}]))]),
    );
    let mut lines: Vec<String> = [
        json!({"kind":0,"v":snapshot}),
        json!({"kind":9,"k":["requests"],"v":[]}),
        json!({"kind":1,"k":["requests",5,"result"],"v":{}}),
        json!({"kind":1,"k":["requests","x"],"v":{}}),
        json!({"kind":2,"k":["sessionId"],"v":[1]}),
        json!({"kind":2,"k":["requests"],"v":[],"i":99}),
        json!({"kind":2,"k":["requests"],"v":"not a list"}),
    ]
    .iter()
    .map(Value::to_string)
    .collect();
    lines.insert(1, "{not json".into());
    lines.push(json!({"kind":2,"k":["requests"],"v":[copilot_request("r2", "Later", json!([{"value":"Later reply"}]))]}).to_string());
    write_lines(&chats.join("bad.jsonl"), &lines);
    // A log must start with a snapshot.
    write_lines(
        &chats.join("headless.jsonl"),
        &[json!({"kind":1,"k":["customTitle"],"v":"headless secret"}).to_string()],
    );
    // Logs over 200,000 lines are skipped.
    let op = json!({"kind":1,"k":["customTitle"],"v":"t"}).to_string();
    let mut long = vec![json!({"kind":0,"v":copilot_session("long", 1, json!([]))}).to_string()];
    long.extend(std::iter::repeat_n(op, 200_000));
    write_lines(&chats.join("long.jsonl"), &long);
    let history = wk_adapters::copilot_history::read(d.path()).unwrap();
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(!all.contains("secret"), "{all}");
    let ids: Vec<_> = history
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["copilot:bad"]);
    let c = &history.conversations[0];
    let got: Vec<_> = c.messages.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(got, ["Prompt", "Reply", "Later", "Later reply"]);
    assert!(!c.complete, "a log with bad lines is never complete");
    assert_eq!(
        history.unreadable_records, 8,
        "malformed line, unknown kind, four unresolvable ops, bad push value, headless log"
    );
    assert_eq!(history.skipped, 1);
}
