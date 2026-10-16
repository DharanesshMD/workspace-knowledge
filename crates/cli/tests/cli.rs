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
fn approval_returns_the_private_binding_needed_for_device_connection() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("product");
    let state = d.path().join("consent");
    std::fs::create_dir_all(root.join("backend")).unwrap();
    let output = run(&[
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
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let binding = value["binding_file"]
        .as_str()
        .expect("approval must identify its saved binding");
    let binding = std::path::Path::new(binding);
    assert_eq!(binding.parent().unwrap(), state.canonicalize().unwrap());
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(binding).unwrap()).unwrap();
    assert_eq!(saved["product"], "demo");
    assert_eq!(saved["services"], serde_json::json!(["backend"]));
    assert_eq!(value["team_authorized"], false);
    assert_eq!(value["live_capture_enabled"], false);
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

/// Run `wk` with every client's home-derived default pointing into `home`, so a test can never
/// read the developer's real agent history.
fn run_home(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_wk"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home)
        .env("XDG_CONFIG_HOME", home)
        .env("CLAUDE_CONFIG_DIR", home.join("claude"))
        .env("CODEX_HOME", home.join("codex"))
        .output()
        .unwrap()
}

const SECRET: &str = "password=SYNTHETIC_SECRET_0123456789abcdefghij";
const REPLY: &str = "Use the shared middleware.";

/// First prompt: its first line is the title; the rest is message text `list` must not print.
fn prompt() -> String {
    format!("Add tenant limits\nBODY_CANARY {SECRET}")
}

fn write_lines(path: &std::path::Path, records: &[serde_json::Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body: Vec<String> = records.iter().map(|r| r.to_string()).collect();
    std::fs::write(path, body.join("\n") + "\n").unwrap();
}

/// Claude Code `projects/` dir: one session in `product`, one in `other`, one without a folder.
fn claude_source(base: &std::path::Path, product: &std::path::Path) -> std::path::PathBuf {
    use serde_json::json;
    let projects = base.join("claude-projects");
    let session = |cwd: Option<&std::path::Path>, text: &str| {
        let rec = |kind: &str, uuid: &str, content: serde_json::Value| {
            let mut v = json!({"type":kind,"sessionId":"x","uuid":uuid,
                "timestamp":"2026-10-15T10:00:00.000Z","isSidechain":false,
                "message":{"role":kind,"content":content}});
            if let Some(cwd) = cwd {
                v["cwd"] = json!(cwd);
            }
            v
        };
        vec![
            rec("user", "u1", json!(text)),
            rec("assistant", "a1", json!([{"type":"text","text":REPLY}])),
        ]
    };
    write_lines(
        &projects.join("-product").join("in.jsonl"),
        &session(Some(product), &prompt()),
    );
    write_lines(
        &projects.join("-other").join("out.jsonl"),
        &session(Some(&base.join("other")), "OUTSIDE_CANARY"),
    );
    write_lines(
        &projects.join("-none").join("nofolder.jsonl"),
        &session(None, "NOFOLDER_CANARY"),
    );
    projects
}

fn codex_session(
    dir: &std::path::Path,
    file: &str,
    id: &str,
    cwd: Option<&std::path::Path>,
    text: &str,
) {
    use serde_json::json;
    let t = "2026-10-15T10:00:00.000Z";
    let mut meta = json!({"id":id,"timestamp":t});
    if let Some(cwd) = cwd {
        meta["cwd"] = json!(cwd);
    }
    let msg = |role: &str, part: &str, text: &str| {
        json!({"timestamp":t,"type":"response_item",
               "payload":{"type":"message","role":role,"content":[{"type":part,"text":text}]}})
    };
    write_lines(
        &dir.join("2026/10/15").join(file),
        &[
            json!({"timestamp":t,"type":"session_meta","payload":meta}),
            msg("user", "input_text", text),
            msg("assistant", "output_text", REPLY),
        ],
    );
}

/// Codex `sessions/` dir: one session in `product`, one in `other`, one without a folder.
fn codex_source(base: &std::path::Path, product: &std::path::Path) -> std::path::PathBuf {
    let sessions = base.join("codex-sessions");
    codex_session(&sessions, "rollout-a.jsonl", "in", Some(product), &prompt());
    codex_session(
        &sessions,
        "rollout-b.jsonl",
        "out",
        Some(&base.join("other")),
        "OUTSIDE_CANARY",
    );
    codex_session(
        &sessions,
        "rollout-c.jsonl",
        "nofolder",
        None,
        "NOFOLDER_CANARY",
    );
    sessions
}

/// A `file:///` URI for `path`, percent-encoding everything but unreserved characters.
fn file_uri(path: &std::path::Path) -> String {
    let path = path.to_str().unwrap().replace('\\', "/");
    let mut uri = String::from("file://");
    if !path.starts_with('/') {
        uri.push('/');
    }
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// VS Code `User/` dir: one Copilot session in `product`, one in `other`, one without a folder.
fn copilot_source(base: &std::path::Path, product: &std::path::Path) -> std::path::PathBuf {
    use serde_json::json;
    let user = base.join("Code").join("User");
    let storage = user.join("workspaceStorage");
    let session = |hash: &str, id: &str, folder: Option<&std::path::Path>, text: &str| {
        let dir = storage.join(hash);
        std::fs::create_dir_all(dir.join("chatSessions")).unwrap();
        if let Some(folder) = folder {
            std::fs::write(
                dir.join("workspace.json"),
                json!({"folder":file_uri(folder)}).to_string(),
            )
            .unwrap();
        }
        let body = json!({"version":3,"sessionId":id,"creationDate":1_792_058_400_000_i64,
            "requests":[{"requestId":"r1","timestamp":1_792_058_401_000_i64,
                "message":{"text":text},"response":[{"value":REPLY}]}]});
        std::fs::write(
            dir.join("chatSessions").join(format!("{id}.json")),
            body.to_string(),
        )
        .unwrap();
    };
    session("h-in", "in", Some(product), &prompt());
    session("h-out", "out", Some(&base.join("other")), "OUTSIDE_CANARY");
    session("h-none", "nofolder", None, "NOFOLDER_CANARY");
    user
}

fn json_out(o: &std::process::Output) -> serde_json::Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn agent_history_lists_shows_and_exports_each_client_scoped_and_masked() {
    type Source = fn(&std::path::Path, &std::path::Path) -> std::path::PathBuf;
    let clients: [(&str, Source); 3] = [
        ("claude-code", claude_source),
        ("codex", codex_source),
        ("copilot", copilot_source),
    ];
    for (client, source) in clients {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let product = d.path().join("product");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&product).unwrap();
        std::fs::create_dir_all(d.path().join("other")).unwrap();
        let src = source(d.path(), &product);
        let (s, p) = (src.to_str().unwrap(), product.to_str().unwrap());
        let inside = format!("{client}:in");
        let wk = |args: &[&str]| {
            let mut all = vec!["agent-history", "--client", client, "--source-dir", s];
            all.extend_from_slice(args);
            run_home(&home, &all)
        };

        // List: only this folder's sessions, titles and counts, never message text.
        let listed = wk(&["list", "--workspace", p]);
        let text = String::from_utf8_lossy(&listed.stdout).to_string();
        let v = json_out(&listed);
        for absent in [
            "BODY_CANARY",
            "SYNTHETIC_SECRET",
            REPLY,
            "OUTSIDE_CANARY",
            "NOFOLDER_CANARY",
        ] {
            assert!(!text.contains(absent), "{client}: list leaked {absent}");
        }
        let items = v["sessions"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{client}: {v}");
        assert_eq!(items[0]["id"], inside.as_str());
        assert_eq!(items[0]["title"], "Add tenant limits");
        assert_eq!(items[0]["messages"], 2);
        assert_eq!(items[0]["complete"], true);
        assert!(items[0]["created_at_ms"].as_i64().unwrap() > 0, "{client}");
        assert_eq!(v["outside_workspace"], 1, "{client}: {v}");
        assert_eq!(v["without_folder"], 1, "{client}: {v}");
        for count in ["unreadable_records", "empty_conversations", "skipped"] {
            assert!(v[count].is_u64(), "{client}: {count} missing");
        }

        // Show: masked Markdown of this folder only; unknown or outside sessions fail.
        let shown = wk(&["show", "--workspace", p]);
        assert!(shown.status.success(), "{client}");
        let text = String::from_utf8(shown.stdout).unwrap();
        assert!(
            text.contains(REPLY) && text.contains("BODY_CANARY"),
            "{client}"
        );
        assert!(text.contains("[REDACTED]"), "{client}");
        for absent in ["SYNTHETIC_SECRET", "OUTSIDE_CANARY", "NOFOLDER_CANARY"] {
            assert!(!text.contains(absent), "{client}: show leaked {absent}");
        }
        let one = wk(&["show", "--workspace", p, "--session", &inside]);
        assert!(String::from_utf8(one.stdout).unwrap().contains(REPLY));
        let outside = format!("{client}:out");
        for bad in [outside.as_str(), "nope"] {
            assert!(
                !wk(&["show", "--workspace", p, "--session", bad])
                    .status
                    .success(),
                "{client}: showed {bad}"
            );
        }

        // Export: confirmed, outside the folder, only this folder's sessions, private and masked.
        let out = d.path().join("exports").join(client);
        let o = out.to_str().unwrap();
        let export = |out: &str, session: &str, yes: bool| {
            let mut args = vec![
                "export",
                "--workspace",
                p,
                "--product",
                "demo",
                "--out",
                out,
                "--session",
                session,
            ];
            if yes {
                args.push("--yes");
            }
            wk(&args)
        };
        assert!(!export(o, &inside, false).status.success(), "{client}");
        assert!(!out.exists(), "{client}: unconfirmed export created --out");
        let inner = product.join("exports");
        assert!(
            !export(inner.to_str().unwrap(), &inside, true)
                .status
                .success(),
            "{client}: export inside workspace allowed"
        );
        assert!(
            !inner.exists(),
            "{client}: created a folder in the workspace"
        );
        assert!(!export("relative/out", &inside, true).status.success());
        for bad in [outside.as_str(), "nope"] {
            assert!(
                !export(o, bad, true).status.success(),
                "{client}: exported {bad}"
            );
        }
        let done = json_out(&export(o, &inside, true));
        assert_eq!(done["published"], false);
        let file = out.join(format!("{client}_in.json"));
        let body: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(body["schema_version"], 1);
        assert_eq!(body["product"], "demo");
        assert_eq!(body["session_id"], inside.as_str());
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert!(!body.to_string().contains("SYNTHETIC_SECRET"), "{client}");
        assert!(body.to_string().contains("[REDACTED]"), "{client}");
        #[cfg(unix)]
        {
            assert_eq!(mode(&file), 0o600, "{client}");
            assert_eq!(mode(&out), 0o700, "{client}");
        }
        assert!(
            !export(o, &inside, true).status.success(),
            "{client}: existing export overwritten"
        );
    }
}

#[test]
fn agent_history_export_names_never_escape_the_output_directory() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let product = d.path().join("product");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&product).unwrap();
    let sessions = d.path().join("codex-sessions");
    let long = "x".repeat(300);
    for (file, id) in [
        ("rollout-a.jsonl", "../../escape"),
        ("rollout-b.jsonl", "..\\..\\escape"),
        ("rollout-c.jsonl", long.as_str()),
    ] {
        codex_session(&sessions, file, id, Some(&product), &prompt());
    }
    // Export whatever IDs the reader produced (raw or reader-sanitized): with either, every
    // file must land inside --out under a safe, distinct name.
    let listed = json_out(&run_home(
        &home,
        &[
            "agent-history",
            "--client",
            "codex",
            "--source-dir",
            sessions.to_str().unwrap(),
            "list",
            "--workspace",
            product.to_str().unwrap(),
        ],
    ));
    let ids: Vec<String> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 3, "{listed}");
    let out = d.path().join("deep").join("exports");
    let mut args = vec![
        "agent-history",
        "--client",
        "codex",
        "--source-dir",
        sessions.to_str().unwrap(),
        "export",
        "--workspace",
        product.to_str().unwrap(),
        "--product",
        "demo",
        "--out",
        out.to_str().unwrap(),
        "--yes",
    ];
    for id in &ids {
        args.extend(["--session", id]);
    }
    let done = json_out(&run_home(&home, &args));
    assert_eq!(done["exported"].as_array().unwrap().len(), 3);
    let mut names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names.len(), 3, "{names:?}");
    for name in &names {
        assert!(
            name.starts_with("codex_") && name.ends_with(".json"),
            "{name}"
        );
        assert!(name.len() <= 125, "{name}");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)),
            "{name}"
        );
    }
    assert_ne!(names[0], names[1], "distinct ids must not collide");
    assert!(!d.path().join("escape.json").exists());
    assert!(!d.path().join("deep").join("escape.json").exists());
}

