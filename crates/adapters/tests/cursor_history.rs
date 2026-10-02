use rusqlite::Connection;
use wk_adapters::cursor_history::{read, redact, within};
#[path = "support/cursor_fixture.rs"]
mod support;
use support::fixture;

#[test]
fn reads_both_layouts_and_scopes_to_the_workspace() {
    let d = tempfile::tempdir().unwrap();
    let (user, product) = fixture(d.path());
    let history = read(&user).unwrap();
    assert_eq!(
        history.unreadable_records, 1,
        "missing bubble is counted, not invented"
    );
    let mine = within(&history, &product);
    let titles: Vec<_> = mine.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Tenant rate limiter",
            "Untitled chat",
            "Legacy limiter chat"
        ]
    );
    let composer = &mine[0];
    assert_eq!(
        composer
            .messages
            .iter()
            .map(|m| m.role.as_str())
            .collect::<Vec<_>>(),
        ["user", "assistant"]
    );
    let all = serde_json::to_string(&mine).unwrap();
    for absent in [
        "HIDDEN_REASONING",
        "PERSONAL_CANARY",
        "ORPHAN_CANARY",
        "read_file",
    ] {
        assert!(!all.contains(absent), "{absent} leaked");
    }
    assert_eq!(mine[2].messages[1].text, "Use the shared middleware.");
    // The orphan is visible as unattributed, never assigned to a workspace.
    assert!(history
        .conversations
        .iter()
        .any(|c| c.title == "Orphan" && c.workspace.is_none()));
}

#[test]
fn snapshot_leaves_source_unchanged_and_works_while_locked() {
    let d = tempfile::tempdir().unwrap();
    let (user, _) = fixture(d.path());
    let db = user.join("globalStorage/state.vscdb");
    let before = std::fs::read(&db).unwrap();
    let lock = Connection::open(&db).unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
    assert!(!read(&user).unwrap().conversations.is_empty());
    lock.execute_batch("ROLLBACK").unwrap();
    assert_eq!(std::fs::read(&db).unwrap(), before);
}

#[test]
fn redaction_masks_tokens_and_key_blocks_but_keeps_prose() {
    let r = redact("key=sk-SYNTHETICabcdef1234567890 and ghp_abcdefghijklmnop1234 ok");
    assert_eq!(r, "[REDACTED] and [REDACTED] ok");
    let r = redact("-----BEGIN PRIVATE KEY-----\nABC\n-----END PRIVATE KEY-----\nDone");
    assert!(!r.contains("ABC") && r.ends_with("Done"));
    let prose = "Use backend/middleware/rate_limit.rs for tenant-aware limits";
    assert_eq!(redact(prose), prose);
}

#[test]
fn missing_directory_yields_empty_history() {
    let d = tempfile::tempdir().unwrap();
    assert!(read(d.path()).unwrap().conversations.is_empty());
}

#[test]
fn diagnose_reports_structure_without_values_paths_or_ids() {
    let d = tempfile::tempdir().unwrap();
    let (user, _) = fixture(d.path());
    let c = Connection::open(user.join("globalStorage/state.vscdb")).unwrap();
    c.execute("INSERT INTO cursorDiskKV VALUES('composerData:c-map',?)", [serde_json::json!({"conversationMap":{"/Users/someone/secret-project/file.rs":{"text":"MAP_CANARY"}},"text":"VALUE_CANARY"}).to_string().into_bytes()]).unwrap();
    drop(c);
    let report = wk_adapters::cursor_history::diagnose(&user)
        .unwrap()
        .to_string();
    for absent in [
        "sk-SYNTHETIC",
        "PERSONAL_CANARY",
        "HIDDEN_REASONING",
        "MAP_CANARY",
        "VALUE_CANARY",
        "secret-project",
        "c-new",
        "Tenant rate limiter",
        "my product",
    ] {
        assert!(
            !report.contains(absent),
            "{absent} leaked in diagnose output"
        );
    }
    for present in [
        "cursorDiskKV",
        "composerData",
        "fullConversationHeadersOnly",
        "<dynamic-key>",
        "empty_conversations",
    ] {
        assert!(report.contains(present), "{present} missing");
    }
}
