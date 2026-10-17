//! Claude Code reader: only typed prompts and real replies are imported. Synthetic fixtures
//! only; anything that must never reach the output contains the sentinel "secret".
use serde_json::{json, Value};
use std::path::Path;
use wk_adapters::claude_history::read;
use wk_adapters::cursor_history::History;

fn record(kind: &str, uuid: &str, ts: &str, content: Value) -> Value {
    json!({"type":kind,"sessionId":"s","uuid":uuid,"timestamp":ts,"cwd":"/work/app",
           "isSidechain":false,"message":{"role":kind,"content":content}})
}

fn user(uuid: &str, content: Value) -> Value {
    record("user", uuid, "2026-10-15T10:00:00Z", content)
}

fn reply(uuid: &str, text: &str) -> Value {
    record(
        "assistant",
        uuid,
        "2026-10-15T10:00:00Z",
        json!([{"type":"text","text":text}]),
    )
}

fn write_lines(path: &Path, lines: &[Vec<u8>], separator: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, lines.join(separator)).unwrap();
}

fn bytes(records: &[Value]) -> Vec<Vec<u8>> {
    records.iter().map(|r| r.to_string().into_bytes()).collect()
}

fn messages(history: &History, id: &str) -> Vec<(String, String, String)> {
    history
        .conversations
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("missing {id}"))
        .messages
        .iter()
        .map(|m| (m.id.clone(), m.role.clone(), m.text.clone()))
        .collect()
}

fn owned(rows: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    rows.iter()
        .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
        .collect()
}

#[test]
fn imports_only_typed_prompts_and_real_replies() {
    let d = tempfile::tempdir().unwrap();
    let mut compact = user(
        "compact",
        json!("This session is being continued from a previous conversation. secret summary"),
    );
    compact["isCompactSummary"] = json!(true);
    let mut transcript_only = user("transcript-only", json!("secret transcript-only"));
    transcript_only["isVisibleInTranscriptOnly"] = json!(true);
    let mut api_error = reply("api-error", "API Error: secret overloaded");
    api_error["isApiErrorMessage"] = json!(true);
    api_error["message"]["model"] = json!("<synthetic>");
    let mut synthetic = reply("synthetic", "No response requested. secret");
    synthetic["message"]["model"] = json!("<synthetic>");
    let records = [
        compact,
        transcript_only,
        user("bash-in", json!("<bash-input>echo secret</bash-input>")),
        user(
            "bash-out",
            json!("<bash-stdout>API_KEY=sk-secret-1</bash-stdout><bash-stderr></bash-stderr>"),
        ),
        user("bash-truncated", json!("<bash-stdout>API_KEY=sk-secret-2 trunc")),
        user(
            "task",
            json!("<task-notification>\n<task-id>t1</task-id>\n<summary>secret done</summary>\n</task-notification>"),
        ),
        user(
            "reminder",
            json!([{"type":"text","text":"<system-reminder>secret reminder</system-reminder>"}]),
        ),
        user(
            "u1",
            json!([
                {"type":"text","text":"<ide_opened_file>The user opened secret.rs</ide_opened_file>"},
                {"type":"text","text":"<ide_selection attr=\"x\">secret selection</ide_selection>\n<user-memory-input>secret</user-memory-input>"},
                {"type":"text","text":"Real prompt one"},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":"secret"}}
            ]),
        ),
        reply("a1", "Reply one"),
        user("int1", json!("[Request interrupted by user]")),
        user(
            "int2",
            json!([{"type":"text","text":"[Request interrupted by user for tool use]"}]),
        ),
        user(
            "mixed",
            json!([
                {"type":"tool_result","tool_use_id":"t","content":"secret tool output"},
                {"type":"text","text":"secret alongside tool result"}
            ]),
        ),
        user("u2", json!("Real prompt two <b>with</b> a tag inside")),
        api_error,
        synthetic,
    ];
    // CRLF line endings throughout.
    write_lines(
        &d.path().join("-work-app/s.jsonl"),
        &bytes(&records),
        b"\r\n",
    );
    let history = read(d.path()).unwrap();
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(!all.contains("secret"), "non-prompt content leaked: {all}");
    assert_eq!(
        messages(&history, "claude-code:s"),
        owned(&[
            ("u1", "user", "Real prompt one"),
            ("a1", "assistant", "Reply one"),
            ("u2", "user", "Real prompt two <b>with</b> a tag inside"),
        ])
    );
    let c = &history.conversations[0];
    assert_eq!(c.title, "Real prompt one");
    assert_eq!(c.outcome.as_deref(), Some("Reply one"));
    assert!(!c.complete, "an API error is not a reply");
    assert_eq!(history.unreadable_records, 0, "skipped records are known");
    assert_eq!(
        history.dropped_injected, 9,
        "bash, task, reminder, two IDE parts and two interrupt markers"
    );
}