#[test]
fn agent_history_reads_cursor_like_cursor_history() {
    let d = tempfile::tempdir().unwrap();
    let (user, product) = cursor_fixture::fixture(d.path());
    let home = d.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // Cursor's agent transcripts live in `~/.cursor`; HOME points at the fixture, never the real one.
    std::fs::rename(
        cursor_fixture::transcripts(d.path(), &product),
        home.join(".cursor"),
    )
    .unwrap();
    let (u, p) = (user.to_str().unwrap(), product.to_str().unwrap());
    let base = ["agent-history", "--client", "cursor", "--source-dir", u];
    let listed = json_out(&run_home(
        &home,
        &[&base[..], &["list", "--workspace", p]].concat(),
    ));
    let ids: Vec<&str> = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"cursor-composer:c-new"), "{listed}");
    assert!(!listed.to_string().contains("sk-SYNTHETIC"));
    let shown = run_home(&home, &[&base[..], &["show", "--workspace", p]].concat());
    let text = String::from_utf8(shown.stdout).unwrap();
    assert!(text.contains("Use the shared middleware.") && !text.contains("sk-SYNTHETIC"));
    assert!(
        text.contains("Reuse the limiter for billing"),
        "transcripts read"
    );
}

/// A Codex source with sessions `in`, `Case` and `case` in `product`, plus the `wk` runner.
fn codex_export_setup(
    d: &std::path::Path,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let home = d.join("home");
    let product = d.join("product");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&product).unwrap();
    let sessions = d.join("codex-sessions");
    for (file, id) in [
        ("rollout-a.jsonl", "in"),
        ("rollout-b.jsonl", "Case"),
        ("rollout-c.jsonl", "case"),
    ] {
        codex_session(&sessions, file, id, Some(&product), &prompt());
    }
    (home, product, sessions)
}

