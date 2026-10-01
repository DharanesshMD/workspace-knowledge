use wk_local::LocalStore;
#[test]
fn update_is_atomic_and_does_not_extend_retention() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("queue.db");
    let mut s = LocalStore::open(&path, [7; 32], 4096).unwrap();
    s.update("session", 100, 10, |old| {
        assert!(old.is_none());
        Ok(b"one".to_vec())
    })
    .unwrap();
    s.update("session", 105, 10, |old| {
        assert_eq!(old.unwrap(), b"one");
        Ok(b"two".to_vec())
    })
    .unwrap();
    assert!(s
        .update("session", 106, 10, |_| Err("rejected".into()))
        .is_err());
    drop(s);
    let mut s = LocalStore::open(&path, [7; 32], 4096).unwrap();
    assert_eq!(s.pending(109).unwrap()[0].1, b"two");
    assert!(s.pending(110).unwrap().is_empty());
}
#[test]
fn failed_growth_preserves_encrypted_original() {
    let d = tempfile::tempdir().unwrap();
    let mut s = LocalStore::open(&d.path().join("queue.db"), [7; 32], 100).unwrap();
    s.update("s", 0, 10, |_| Ok(b"PRIVATE_CANARY".to_vec()))
        .unwrap();
    assert!(s.update("s", 1, 10, |_| Ok(vec![0; 100])).is_err());
    assert_eq!(s.pending(1).unwrap()[0].1, b"PRIVATE_CANARY");
    let bytes = std::fs::read(d.path().join("queue.db")).unwrap();
    assert!(!bytes.windows(14).any(|w| w == b"PRIVATE_CANARY"));
}

#[test]
fn concurrent_connections_do_not_lose_updates() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("queue.db");
    let mut s = LocalStore::open(&path, [7; 32], 4096).unwrap();
    s.update("counter", 0, 100, |_| Ok(vec![0])).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut s = LocalStore::open(&path, [7; 32], 4096).unwrap();
                barrier.wait();
                for _ in 0..10 {
                    s.update("counter", 1, 100, |old| Ok(vec![old.unwrap()[0] + 1]))
                        .unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(s.pending(2).unwrap()[0].1, vec![20]);
}

#[test]
fn wrong_key_cannot_replace_existing_payload() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("queue.db");
    let mut s = LocalStore::open(&path, [7; 32], 4096).unwrap();
    s.update("s", 0, 10, |_| Ok(b"original".to_vec())).unwrap();
    let mut other = LocalStore::open(&path, [8; 32], 4096).unwrap();
    assert!(other
        .update("s", 1, 10, |_| Ok(b"replacement".to_vec()))
        .is_err());
    assert_eq!(s.pending(2).unwrap()[0].1, b"original");
}

#[test]
fn ack_and_expiry_close_ids_across_every_write_api() {
    let d = tempfile::tempdir().unwrap();
    let mut s = LocalStore::open(&d.path().join("queue.db"), [7; 32], 4096).unwrap();
    for id in ["acked", "expired"] {
        s.update(id, 0, 10, |_| Ok(b"private".to_vec())).unwrap();
    }
    s.ack("acked").unwrap();
    assert!(s.update("acked", 1, 10, |_| Ok(vec![])).is_err());
    assert!(s.enqueue("acked", b"retry", 1, 10).is_err());
    assert!(s.update("expired", 10, 10, |_| Ok(vec![])).is_err());
    assert!(s.enqueue("expired", b"retry", 10, 10).is_err());
    assert!(s.pending(10).unwrap().is_empty());
}