#[test]
fn invalid_utf8_line_is_unreadable_and_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let mut lines = bytes(&[user("u1", json!("Prompt"))]);
    lines.push(vec![b'{', 0xff, 0xfe, b'}']);
    lines.extend(bytes(&[reply("a1", "Reply")]));
    write_lines(&d.path().join("-work-app/s.jsonl"), &lines, b"\n");
    let history = read(d.path()).unwrap();
    assert_eq!(
        messages(&history, "claude-code:s"),
        owned(&[("u1", "user", "Prompt"), ("a1", "assistant", "Reply")])
    );
    assert_eq!(history.unreadable_records, 1);
    assert!(!history.conversations[0].complete);
}

#[test]
fn timestamps_follow_rfc3339() {
    let d = tempfile::tempdir().unwrap();
    let cases = [
        ("half", "2026-10-15T10:00:00.5Z", Some(1_792_058_400_500)),
        (
            "negative",
            "2026-10-15T08:30:00-01:30",
            Some(1_792_058_400_000),
        ),
        ("lower", "2026-10-15t10:00:00z", Some(1_792_058_400_000)),
        ("leap", "2024-02-29T01:00:00Z", Some(1_709_168_400_000)),
        ("feb31", "2026-02-31T10:00:00Z", None),
        ("feb29", "2025-02-29T10:00:00Z", None),
        ("apr31", "2026-04-31T10:00:00Z", None),
    ];
    for (name, ts, _) in cases {
        write_lines(
            &d.path().join(format!("-work-app/{name}.jsonl")),
            &bytes(&[
                record("user", "u", ts, json!("Prompt")),
                record(
                    "assistant",
                    "a",
                    ts,
                    json!([{"type":"text","text":"Reply"}]),
                ),
            ]),
            b"\n",
        );
    }
    let history = read(d.path()).unwrap();
    for (name, ts, want) in cases {
        let c = history
            .conversations
            .iter()
            .find(|c| c.id == format!("claude-code:{name}"))
            .unwrap();
        assert_eq!(c.created_at_ms, want, "{ts}");
    }
}

#[cfg(unix)]
#[test]
fn unreadable_project_folder_is_counted() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let locked = d.path().join("-work-locked");
    write_lines(
        &locked.join("s.jsonl"),
        &bytes(&[user("u", json!("secret"))]),
        b"\n",
    );
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let denied = std::fs::read_dir(&locked).is_err();
    let history = read(d.path());
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    let history = history.unwrap();
    assert!(history.conversations.is_empty() == denied);
    if denied {
        assert_eq!(history.unreadable_records, 1);
    }
}

#[test]
fn file_at_the_size_limit_is_read_and_one_byte_over_is_skipped() {
    let d = tempfile::tempdir().unwrap();
    let limit = 32 * 1024 * 1024;
    let mut body = bytes(&[user("u", json!("Prompt")), reply("a", "Reply")]).join(&b'\n');
    body.push(b'\n');
    let pad = limit - body.len();
    let mut exact = body.clone();
    exact.resize(limit, b' ');
    assert_eq!(exact.len(), limit, "pad {pad}");
    std::fs::create_dir_all(d.path().join("-work-app")).unwrap();
    std::fs::write(d.path().join("-work-app/exact.jsonl"), &exact).unwrap();
    exact.push(b' ');
    std::fs::write(d.path().join("-work-app/over.jsonl"), &exact).unwrap();
    let history = read(d.path()).unwrap();
    let ids: Vec<_> = history
        .conversations
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, ["claude-code:exact"]);
    assert_eq!(history.skipped, 1);
}