fn codex_export(
    home: &std::path::Path,
    sessions: &std::path::Path,
    product: &std::path::Path,
    out: &std::path::Path,
    ids: &[&str],
) -> std::process::Output {
    let mut args = vec![
        "agent-history",
        "--client",
        "codex",
        "--source-dir",
        sessions.to_str().unwrap(),
        "export",
        "--workspace",
        product.to_str().unwrap(),
        "--product",
        "demo",
        "--out",
        out.to_str().unwrap(),
        "--yes",
    ];
    for id in ids {
        args.extend(["--session", id]);
    }
    run_home(home, &args)
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn agent_history_export_refuses_clashes_before_writing_anything() {
    let d = tempfile::tempdir().unwrap();
    let (home, product, sessions) = codex_export_setup(d.path());
    let out = d.path().join("out");
    let o = codex_export(&home, &sessions, &product, &out, &["codex:in", "codex:in"]);
    assert!(!o.status.success(), "duplicate session accepted");
    assert!(!out.exists());
    let o = codex_export(
        &home,
        &sessions,
        &product,
        &out,
        &["codex:Case", "codex:case"],
    );
    assert!(!o.status.success(), "case-only file name clash accepted");
    assert!(!out.exists());

    // A pre-existing target file: nothing is written, the existing file is untouched.
    std::fs::create_dir(&out).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(out.join("codex_in.json"), "keep").unwrap();
    let o = codex_export(
        &home,
        &sessions,
        &product,
        &out,
        &["codex:case", "codex:in"],
    );
    assert!(!o.status.success(), "existing target overwritten");
    assert_eq!(
        entries(&out),
        ["codex_in.json"],
        "partial export left behind"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("codex_in.json")).unwrap(),
        "keep"
    );
    json_out(&codex_export(
        &home,
        &sessions,
        &product,
        &out,
        &["codex:case"],
    ));
    assert_eq!(entries(&out), ["codex_case.json", "codex_in.json"]);
}

