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
    let shown = run(&[
        "cursor-history",
        "--cursor-dir",
        u,
        "show",
        "--workspace",
        p,
    ]);
    assert!(shown.status.success());
    let text = String::from_utf8(shown.stdout).unwrap();
    assert!(text.contains("## Tenant rate limiter") && text.contains("Use the shared middleware."));
    for absent in [
        "sk-SYNTHETIC",
        "PERSONAL_CANARY",
        "ORPHAN_CANARY",
        "HIDDEN_REASONING",
    ] {
        assert!(!text.contains(absent), "{absent} shown");
    }
    let listed: serde_json::Value =
        serde_json::from_slice(&run(&["cursor-history", "--cursor-dir", u, "list"]).stdout)
            .unwrap();
    assert_eq!(listed["unattributed_conversations"], 1);

    let out = d.path().join("exports");
    let o = out.to_str().unwrap();
    let base = [
        "cursor-history",
        "--cursor-dir",
        u,
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