#[test]
fn known_injected_blocks_are_dropped_even_with_trailing_text() {
    let d = tempfile::tempdir().unwrap();
    let records = [
        user(
            "stdout",
            json!("<local-command-stdout>API_KEY=sk-secret-3</local-command-stdout>\ntrailing secret"),
        ),
        user(
            "task",
            json!("<task-notification><summary>secret</summary></task-notification>\ntrailing secret"),
        ),
        user(
            "nested",
            json!("<system-reminder><system-reminder>inner secret</system-reminder> outer secret</system-reminder>"),
        ),
        user(
            "parts",
            json!([
                {"type":"text","text":"<command-name>/review</command-name> secret args"},
                {"type":"text","text":"<ide_opened_file>secret.rs</ide_opened_file> then secret"},
                {"type":"text","text":"<user-prompt-submit-hook>secret hook</user-prompt-submit-hook> more"},
                {"type":"text","text":"<user-memory-input>secret memory</user-memory-input> more"},
                {"type":"text","text":"<bash-notification>secret</bash-notification> more"}
            ]),
        ),
        user("u1", json!("Real prompt mentioning <system-reminder> later")),
        reply("a1", "Reply"),
    ];
    write_lines(&d.path().join("-work-app/s.jsonl"), &bytes(&records), b"\n");
    let history = read(d.path()).unwrap();
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(!all.contains("secret"), "injected content leaked: {all}");
    assert_eq!(
        messages(&history, "claude-code:s"),
        owned(&[
            (
                "u1",
                "user",
                "Real prompt mentioning <system-reminder> later"
            ),
            ("a1", "assistant", "Reply"),
        ])
    );
    assert_eq!(history.unreadable_records, 0);
}

#[test]
fn developer_xml_prompts_are_kept_and_injected_parts_counted() {
    let d = tempfile::tempdir().unwrap();
    let records = [
        user("u1", json!("<task>Fix the limiter</task>")),
        user("env", json!("<env_x>\nsecret env\n</env_x>")),
        user(
            "parts",
            json!([
                {"type":"text","text":"<ide_opened_file>secret.rs</ide_opened_file>"},
                {"type":"text","text":"<context>Keep this</context>"}
            ]),
        ),
        user(
            "mixed",
            json!("<env_x attr=\"a>b\">secret</env_x>\n<task>secret mixed</task>"),
        ),
        user("unclosed", json!("<env_x>Unclosed is typed")),
        reply("a1", "Reply"),
    ];
    write_lines(&d.path().join("-work-app/s.jsonl"), &bytes(&records), b"\n");
    let history = read(d.path()).unwrap();
    let all = serde_json::to_string(&history.conversations).unwrap();
    assert!(!all.contains("secret"), "injected content leaked: {all}");
    assert_eq!(
        messages(&history, "claude-code:s"),
        owned(&[
            (
                "u1",
                "user",
                "<task>Fix the limiter</task>\n\n<context>Keep this</context>\n\n<env_x>Unclosed is typed"
            ),
            ("a1", "assistant", "Reply"),
        ])
    );
    assert_eq!(
        history.conversations[0].title,
        "<task>Fix the limiter</task>"
    );
    assert_eq!(
        history.dropped_injected, 3,
        "tool-style tag blocks (alone or with plain tags) and the IDE part"
    );
    assert_eq!(history.unreadable_records, 0);
}

