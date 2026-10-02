use std::process::Command;
fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_wk"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn help_is_available() {
    let o = run(&["--help"]);
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("workspace"));
}
#[test]
fn approval_pause_and_scope_check() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("product");
    let state = d.path().join("state");
    std::fs::create_dir_all(root.join("backend")).unwrap();
    let r = root.to_str().unwrap();
    let s = state.to_str().unwrap();
    let p = root.join("backend");
    let p = p.to_str().unwrap();
    assert!(!run(&[
        "--state-dir",
        s,
        "workspace",
        "approve",
        r,
        "--product",
        "demo",
        "--service",
        "backend"
    ])
    .status
    .success());
    assert!(run(&[
        "--state-dir",
        s,
        "workspace",
        "approve",
        r,
        "--product",
        "demo",
        "--service",
        "backend",
        "--yes"
    ])
    .status
    .success());
    let o = run(&["--state-dir", s, "workspace", "check", r, p]);
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["eligible"], true);
    assert_eq!(v["team_authorized"], false);
    assert!(run(&["--state-dir", s, "workspace", "pause", r])
        .status
        .success());
    let o = run(&["--state-dir", s, "workspace", "check", r, p]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["eligible"], false);
}
#[test]
fn state_inside_product_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("backend")).unwrap();
    let o = run(&[
        "--state-dir",
        d.path().join("state").to_str().unwrap(),
        "workspace",
        "approve",
        d.path().to_str().unwrap(),
        "--product",
        "demo",
        "--service",
        "backend",
        "--yes",
    ]);
    assert!(!o.status.success());
}
#[test]
fn status_does_not_create_state_directory() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("product");
    std::fs::create_dir(&root).unwrap();
    let state = d.path().join("missing-state");
    let o = run(&[
        "--state-dir",
        state.to_str().unwrap(),
        "workspace",
        "status",
        root.to_str().unwrap(),
    ]);
    assert!(!o.status.success());
    assert!(!state.exists());
}
#[cfg(unix)]
#[test]
fn existing_directory_permissions_are_not_changed() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("product");
    std::fs::create_dir_all(root.join("backend")).unwrap();
    let state = d.path().join("existing");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
    let o = run(&[
        "--state-dir",
        state.to_str().unwrap(),
        "workspace",
        "approve",
        root.to_str().unwrap(),
        "--product",
        "demo",
        "--service",
        "backend",
        "--yes",
    ]);
    assert!(!o.status.success());
    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[path = "../../adapters/tests/support/cursor_fixture.rs"]
mod cursor_fixture;

