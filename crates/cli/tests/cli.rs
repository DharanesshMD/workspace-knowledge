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