/// `claude-code:h-` followed by 16 lowercase hex digits: the hashed fallback session id.
fn is_hashed(id: &str) -> bool {
    id.strip_prefix("claude-code:h-")
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[test]
fn session_file_stems_that_are_not_safe_ids_get_a_path_hash() {
    let d = tempfile::tempdir().unwrap();
    let records = bytes(&[user("u1", json!("Prompt")), reply("a1", "Reply")]);
    // The same bad name in two project folders must not collide.
    write_lines(&d.path().join("-work-a/bad name.jsonl"), &records, b"\n");
    write_lines(&d.path().join("-work-b/bad name.jsonl"), &records, b"\n");
    let history = read(d.path()).unwrap();
    let ids: Vec<_> = history.conversations.iter().map(|c| &c.id).collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert!(ids.iter().all(|id| is_hashed(id)), "{ids:?}");
    assert_ne!(ids[0], ids[1]);
    assert_eq!(history.unreadable_records, 0);
}

fn set_mtime(path: &Path, secs_ago: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn a_session_id_found_in_two_project_folders_keeps_the_newest_file() {
    let d = tempfile::tempdir().unwrap();
    let write = |folder: &str, id: &str, prompt: &str, secs_ago: u64| {
        let path = d.path().join(folder).join(format!("{id}.jsonl"));
        let records = bytes(&[user("u1", json!(prompt)), reply("a1", "Reply")]);
        write_lines(&path, &records, b"\n");
        set_mtime(&path, secs_ago);
    };
    write("-work-a", "first", "Newest first", 10);
    write("-work-b", "first", "Older first", 1_000);
    write("-work-a", "second", "Older second", 1_000);
    write("-work-b", "second", "Newest second", 10);
    let history = read(d.path()).unwrap();
    let mut got: Vec<_> = history
        .conversations
        .iter()
        .map(|c| (c.id.as_str(), c.messages[0].text.as_str()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            ("claude-code:first", "Newest first"),
            ("claude-code:second", "Newest second")
        ]
    );
    assert_eq!(history.unreadable_records, 2, "one dropped duplicate each");
}

#[test]
fn only_the_first_absolute_cwd_without_dot_components_is_a_workspace() {
    let d = tempfile::tempdir().unwrap();
    let with_cwds = |name: &str, cwds: [&str; 2]| {
        let mut prompt = user("u1", json!("Prompt"));
        prompt["cwd"] = json!(cwds[0]);
        let mut answer = reply("a1", "Reply");
        answer["cwd"] = json!(cwds[1]);
        write_lines(
            &d.path().join(format!("-work-app/{name}.jsonl")),
            &bytes(&[prompt, answer]),
            b"\n",
        );
    };
    with_cwds("relative", ["relative/app", "relative/other"]);
    with_cwds("parent", ["/work/../etc", "/work/./app"]);
    with_cwds("curdir", ["/work/./app", "/work/app/.."]);
    with_cwds("later", ["relative/app", "/work/app"]);
    let history = read(d.path()).unwrap();
    let workspace = |id: &str| {
        history
            .conversations
            .iter()
            .find(|c| c.id == format!("claude-code:{id}"))
            .unwrap()
            .workspace
            .clone()
    };
    assert_eq!(workspace("relative"), None);
    assert_eq!(workspace("parent"), None);
    assert_eq!(workspace("curdir"), None);
    assert_eq!(workspace("later"), Some(Path::new("/work/app").into()));
}

#[test]
fn aborted_sessions_add_nothing_to_the_counters() {
    let d = tempfile::tempdir().unwrap();
    let mut lines = vec![b"{not json".to_vec()];
    lines.extend(bytes(&[user(
        "inj",
        json!("<ide_opened_file>x.rs</ide_opened_file>"),
    )]));
    // 5,001 alternating messages exceed the per-session limit.
    lines.extend((0..5001).map(|i| {
        if i % 2 == 0 {
            user(&format!("u{i}"), json!("q")).to_string().into_bytes()
        } else {
            reply(&format!("a{i}"), "a").to_string().into_bytes()
        }
    }));
    write_lines(&d.path().join("-work-app/huge.jsonl"), &lines, b"\n");
    let history = read(d.path()).unwrap();
    assert!(history.conversations.is_empty());
    assert_eq!(history.skipped, 1);
    assert_eq!(
        history.unreadable_records, 0,
        "the malformed line was not kept"
    );
    assert_eq!(
        history.dropped_injected, 0,
        "the injected part was not kept"
    );
}

#[test]
fn titles_keep_up_to_four_hundred_characters_of_the_first_line() {
    let d = tempfile::tempdir().unwrap();
    let prompt = format!("{}\nsecond line", "t".repeat(500));
    write_lines(
        &d.path().join("-work-app/s.jsonl"),
        &bytes(&[user("u1", json!(prompt)), reply("a1", "Reply")]),
        b"\n",
    );
    let history = read(d.path()).unwrap();
    assert_eq!(history.conversations[0].title, "t".repeat(400));
}
