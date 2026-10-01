use serde_json::{json, Value};
use wk_capture::journal::ingest;
use wk_local::LocalStore;
use wk_scope::enrollment::Binding;
fn event(_b: &Binding, id: u32, path: &std::path::Path) -> String {
    json!({"schema_version":1,"event_id":id.to_string(),"session_id":"synthetic-session","agent_id":"a","turn_id":"t","sequence":id,"kind":if id==1 {"user_prompt"} else {"assistant_response"},"path":path,"text":"PRIVATE_CANARY"}).to_string()
}
fn setup() -> (tempfile::TempDir, Binding, LocalStore) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("product/backend")).unwrap();
    let b = Binding::approve(&d.path().join("product"), "demo", vec!["backend".into()]).unwrap();
    let s = LocalStore::open(&d.path().join("queue.db"), [9; 32], 1024 * 1024).unwrap();
    (d, b, s)
}
#[test]
fn reconnect_deduplicates_without_plaintext_or_content_in_receipt() {
    let (d, b, mut s) = setup();
    let e = event(&b, 1, &b.root.join("backend"));
    let first = ingest(&mut s, &b, &e, 100).unwrap();
    assert_eq!(first["event_count"], 1);
    assert!(!first.to_string().contains("PRIVATE_CANARY"));
    drop(s);
    let mut s = LocalStore::open(&d.path().join("queue.db"), [9; 32], 1024 * 1024).unwrap();
    assert_eq!(ingest(&mut s, &b, &e, 101).unwrap()["event_count"], 1);
    assert_eq!(
        ingest(&mut s, &b, &event(&b, 2, &b.root.join("backend")), 102).unwrap()["event_count"],
        2
    );
    let bytes = std::fs::read(d.path().join("queue.db")).unwrap();
    for canary in [b"PRIVATE_CANARY".as_slice(), b"synthetic-session"] {
        assert!(!bytes.windows(canary.len()).any(|w| w == canary));
    }
}
#[test]
fn outside_context_erases_session_payload_and_hold_survives_restart() {
    let (d, b, mut s) = setup();
    ingest(&mut s, &b, &event(&b, 1, &b.root.join("backend")), 100).unwrap();
    let receipt = ingest(&mut s, &b, &event(&b, 2, &d.path().join("personal")), 101).unwrap();
    assert_eq!(receipt["held"], true);
    let plain = String::from_utf8(s.pending(101).unwrap()[0].1.clone()).unwrap();
    assert!(!plain.contains("PRIVATE_CANARY"));
    assert!(!plain.contains("personal"));
    drop(s);
    let mut s = LocalStore::open(&d.path().join("queue.db"), [9; 32], 1024 * 1024).unwrap();
    let r = ingest(&mut s, &b, &event(&b, 3, &b.root.join("backend")), 102).unwrap();
    assert_eq!(r["held"], true);
    assert_eq!(r["event_count"], 0);
}
#[test]
fn conflict_and_pause_hold_previously_stored_session() {
    for pause in [false, true] {
        let (_d, mut b, mut s) = setup();
        let e = event(&b, 1, &b.root.join("backend"));
        ingest(&mut s, &b, &e, 100).unwrap();
        if pause {
            b.pause();
        }
        let e = e.replace("PRIVATE_CANARY", "changed");
        assert_eq!(ingest(&mut s, &b, &e, 101).unwrap()["held"], true);
        assert!(!String::from_utf8(s.pending(101).unwrap()[0].1.clone())
            .unwrap()
            .contains("changed"));
    }
}
#[test]
fn malformed_input_never_creates_a_record() {
    let (_d, b, mut s) = setup();
    let mut e: Value = serde_json::from_str(&event(&b, 1, &b.root.join("backend"))).unwrap();
    e["kind"] = json!("hidden_reasoning");
    assert!(ingest(&mut s, &b, &e.to_string(), 100).is_err());
    assert!(s.pending(100).unwrap().is_empty());
}

#[test]
fn child_agent_gap_holds_entire_session_but_not_another_session() {
    let (_d, b, mut s) = setup();
    let first = event(&b, 1, &b.root.join("backend"));
    ingest(&mut s, &b, &first, 100).unwrap();
    let mut gap: Value = serde_json::from_str(&first).unwrap();
    gap["agent_id"] = json!("child");
    gap["kind"] = json!("capture_gap");
    assert_eq!(
        ingest(&mut s, &b, &gap.to_string(), 101).unwrap()["held"],
        true
    );
    assert_eq!(ingest(&mut s, &b, &first, 102).unwrap()["held"], true);
    let other = first.replace("synthetic-session", "another-session");
    assert_eq!(ingest(&mut s, &b, &other, 103).unwrap()["held"], false);
}

#[test]
fn policy_change_and_session_growth_cannot_retain_partial_payload() {
    let (_d, mut b, mut s) = setup();
    let first = event(&b, 1, &b.root.join("backend"));
    ingest(&mut s, &b, &first, 100).unwrap();
    b.policy_hash = "changed".into();
    assert_eq!(ingest(&mut s, &b, &first, 101).unwrap()["held"], true);

    let (_d, b, mut s) = setup();
    for id in 1..=4 {
        let mut e: Value = serde_json::from_str(&event(&b, id, &b.root.join("backend"))).unwrap();
        e["text"] = json!("x".repeat(240 * 1024));
        let receipt = ingest(&mut s, &b, &e.to_string(), 100 + id as i64).unwrap();
        assert_eq!(receipt["held"], id == 4);
    }
    let rows = s.pending(105).unwrap();
    let state: Value = serde_json::from_slice(&rows[0].1).unwrap();
    assert_eq!(state["events"], json!([]));
}

#[test]
fn expiry_and_restart_do_not_reopen_a_contaminated_session() {
    let (d, b, mut s) = setup();
    ingest(&mut s, &b, &event(&b, 1, &b.root.join("backend")), 100).unwrap();
    ingest(&mut s, &b, &event(&b, 2, &d.path().join("personal")), 101).unwrap();
    assert!(s.pending(86500).unwrap().is_empty());
    drop(s);
    let mut s = LocalStore::open(&d.path().join("queue.db"), [9; 32], 1024 * 1024).unwrap();
    assert!(ingest(&mut s, &b, &event(&b, 3, &b.root.join("backend")), 86501).is_err());
    assert!(s.pending(86501).unwrap().is_empty());
}
