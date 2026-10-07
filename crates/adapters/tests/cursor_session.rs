use serde_json::{json, Value};
use wk_adapters::cursor_session::{parse_transcript, MessageState, ScopeState};
use wk_scope::enrollment::Binding;
fn setup() -> (tempfile::TempDir, Binding) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("frontend")).unwrap();
    std::fs::write(root.path().join("frontend/a.ts"), "synthetic").unwrap();
    let b = Binding::approve(root.path(), "demo", vec!["frontend".into()]).unwrap();
    (root, b)
}
fn text(role: &str, value: &str) -> Value {
    json!({"role":role,"message":{"content":[{"type":"text","text":value}]}})
}
fn bytes(records: Vec<Value>) -> Vec<u8> {
    records
        .into_iter()
        .map(|v| v.to_string() + "\n")
        .collect::<String>()
        .into_bytes()
}
fn terminal() -> Value {
    json!({"type":"turn_ended","status":"success"})
}
#[test]
fn assistant_text_without_terminal_record_is_partial() {
    let (_d, b) = setup();
    let s = parse_transcript(
        &bytes(vec![text("user", "hello"), text("assistant", "world")]),
        &b,
        "frontend",
    )
    .unwrap();
    assert_eq!(s.turns[0].messages, MessageState::Partial);
}
#[test]
fn identical_requests_are_distinct_completed_turns() {
    let (_d, b) = setup();
    let mut records = vec![
        text("user", "hello"),
        text("assistant", "world"),
        terminal(),
    ];
    records.extend(records.clone());
    let s = parse_transcript(&bytes(records), &b, "frontend").unwrap();
    assert_eq!(s.turns.len(), 2);
    assert_ne!(s.turns[0].index, s.turns[1].index);
    assert_ne!(s.turns[0].digest, s.turns[1].digest);
    assert!(s
        .turns
        .iter()
        .all(|t| t.messages == MessageState::Complete && t.scope == ScopeState::Verified));
}
#[test]
fn partial_lines_wait_but_malformed_complete_records_hold() {
    let (_d, b) = setup();
    let mut data = bytes(vec![text("user", "hello"), text("assistant", "world")]);
    let boundary = data.len();
    data.extend(b"{bad");
    let s = parse_transcript(&data, &b, "frontend").unwrap();
    assert_eq!(s.consumed, boundary);
    assert!(s.held_reason.is_none());
    data.push(b'\n');
    assert!(parse_transcript(&data, &b, "frontend")
        .unwrap()
        .held_reason
        .is_some());
}
#[test]
fn tools_require_scope_and_uncertainty_contaminates_later_turns() {
    let (_d, b) = setup();
    for (name, path, want) in [
        ("Read", "frontend/a.ts", ScopeState::Verified),
        ("StrReplace", "frontend/a.ts", ScopeState::Verified),
        ("Read", "/outside/file", ScopeState::Violated),
        ("Shell", "frontend/a.ts", ScopeState::Unknown),
        ("MCP", "frontend/a.ts", ScopeState::Unknown),
    ] {
        let data = bytes(vec![
            text("user", "change"),
            json!({"role":"assistant","message":{"content":[{"type":"tool_use","name":name,"input":{"path":path}}]}}),
            text("assistant", "done"),
            terminal(),
            text("user", "next"),
            text("assistant", "fine"),
            terminal(),
        ]);
        let s = parse_transcript(&data, &b, "frontend").unwrap();
        assert!(s.turns.iter().all(|t| t.scope == want), "{name}");
        if want == ScopeState::Verified {
            assert!(s.turns[0].files_read.len() + s.turns[0].files_changed.len() == 1);
        }
    }
}
#[test]
fn unsupported_structures_and_attachments_hold() {
    let (_d, b) = setup();
    for extra in [
        json!({"type":"compaction"}),
        json!({"type":"subagent"}),
        json!({"role":"user","message":{"content":[{"type":"image","data":"private"}]}}),
        json!({"unknown":true}),
    ] {
        let s = parse_transcript(
            &bytes(vec![
                text("user", "hello"),
                extra,
                text("assistant", "world"),
                terminal(),
            ]),
            &b,
            "frontend",
        )
        .unwrap();
        assert!(s.held_reason.is_some());
    }
}
#[test]
fn failed_or_empty_turns_do_not_complete_and_reasoning_is_not_retained() {
    let (_d, b) = setup();
    for status in ["error", "cancelled"] {
        let s = parse_transcript(
            &bytes(vec![
                text("user", "hello"),
                text("assistant", "world"),
                json!({"type":"turn_ended","status":status}),
            ]),
            &b,
            "frontend",
        )
        .unwrap();
        assert_eq!(s.turns[0].messages, MessageState::Partial);
    }
    let s = parse_transcript(
        &bytes(vec![text("user", "hello"), terminal()]),
        &b,
        "frontend",
    )
    .unwrap();
    assert_eq!(s.turns[0].messages, MessageState::Partial);
    let s=parse_transcript(&bytes(vec![text("user","<system_reminder>injected</system_reminder><user_query>hello</user_query>"),json!({"role":"assistant","message":{"content":[{"type":"thinking","thinking":"REASONING-CANARY"},{"type":"text","text":"world"}]}}),terminal()]),&b,"frontend").unwrap();
    assert_eq!(s.turns[0].prompt, "hello");
    assert!(!serde_json::to_string(&s)
        .unwrap()
        .contains("REASONING-CANARY"));
}
#[test]
fn limits_hold_instead_of_truncating() {
    let (_d, b) = setup();
    assert!(parse_transcript(&vec![b' '; 4 * 1024 * 1024 + 1], &b, "frontend").is_err());
    let s = parse_transcript(
        &bytes(vec![text("user", &"a".repeat(256 * 1024))]),
        &b,
        "frontend",
    )
    .unwrap();
    assert!(s.held_reason.is_some());
}

#[test]
fn observed_record_layout_preserves_visible_decision_and_file_evidence() {
    let (_d, b) = setup();
    let s = parse_transcript(
        include_bytes!("fixtures/cursor-session-v3.jsonl"),
        &b,
        "frontend",
    )
    .unwrap();
    assert_eq!(s.turns.len(), 1);
    assert_eq!(s.turns[0].response, "Retries require idempotency.");
    assert_eq!(s.turns[0].files_read, vec!["frontend/a.ts"]);
    assert_eq!(s.turns[0].messages, MessageState::Complete);
}
