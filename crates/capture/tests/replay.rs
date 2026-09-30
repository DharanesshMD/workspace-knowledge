use serde_json::json;
fn setup() -> (tempfile::TempDir, wk_scope::enrollment::Binding) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("backend")).unwrap();
    let b = wk_scope::enrollment::Binding::approve(d.path(), "product", vec!["backend".into()])
        .unwrap();
    (d, b)
}
fn event(id: &str, agent: &str, kind: &str, path: &std::path::Path) -> serde_json::Value {
    json!({"schema_version":1,"event_id":id,"session_id":"s","agent_id":agent,"turn_id":"t","sequence":id.parse::<u32>().unwrap_or(0),"kind":kind,"path":path,"text":"PRIVATE_CANARY"})
}
fn jsonl(events: Vec<serde_json::Value>) -> String {
    events
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn separates_parallel_agents_and_deduplicates_events() {
    let (d, b) = setup();
    let p = d.path().join("backend");
    let a = event("1", "a", "user_prompt", &p);
    let events = jsonl(vec![
        a.clone(),
        event("2", "b", "user_prompt", &p),
        event("3", "a", "assistant_response", &p),
        event("4", "a", "turn_end", &p),
        a,
    ]);
    let result = wk_capture::inspect(&b, &events).unwrap();
    let turns = result["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["complete"], true);
    assert_eq!(turns[1]["complete"], false);
    assert!(!result.to_string().contains("PRIVATE_CANARY"));
}
#[test]
fn mixed_session_taints_later_turns() {
    let (d, b) = setup();
    let p = d.path().join("backend");
    let events = jsonl(vec![
        event("1", "a", "user_prompt", &p),
        event("2", "a", "assistant_response", &d.path().join("personal")),
        event("3", "a", "turn_end", &p),
    ]);
    let r = wk_capture::inspect(&b, &events).unwrap();
    assert_eq!(r["turns"][0]["scope"], "mixed");
    assert_eq!(r["turns"][0]["eligible"], false);
}
#[test]
fn missing_or_malformed_data_is_not_silently_complete() {
    let (_, b) = setup();
    assert!(wk_capture::inspect(&b, "not json").is_err());
    let mut e = event("1", "a", "hidden_reasoning", &b.root.join("backend"));
    assert!(wk_capture::inspect(&b, &e.to_string()).is_err());
    e["kind"] = json!("user_prompt");
    e["schema_version"] = json!(2);
    assert!(wk_capture::inspect(&b, &e.to_string()).is_err());
}
#[test]
fn conflicting_duplicate_fails_instead_of_overwriting() {
    let (d, b) = setup();
    let a = event("1", "a", "user_prompt", &d.path().join("backend"));
    let mut changed = a.clone();
    changed["text"] = json!("different");
    assert!(wk_capture::inspect(&b, &jsonl(vec![a, changed])).is_err());
}