#[test]
fn agent_history_export_requires_review_and_names_the_show_command() {
    let d = tempfile::tempdir().unwrap();
    let (home, product, sessions) = codex_export_setup(d.path());
    let p = product.to_str().unwrap();
    let o = run_home(
        &home,
        &[
            "agent-history",
            "--client",
            "codex",
            "--source-dir",
            sessions.to_str().unwrap(),
            "export",
            "--workspace",
            p,
            "--product",
            "demo",
            "--out",
            d.path().join("out").to_str().unwrap(),
            "--session",
            "codex:in",
        ],
    );
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("wk agent-history --client codex show --workspace DIR"),
        "{err}"
    );
    assert!(!err.contains(p), "error carries the workspace path: {err}");
    assert!(!run_home(
        &home,
        &["agent-history", "--client", "vim", "list", "--workspace", p]
    )
    .status
    .success());
}

#[test]
fn agent_history_help_explains_scope_and_confirmation() {
    let d = tempfile::tempdir().unwrap();
    let help = |args: &[&str]| {
        let o = run_home(d.path(), args);
        assert!(o.status.success());
        String::from_utf8(o.stdout).unwrap()
    };
    let top = help(&["agent-history", "--help"]);
    assert!(top.contains("only the Cursor user directory"), "{top}");
    for client in ["claude-code", "codex", "copilot", "cursor"] {
        assert!(top.contains(client), "{top}");
    }
    let export = help(&["agent-history", "export", "--help"]);
    for text in [
        "absolute path outside the workspace; created private",
        "Confirm you reviewed `show` output",
    ] {
        assert!(export.contains(text), "{export}");
    }
}

