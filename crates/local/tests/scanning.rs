use wk_local::LocalStore;

#[test]
fn session_rotation_survives_restart_and_held_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut s = LocalStore::open(&path, [1; 32], 100_000).unwrap();
    for n in 0..135 {
        s.enqueue(&format!("session:{n:03}"), b"held", 0, 60)
            .unwrap();
        s.enqueue(&format!("draft:{n:03}"), b"draft", 0, 60)
            .unwrap();
    }
    let mut found = std::collections::BTreeSet::new();
    for _ in 0..14 {
        let ids = s.next_session_ids(1).unwrap();
        assert!(ids.len() <= 10);
        found.extend(ids);
        drop(s);
        s = LocalStore::open(&path, [1; 32], 100_000).unwrap();
    }
    assert_eq!(found.len(), 135);
    assert_eq!(s.next_session_ids(1).unwrap()[0], "session:000");
    assert!(s.next_session_ids(60).unwrap().is_empty());
}

#[test]
fn pagination_filters_before_limiting_and_uses_literal_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = LocalStore::open(&dir.path().join("db"), [1; 32], 100_000).unwrap();
    for n in 0..125 {
        s.enqueue(&format!("session:{n:03}"), b"private", 0, 60)
            .unwrap();
        s.enqueue(&format!("draft:{n:03}"), b"summary", 0, 60)
            .unwrap();
    }
    let first = s.pending_ids("draft:", "", 1, 100).unwrap();
    let second = s
        .pending_ids("draft:", first.last().unwrap(), 1, 100)
        .unwrap();
    assert_eq!(first.len(), 100);
    assert_eq!(second.len(), 25);
    assert!(s.pending_ids("%", "", 1, 100).unwrap().is_empty());
    assert!(s.pending_ids("draft:", "", 60, 100).unwrap().is_empty());
    assert!(s.pending_ids("draft:", "", 1, 101).is_err());
}

#[test]
fn guarded_draft_cannot_be_regenerated_after_submit_or_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = LocalStore::open(&dir.path().join("db"), [1; 32], 100_000).unwrap();
    s.update("draft:a", 0, 60, |_| Ok(b"summary".to_vec()))
        .unwrap();
    s.ack("draft:a").unwrap();
    assert!(s.seen("draft:a").unwrap());
    assert!(s
        .update("draft:a", 1, 60, |_| Ok(b"duplicate".to_vec()))
        .is_err());
    s.update("draft:b", 0, 60, |_| Ok(b"summary".to_vec()))
        .unwrap();
    s.pending(60).unwrap();
    assert!(s.seen("draft:b").unwrap());
    assert!(s.read_record("draft:b", 60).unwrap().is_none());
}

#[test]
fn cleanup_does_not_need_a_server_and_guards_survive() {
    let d = tempfile::tempdir().unwrap();
    let mut s = LocalStore::open(&d.path().join("db"), [7; 32], 4096).unwrap();
    s.update("native:test", 100, 10, |_| Ok(b"synthetic".to_vec()))
        .unwrap();
    assert_eq!(s.purge_expired(110).unwrap(), 1);
    assert!(s.update("native:test", 111, 10, |_| Ok(vec![])).is_err());
}
#[test]
fn native_scan_rotates_without_disturbing_legacy_scan() {
    let d = tempfile::tempdir().unwrap();
    let mut s = LocalStore::open(&d.path().join("db"), [7; 32], 100_000).unwrap();
    for n in 0..25 {
        s.update(&format!("native:{n:03}"), 100, 100, |_| Ok(vec![]))
            .unwrap();
    }
    let mut ids = std::collections::BTreeSet::new();
    for _ in 0..3 {
        ids.extend(s.next_ids("native", "native:", 101, 10).unwrap());
        assert!(s.next_session_ids(101).unwrap().is_empty());
    }
    assert_eq!(ids.len(), 25);
    assert_eq!(
        s.next_ids("native", "native:", 101, 10).unwrap()[0],
        "native:000"
    );
}
