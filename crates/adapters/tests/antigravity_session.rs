use wk_adapters::{
    antigravity_session::{parse, reconcile},
    native_evidence::eligible,
};
const FIXTURE: &[u8] = include_bytes!("fixtures/native/antigravity-visible.jsonl");
#[test]
fn visible_turns_are_separate_and_terminal_counters_cannot_complete_them() {
    let snapshot = parse(FIXTURE).unwrap();
    assert_eq!(snapshot.turns.len(), 2);
    assert_eq!(snapshot.turns[0].turn, "0");
    assert_eq!(snapshot.turns[1].turn, "2");
    assert_eq!(snapshot.turns[0].response, "VIOLET-HOOK-ONE");
    assert_eq!(snapshot.turns[1].response, "VIOLET-HOOK-TWO");
    assert!(!snapshot.turns[0].prompt.contains("ADDITIONAL_METADATA"));
    let stop = serde_json::json!({"executionNum":0,"invocationNum":0,"fullyIdle":true,"terminationReason":"NO_TOOL_CALL"});
    let next = reconcile(Some(&snapshot), FIXTURE, Some(&stop)).unwrap();
    assert!(next.turns.iter().all(|t| !eligible(t)));
}
#[test]
fn partial_lines_and_prefix_mutations_never_complete() {
    let snapshot = parse(b"{\"step_index\":0").unwrap();
    assert_eq!(snapshot.consumed, 0);
    assert!(snapshot.turns.is_empty());
    let snapshot = parse(FIXTURE).unwrap();
    assert!(reconcile(Some(&snapshot), &FIXTURE[..FIXTURE.len() - 10], None).is_err());
    let mut changed = FIXTURE.to_vec();
    changed[10] = b'!';
    assert!(reconcile(Some(&snapshot), &changed, None).is_err());
}
#[test]
fn unknown_wrappers_records_and_oversized_input_are_rejected() {
    let mut rows: Vec<serde_json::Value> = std::str::from_utf8(FIXTURE)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    rows[0]["content"] = serde_json::json!(
        "<USER_REQUEST>\nhello\n</USER_REQUEST>\n<ATTACHMENT>SECRET</ATTACHMENT>"
    );
    assert!(parse(format!("{}\n", rows[0]).as_bytes()).is_err());
    rows[0]["type"] = serde_json::json!("THINKING");
    assert!(parse(format!("{}\n", rows[0]).as_bytes()).is_err());
    rows[0]["type"] = serde_json::json!("USER_INPUT");
    rows[0]["content"] = serde_json::json!("x".repeat(256 * 1024));
    assert!(parse(format!("{}\n", rows[0]).as_bytes()).is_err());
    assert!(parse(&vec![b'x'; 4 * 1024 * 1024 + 1]).is_err());
}
#[test]
fn duplicate_steps_or_orphan_response_are_rejected_and_reasoning_is_excluded() {
    let mut rows: Vec<serde_json::Value> = std::str::from_utf8(FIXTURE)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(parse(format!("{}\n", rows[1]).as_bytes()).is_err());
    rows[1]["step_index"] = serde_json::json!(0);
    assert!(parse(format!("{}\n{}\n", rows[0], rows[1]).as_bytes()).is_err());
    rows[1]["step_index"] = serde_json::json!(1);
    rows[1]["reasoning"] = serde_json::json!("SECRET-CANARY");
    assert!(parse(format!("{}\n{}\n", rows[0], rows[1]).as_bytes()).is_err());
}