#[cfg(unix)]
#[test]
fn agent_history_export_refuses_shared_or_symlinked_output_directories() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let d = tempfile::tempdir().unwrap();
    let (home, product, sessions) = codex_export_setup(d.path());
    let export =
        |out: &std::path::Path| codex_export(&home, &sessions, &product, out, &["codex:in"]);

    // An existing directory others can read is refused, and its permissions are left alone.
    let shared = d.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
    let o = export(&shared);
    assert!(!o.status.success(), "shared output directory accepted");
    assert!(String::from_utf8_lossy(&o.stderr).contains(
        "existing output directory must be private (mode 0700); permissions were not changed"
    ));
    assert_eq!(mode(&shared), 0o755);
    assert!(entries(&shared).is_empty());

    // An intermediate symlink into the workspace.
    symlink(&product, d.path().join("via")).unwrap();
    assert!(!export(&d.path().join("via").join("exports"))
        .status
        .success());
    // An existing --out that is a symlink to a private folder inside the workspace.
    let inner = product.join("existing");
    std::fs::create_dir(&inner).unwrap();
    std::fs::set_permissions(&inner, std::fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&inner, d.path().join("link")).unwrap();
    assert!(!export(&d.path().join("link")).status.success());
    assert!(entries(&inner).is_empty());
    // Dangling symlinks into the workspace, as --out itself and as an intermediate folder.
    symlink(product.join("dangling"), d.path().join("dangling")).unwrap();
    assert!(!export(&d.path().join("dangling")).status.success());
    symlink(product.join("dangling2"), d.path().join("dangling2")).unwrap();
    assert!(!export(&d.path().join("dangling2").join("exports"))
        .status
        .success());
    assert_eq!(
        entries(&product),
        ["existing"],
        "something was created in the workspace"
    );
}