#[test]
fn cursor_history_show_and_export_are_scoped_masked_and_confirmed() {
    let d = tempfile::tempdir().unwrap();
    let (user, product) = cursor_fixture::fixture(d.path());
    let (u, p) = (user.to_str().unwrap(), product.to_str().unwrap());
    // Never read the developer's real ~/.cursor during tests.
    let dot = cursor_fixture::transcripts(d.path(), &product);
    let dc = dot.to_str().unwrap();
    let shown = run(&[
        "cursor-history",
        "--cursor-dir",
        u,
        "--dot-cursor-dir",
        dc,
        "show",
        "--workspace",
        p,
    ]);
    assert!(shown.status.success());
    let text = String::from_utf8(shown.stdout).unwrap();
    assert!(text.contains("## Tenant rate limiter") && text.contains("Use the shared middleware."));
    assert!(
        text.contains("## Reuse the limiter for billing")
            && !text.contains("PERSONAL_TRANSCRIPT_CANARY")
    );
    for absent in [
        "sk-SYNTHETIC",
        "PERSONAL_CANARY",
        "ORPHAN_CANARY",
        "HIDDEN_REASONING",
    ] {
        assert!(!text.contains(absent), "{absent} shown");
    }
    let listed: serde_json::Value = serde_json::from_slice(
        &run(&[
            "cursor-history",
            "--cursor-dir",
            u,
            "--dot-cursor-dir",
            dc,
            "list",
        ])
        .stdout,
    )
    .unwrap();
    assert_eq!(listed["unattributed_conversations"], 2);

    let out = d.path().join("exports");
    let o = out.to_str().unwrap();
    let base = [
        "cursor-history",
        "--cursor-dir",
        u,
        "--dot-cursor-dir",
        dc,
        "export",
        "--workspace",
        p,
        "--product",
        "demo",
        "--out",
        o,
        "--session",
        "cursor-composer:c-new",
    ];
    assert!(!run(&base).status.success(), "export requires --yes");
    assert!(!out.exists());
    let personal = [
        "cursor-history",
        "--cursor-dir",
        u,
        "--dot-cursor-dir",
        dc,
        "export",
        "--workspace",
        p,
        "--product",
        "demo",
        "--out",
        o,
        "--session",
        "cursor-composer:c-personal",
        "--yes",
    ];
    assert!(
        !run(&personal).status.success(),
        "out-of-workspace session exported"
    );
    let inside = product.join("exports");
    let inside_args = [
        "cursor-history",
        "--cursor-dir",
        u,
        "--dot-cursor-dir",
        dc,
        "export",
        "--workspace",
        p,
        "--product",
        "demo",
        "--out",
        inside.to_str().unwrap(),
        "--session",
        "cursor-composer:c-new",
        "--yes",
    ];
    assert!(
        !run(&inside_args).status.success(),
        "export inside workspace allowed"
    );
    let mut ok = base.to_vec();
    ok.push("--yes");
    assert!(run(&ok).status.success());
    let file = out.join("cursor-composer_c-new.json");
    let body: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["session_id"], "cursor-composer:c-new");
    assert_eq!(body["messages"].as_array().unwrap().len(), 2);
    assert!(!body.to_string().contains("sk-SYNTHETIC"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(!run(&ok).status.success(), "existing export overwritten");
}

/// Fake Ollama `/api/chat` answering every request with one fixed summary; returns its URL.
fn fake_model() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/chat", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 16384];
            loop {
                let n = stream.read(&mut buf).unwrap();
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if raw.len() >= split + 4 + length || n == 0 {
                        break;
                    }
                }
            }
            let summary = serde_json::json!({"title":"Billing limiter reuse","intent":"Reuse the limiter for billing","rationale":"","decisions":[],"constraints":[],"open_questions":[],"sensitive":false});
            let body =
                serde_json::json!({"message":{"role":"assistant","content":summary.to_string()}})
                    .to_string();
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        }
    });
    url
}

#[test]
fn cursor_history_cards_carry_changed_files_and_session_time() {
    let d = tempfile::tempdir().unwrap();
    let (user, product) = cursor_fixture::fixture(d.path());
    let dot = cursor_fixture::transcripts(d.path(), &product);
    let out = d.path().join("cards.jsonl");
    let o = run(&[
        "cursor-history",
        "--cursor-dir",
        user.to_str().unwrap(),
        "--dot-cursor-dir",
        dot.to_str().unwrap(),
        "cards",
        "--workspace",
        product.to_str().unwrap(),
        "--latest",
        "10",
        "--service",
        "backend",
        "--endpoint",
        &fake_model(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let edited = lines
        .iter()
        .find(|l| l["id"].as_str().unwrap().contains("44444444"))
        .unwrap();
    assert_eq!(
        edited["card"]["evidence"],
        serde_json::json!([{"kind":"file_observation","reference":"backend/limits.rs"},{"kind":"file_observation","reference":"web/app.tsx"}])
    );
    assert!(edited["card"]["observed_at_ms"].as_i64().unwrap() > 0);
    assert_eq!(
        edited["card"]["service_ids"],
        serde_json::json!(["backend"])
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(
        !run(&[
            "cursor-history",
            "--cursor-dir",
            user.to_str().unwrap(),
            "--dot-cursor-dir",
            dot.to_str().unwrap(),
            "cards",
            "--workspace",
            product.to_str().unwrap(),
            "--out",
            out.to_str().unwrap()
        ])
        .status
        .success(),
        "existing output overwritten"
    );
}
