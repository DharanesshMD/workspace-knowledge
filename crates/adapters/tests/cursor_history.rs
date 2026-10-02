use rusqlite::Connection;
use wk_adapters::cursor_history::{read_with, redact, within};
#[path = "support/cursor_fixture.rs"]
mod support;
use support::fixture;

#[test]
fn reads_both_layouts_and_scopes_to_the_workspace() {
    let d = tempfile::tempdir().unwrap();
    let (user, product) = fixture(d.path());
    let history = read_with(&user, None).unwrap();
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
    assert!(!read_with(&user, None).unwrap().conversations.is_empty());
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
    assert!(read_with(d.path(), None).unwrap().conversations.is_empty());
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

#[test]
fn agent_transcripts_are_attributed_by_resolved_project_path() {
    let d = tempfile::tempdir().unwrap();
    let (user, product) = fixture(d.path());
    let dot = support::transcripts(d.path(), &product);
    let history = read_with(&user, Some(&dot)).unwrap();
    assert_eq!(
        history.unreadable_records, 2,
        "missing bubble + malformed transcript line"
    );
    let mine = within(&history, &product);
    let t = mine
        .iter()
        .find(|c| c.id.contains("11111111"))
        .expect("transcript in workspace");
    assert_eq!(t.title, "Reuse the limiter for billing");
    assert_eq!(
        t.messages.len(),
        2,
        "assistant steps merge into one message"
    );
    assert_eq!(t.messages[0].text, "Reuse the limiter for billing");
    assert_eq!(
        t.messages[1].text,
        "Checking the middleware.\n\nBilling now uses the shared limiter."
    );
    let shown = serde_json::to_string(&mine).unwrap();
    for absent in [
        "THINKING_CANARY",
        "TOOL_CANARY",
        "ATTACHED_CANARY",
        "PERSONAL_TRANSCRIPT_CANARY",
        "Gone project",
    ] {
        assert!(!shown.contains(absent), "{absent} leaked");
    }
    let gone = history
        .conversations
        .iter()
        .find(|c| c.title == "Gone project")
        .unwrap();
    assert!(
        gone.workspace.is_none(),
        "unresolvable project must stay unattributed"
    );
}

#[test]
fn transcripts_capture_changed_files_outcome_and_drop_injected_context() {
    let d = tempfile::tempdir().unwrap();
    let (user, product) = fixture(d.path());
    let dot = support::transcripts(d.path(), &product);
    let history = read_with(&user, Some(&dot)).unwrap();
    let c = history
        .conversations
        .iter()
        .find(|c| c.id.contains("44444444"))
        .unwrap();
    assert_eq!(c.messages[0].role, "user");
    assert_eq!(
        c.messages[0].text, "Wrap the badge in <div class=\"x\">hi</div> please",
        "HTML kept, injected block dropped"
    );
    assert_eq!(c.files_changed, ["backend/limits.rs", "web/app.tsx"]);
    assert_eq!(
        c.files_read,
        ["backend"],
        "files that were changed are not also listed as read"
    );
    assert_eq!(c.outcome.as_deref(), Some("## Done\nWrapped the badge."));
    assert!(
        c.complete,
        "fully read session ending with the agent's reply"
    );
    let gappy = history
        .conversations
        .iter()
        .find(|c| c.title == "Reuse the limiter for billing")
        .unwrap();
    assert!(!gappy.complete, "a malformed line keeps coverage partial");
    let all = serde_json::to_string(&history).unwrap();
    for absent in [
        "SYSTEM_CONTEXT_CANARY",
        "OLD_CANARY",
        "NEW_CANARY",
        "outside-workspace",
        "escape.txt",
    ] {
        assert!(!all.contains(absent), "{absent} leaked");
    }
}
