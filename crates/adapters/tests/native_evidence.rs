use wk_adapters::native_evidence::*;
fn complete() -> Evidence {
    Evidence {
        client: Client::Codex,
        client_version: "0.162.0-alpha.2".into(),
        parser_version: 1,
        session: "s".into(),
        agent: "main".into(),
        turn: "t".into(),
        prompt: "hello".into(),
        response: "ok".into(),
        messages_complete: true,
        scope: Scope::Verified,
        terminal: Terminal::Complete,
        digest: "digest".into(),
        files_read: vec![],
        files_changed: vec![],
        hold_reason: None,
    }
}
#[test]
fn completeness_alone_cannot_authorize_native_evidence() {
    assert!(eligible(&complete()));
    for terminal in [Terminal::Open, Terminal::Cancelled, Terminal::Held] {
        let mut e = complete();
        e.terminal = terminal;
        assert!(!eligible(&e));
    }
    for scope in [Scope::Unknown, Scope::Violated] {
        let mut e = complete();
        e.scope = scope;
        assert!(!eligible(&e));
    }
    let mut e = complete();
    e.messages_complete = false;
    assert!(!eligible(&e));
    let mut e = complete();
    e.hold_reason = Some("uncertain".into());
    assert!(!eligible(&e));
}
