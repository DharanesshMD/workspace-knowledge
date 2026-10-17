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
fn redaction_masks_short_credential_assignments_but_keeps_the_key() {
    for (input, want) in [
        ("password=hunter2", "password=[REDACTED]"),
        ("PASSWORD=hunter2 next", "PASSWORD=[REDACTED] next"),
        ("DB_PASSWORD=hunter2", "DB_PASSWORD=[REDACTED]"),
        (
            "export GITHUB_TOKEN=abc123",
            "export GITHUB_TOKEN=[REDACTED]",
        ),
        ("passwd: s3cret, user: me", "passwd: [REDACTED], user: me"),
        ("pwd = x1", "pwd = [REDACTED]"),
        ("Secret:abc", "Secret:[REDACTED]"),
        ("token='t0k'", "token='[REDACTED]'"),
        (
            "api_key=k1 apikey=k2 api-key=k3",
            "api_key=[REDACTED] apikey=[REDACTED] api-key=[REDACTED]",
        ),
        (
            "access_key: a1\nprivate_key: p1",
            "access_key: [REDACTED]\nprivate_key: [REDACTED]",
        ),
        ("client_secret=c1;", "client_secret=[REDACTED]"),
        ("auth: Bearer xyz", "auth: Bearer [REDACTED]"),
        ("bearer: b1", "bearer: [REDACTED]"),
        (
            r#"{"password": "hunter2", "user": "me"}"#,
            r#"{"password": "[REDACTED]", "user": "me"}"#,
        ),
        (r#"{"token":"t1"}"#, r#"{"token":"[REDACTED]"}"#),
        (
            "Authorization: Bearer abc.def",
            "Authorization: Bearer [REDACTED]",
        ),
        (
            r#""Authorization": "Bearer abc""#,
            r#""Authorization": "Bearer [REDACTED]""#,
        ),
        (
            "authorization: Basic dXNlcg==",
            "authorization: Basic [REDACTED]",
        ),
        ("config.password=x", "config.password=[REDACTED]"),
        (
            "pässword=x password=\u{e9}t\u{e9}",
            "pässword=x password=[REDACTED]",
        ),
    ] {
        assert_eq!(redact(input), want, "{input}");
    }
}

#[test]
fn redaction_leaves_credential_words_without_assignments_alone() {
    for prose in [
        "Reset the password for the admin account",
        "The token budget is spent; auth is handled upstream",
        "max_tokens=500 and author: Jane and tokens: 5",
        "password=",
        "password: \"\"",
        "password == other",
        "see https://x.test/secret/path",
        "auth: xyz",
        "oauth=x ppwd=x signature=abc",
    ] {
        assert_eq!(redact(prose), prose);
    }
    // Long tokens keep their existing whole-word masking.
    assert_eq!(
        redact("token=sk-SYNTHETICabcdef1234567890 ok"),
        "[REDACTED] ok"
    );
}

const COMPOUND_AND_QUOTED: &[(&str, &str)] = &[
    ("accessToken=a1", "accessToken=[REDACTED]"),
    (r#""refreshToken": "r1""#, r#""refreshToken": "[REDACTED]""#),
    ("clientSecret: c1", "clientSecret: [REDACTED]"),
    (
        "secretAccessKey=wJalrXUtn/K7MDENG/bPxRfi",
        "secretAccessKey=[REDACTED]",
    ),
    ("openaiApiKey=k1", "openaiApiKey=[REDACTED]"),
    ("PGPASSWORD=pg1 psql", "PGPASSWORD=[REDACTED] psql"),
    ("SECRET_KEY=s1", "SECRET_KEY=[REDACTED]"),
    ("SECRET_KEY_BASE=s2", "SECRET_KEY_BASE=[REDACTED]"),
    ("passphrase: pp", "passphrase: [REDACTED]"),
    (
        "signing_key=a session_key=b",
        "signing_key=[REDACTED] session_key=[REDACTED]",
    ),
    (
        "encryption_key=c credentials: d",
        "encryption_key=[REDACTED] credentials: [REDACTED]",
    ),
    ("dbPwd=p", "dbPwd=[REDACTED]"),
    (
        "https://x.test/f?sig=abc123&se=2026",
        "https://x.test/f?sig=[REDACTED]&se=2026",
    ),
    (
        "https://x.test/f?X-Amz-Signature=ab/c+d==&x=1",
        "https://x.test/f?X-Amz-Signature=[REDACTED]&x=1",
    ),
    (
        r#"password = "correct horse battery staple""#,
        r#"password = "[REDACTED]""#,
    ),
    (
        r#"{"password": "a\"b c", "user": "me"}"#,
        r#"{"password": "[REDACTED]", "user": "me"}"#,
    ),
    ("password='x, y' z", "password='[REDACTED]' z"),
    ("password=a,b next", "password=[REDACTED] next"),
    ("password=abc, next", "password=[REDACTED], next"),
    (
        r#"{\"password\":\"hu nter2\",\"x\":1}"#,
        r#"{\"password\":\"[REDACTED]\",\"x\":1}"#,
    ),
    (
        "postgres://app:hunter2@db.test:5432/app",
        "postgres://app:[REDACTED]@db.test:5432/app",
    ),
    (
        "git clone https://me:p%40ss@github.com/o/r.git",
        "git clone https://me:[REDACTED]@github.com/o/r.git",
    ),
    (
        "curl -u alice:s3cret https://x.test",
        "curl -u alice:[REDACTED] https://x.test",
    ),
    (
        r#"curl --user "alice:pass word" url"#,
        r#"curl --user "alice:[REDACTED]" url"#,
    ),
    (
        "curl --user=alice:pw url",
        "curl --user=alice:[REDACTED] url",
    ),
    (
        r#"const PASSWORD: &str = "hunter2";"#,
        r#"const PASSWORD: &str = "[REDACTED]";"#,
    ),
    (
        "static TOKEN: &'static str = 'abc';",
        "static TOKEN: &'static str = '[REDACTED]';",
    ),
    (
        "let x = {password => 'p1'}",
        "let x = {password => '[REDACTED]'}",
    ),
    (
        "password:\n  hunter2\nnext: x\n",
        "password:\n  [REDACTED]\nnext: x\n",
    ),
    (
        "token: |\n  line1\n  line2\n\n  line3\nafter\n",
        "token: |\n  [REDACTED]\n  [REDACTED]\n\n  [REDACTED]\nafter\n",
    ),
    (
        "  - secret:\n      name: foo\n  - other: 1",
        "  - secret:\n      [REDACTED]\n  - other: 1",
    ),
    ("x-api-key: abc\r\nnext", "x-api-key: [REDACTED]\r\nnext"),
    (
        "Proxy-Authorization: Basic abc",
        "Proxy-Authorization: Basic [REDACTED]",
    ),
    ("PASSWORD=été ünï", "PASSWORD=[REDACTED] ünï"),
    (
        "aws_secret_access_key = abc/def",
        "aws_secret_access_key = [REDACTED]",
    ),
    ("token: Token abc", "token: Token [REDACTED]"),
];

#[test]
fn redaction_masks_compound_names_quoted_values_urls_and_blocks() {
    for (input, want) in COMPOUND_AND_QUOTED {
        assert_eq!(redact(input), *want, "{input}");
    }
}

const TOKEN_USERINFO: &str = "ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// Examples written with `{T}` standing for a 40-character GitHub token.
fn with_token(text: &str) -> String {
    text.replace("{T}", TOKEN_USERINFO)
}

const FLAGS_USERINFO_COOKIES: &[(&str, &str)] = &[
    (
        "git clone https://{T}@github.com/o/r.git",
        "git clone https://[REDACTED]@github.com/o/r.git",
    ),
    (
        "https://{T}:x-oauth-basic@github.com/o/r.git",
        "https://[REDACTED]:[REDACTED]@github.com/o/r.git",
    ),
    (
        "mytool --password hunter2 --verbose",
        "mytool --password [REDACTED] --verbose",
    ),
    (
        "t --token abc --secret \"two words\" --api-key k --auth-token t --passwd p",
        "t --token [REDACTED] --secret \"[REDACTED]\" --api-key [REDACTED] --auth-token [REDACTED] --passwd [REDACTED]",
    ),
    ("t --access-token tk --db-password 'a b'", "t --access-token [REDACTED] --db-password '[REDACTED]'"),
    ("t --password=x --token=y z", "t --password=[REDACTED] --token=[REDACTED] z"),
    (
        "Cookie: a=1; b=two",
        "Cookie: a=[REDACTED]; b=[REDACTED]",
    ),
    (
        "Set-Cookie: sid=abc123; Path=/; HttpOnly",
        "Set-Cookie: sid=[REDACTED]; Path=/; HttpOnly",
    ),
    (
        "curl -H 'Cookie: a=1; b=2' url",
        "curl -H 'Cookie: a=[REDACTED]; b=[REDACTED]' url",
    ),
    (
        r#"{"Cookie": "a=1; b=2", "x": "y"}"#,
        r#"{"Cookie": "a=[REDACTED]; b=[REDACTED]", "x": "y"}"#,
    ),
    ("cookie: a=b", "cookie: a=[REDACTED]"),
];

#[test]
fn redaction_masks_userinfo_tokens_secret_flags_and_cookies() {
    for (input, want) in FLAGS_USERINFO_COOKIES {
        let (input, want) = (with_token(input), with_token(want));
        assert_eq!(redact(&input), want, "{input}");
    }
}

#[test]
fn redaction_leaves_ordinary_flags_users_and_cookie_words_alone() {
    for text in [
        "https://alice@host/x",
        "t --no-password next",
        "t --token-file /tmp/f --verbose",
        "t --password --verbose",
        "t --password",
        "ssh -p 22 host; docker run -p 80:80 img",
        "cookies are tasty: yes",
        "Cookie:",
        "Cookie: ",
        "set cookie: later",
    ] {
        assert_eq!(redact(text), text);
    }
}

#[test]
fn redaction_stays_linear_on_pathological_lines() {
    // Every key is followed by a type annotation and a long run that never reaches `=`.
    let line = "token: String ".repeat(60_000);
    let started = std::time::Instant::now();
    assert_eq!(redact(&line), line);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn redaction_avoids_code_and_url_false_positives() {
    for text in [
        "crate::auth::login(token)",
        "use crate::auth::token::verify;",
        "pub token: &str,",
        "token: String,",
        "password: Option<String>",
        "let password: &'static str;",
        "secret: string; auth: bool",
        "https://user@host/x",
        "https://host:8080/a?email=me@x.test",
        "curl -u alice https://x.test",
        "curl --user-agent x:y",
        "auth:\n  x",
        "password:\n",
        "a token:\nplain line",
    ] {
        assert_eq!(redact(text), text);
    }
}

#[test]
fn redaction_is_idempotent() {
    let mut inputs: Vec<String> = COMPOUND_AND_QUOTED
        .iter()
        .chain(FLAGS_USERINFO_COOKIES)
        .map(|(i, _)| with_token(i))
        .collect();
    inputs.extend(
        [
            "-----BEGIN PRIVATE KEY-----\nABC\n-----END PRIVATE KEY-----\nDone",
            "key=sk-SYNTHETICabcdef1234567890 and ghp_abcdefghijklmnop1234 ok",
            "Authorization: Bearer [REDACTED]",
            "password: [REDACTED],",
            "user:[REDACTED]@host",
        ]
        .map(String::from),
    );
    for input in inputs {
        let once = redact(&input);
        assert_eq!(redact(&once), once, "{input}");
    }
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
