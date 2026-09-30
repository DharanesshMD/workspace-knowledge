use wk_local::LocalStore;
#[test]
fn encrypted_events_survive_restart_without_plaintext() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("capture.db");
    {
        let mut s = LocalStore::open(&p, [7; 32], 4096).unwrap();
        assert!(s
            .enqueue("evt-1", b"PRIVATE_PROMPT_CANARY", 100, 3600)
            .unwrap());
    }
    let mut s = LocalStore::open(&p, [7; 32], 4096).unwrap();
    assert_eq!(
        s.pending(101).unwrap(),
        vec![("evt-1".into(), b"PRIVATE_PROMPT_CANARY".to_vec())]
    );
    for e in std::fs::read_dir(d.path()).unwrap() {
        let bytes = std::fs::read(e.unwrap().path()).unwrap();
        assert!(!bytes.windows(21).any(|w| w == b"PRIVATE_PROMPT_CANARY"));
    }
}
#[test]
fn duplicate_event_is_not_overwritten() {
    let d = tempfile::tempdir().unwrap();
    let mut s = LocalStore::open(&d.path().join("db"), [1; 32], 4096).unwrap();
    assert!(s.enqueue("1", b"original", 0, 60).unwrap());
    assert!(!s.enqueue("1", b"changed", 1, 60).unwrap());
    assert_eq!(s.pending(2).unwrap()[0].1, b"original");
}
#[test]
fn expiry_and_ack_remove_payloads() {
    let d = tempfile::tempdir().unwrap();
    let mut s = LocalStore::open(&d.path().join("db"), [1; 32], 4096).unwrap();
    s.enqueue("old", b"expired", 0, 5).unwrap();
    s.enqueue("new", b"keep", 4, 60).unwrap();
    assert_eq!(
        s.pending(5)
            .unwrap()
            .iter()
            .map(|x| x.0.as_str())
            .collect::<Vec<_>>(),
        vec!["new"]
    );
    s.ack("new").unwrap();
    assert!(s.pending(5).unwrap().is_empty());
}
#[test]
fn wrong_key_fails_closed() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    LocalStore::open(&p, [1; 32], 4096)
        .unwrap()
        .enqueue("e", b"private", 0, 60)
        .unwrap();
    assert!(LocalStore::open(&p, [2; 32], 4096)
        .unwrap()
        .pending(1)
        .is_err());
}
#[test]
fn quota_and_zero_retention_reject_persistence() {
    let d = tempfile::tempdir().unwrap();
    let mut s = LocalStore::open(&d.path().join("db"), [1; 32], 64).unwrap();
    assert!(s.enqueue("e", &[9; 128], 0, 60).is_err());
    assert!(s.enqueue("e", b"raw", 0, 0).is_err());
    assert!(s.pending(0).unwrap().is_empty());
}
#[test]
fn ciphertext_tampering_is_detected() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    LocalStore::open(&p, [1; 32], 4096)
        .unwrap()
        .enqueue("e", b"private", 0, 60)
        .unwrap();
    let conn = rusqlite::Connection::open(&p).unwrap();
    conn.execute("UPDATE events SET expires=999", []).unwrap();
    drop(conn);
    assert!(LocalStore::open(&p, [1; 32], 4096)
        .unwrap()
        .pending(1)
        .is_err());
}
