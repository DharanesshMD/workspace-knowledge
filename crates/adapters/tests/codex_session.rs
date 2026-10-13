use serde_json::{json, Value};
use wk_adapters::{codex_session::apply, native_evidence::*};
fn prompt(root: &std::path::Path) -> Value {
    json!({"session_id":"session", "turn_id":"one", "cwd":root,
        "hook_event_name":"UserPromptSubmit", "prompt":"hello"})
}
#[test]
fn consistent_no_tool_turn_completes_and_exact_replay_is_idempotent() {
    let root = std::env::temp_dir().canonicalize().unwrap();
    let p = prompt(&root);
    let first = apply(None, &p, &root).unwrap();
    assert!(!eligible(&first));
    let stop = json!({"session_id":"session","turn_id":"one","cwd":root,
        "hook_event_name":"Stop","last_assistant_message":"ok","stop_hook_active":false});
    let final_turn = apply(Some(&first), &stop, &root).unwrap();
    assert!(eligible(&final_turn));
    assert_eq!(
        apply(Some(&final_turn), &stop, &root).unwrap().digest,
        final_turn.digest
    );
    assert_eq!(
        apply(Some(&final_turn), &p, &root).unwrap().digest,
        final_turn.digest
    );
}
#[test]
fn interruption_and_conflict_cannot_be_repaired_by_a_stop() {
    let root = std::env::temp_dir().canonicalize().unwrap();
    let p = prompt(&root);
    let first = apply(None, &p, &root).unwrap();
    let stop = json!({"session_id":"session","turn_id":"one","cwd":root,
        "hook_event_name":"Stop","last_assistant_message":"ok","stop_hook_active":false});
    for event in [
        "Interrupt",
        "PreCompact",
        "SubagentStart",
        "PreToolUse",
        "PostToolUse",
        "unknown",
    ] {
        let mut changed = p.clone();
        changed["hook_event_name"] = json!(event);
        let held = apply(Some(&first), &changed, &root).unwrap();
        let restored: Evidence =
            serde_json::from_str(&serde_json::to_string(&held).unwrap()).unwrap();
        assert!(
            !eligible(&apply(Some(&restored), &stop, &root).unwrap()),
            "{event}"
        );
    }
    let mut conflict = p;
    conflict["prompt"] = json!("different");
    let held = apply(Some(&first), &conflict, &root).unwrap();
    assert!(!eligible(&apply(Some(&held), &stop, &root).unwrap()));
    assert!(!eligible(&apply(None, &stop, &root).unwrap()));
}
#[test]
fn foreign_identity_scope_and_unknown_context_are_not_accepted() {
    let root = std::env::temp_dir().canonicalize().unwrap();
    let p = prompt(&root);
    let first = apply(None, &p, &root).unwrap();
    for (key, value) in [
        ("cwd", json!("/")),
        ("turn_id", json!("two")),
        ("session_id", json!("other")),
        ("agent_id", json!("child")),
    ] {
        let mut bad = p.clone();
        bad[key] = value;
        assert!(apply(Some(&first), &bad, &root).is_err());
    }
    for key in ["attachments", "external_context", "reasoning", "thinking"] {
        let mut bad = p.clone();
        bad[key] = json!("SECRET-CANARY");
        let held = apply(None, &bad, &root).unwrap();
        assert!(!eligible(&held));
        assert!(!serde_json::to_string(&held)
            .unwrap()
            .contains("SECRET-CANARY"));
    }
    let mut bad = p.clone();
    bad.as_object_mut().unwrap().remove("turn_id");
    assert!(apply(None, &bad, &root).is_err());
    let mut huge = p;
    huge["prompt"] = json!("a".repeat(256 * 1024));
    assert!(apply(None, &huge, &root).is_err());
}
#[test]
fn changed_final_response_is_held_and_sequential_turns_stay_distinct() {
    let root = std::env::temp_dir().canonicalize().unwrap();
    let p = prompt(&root);
    let first = apply(None, &p, &root).unwrap();
    let mut stop = json!({"session_id":"session","turn_id":"one","cwd":root,"hook_event_name":"Stop","last_assistant_message":"ok","stop_hook_active":false});
    let done = apply(Some(&first), &stop, &root).unwrap();
    stop["last_assistant_message"] = json!("changed");
    assert!(!eligible(&apply(Some(&done), &stop, &root).unwrap()));
    let mut second = p;
    second["turn_id"] = json!("two");
    let next = apply(None, &second, &root).unwrap();
    assert_ne!(first.turn, next.turn);
    assert_ne!(first.digest, next.digest);
}
