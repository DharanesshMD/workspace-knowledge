use wk_scope::consent::*;

fn product() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap().join("product");
    std::fs::create_dir_all(root.join("backend")).unwrap();
    (d, root)
}
#[test]
fn approval_writes_a_private_binding_and_reports_it() {
    let (d, root) = product();
    let state = d.path().canonicalize().unwrap().join("consent");
    let approved = approve(Some(&state), &root, "demo", vec!["backend".into()]).unwrap();
    assert_eq!(approved.binding_file, binding_file(&state, &root).unwrap());
    assert_eq!(approved.binding_file.parent().unwrap(), state);
    let summary = approved.summary();
    assert_eq!(summary["local_approved"], true);
    assert_eq!(
        summary["binding_file"],
        serde_json::json!(approved.binding_file)
    );
    assert_eq!(summary["root"], serde_json::json!(root));
    assert_eq!(summary["services"], serde_json::json!(["backend"]));
    assert_eq!(summary["team_authorized"], false);
    assert_eq!(summary["live_capture_enabled"], false);
    let stored = read_binding(&state, &root).unwrap();
    assert!(stored.intact() && stored.eligible(&root.join("backend")));
    assert!(!stored.eligible(&root));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&state).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }
}
#[test]
fn the_consent_directory_rules_are_unchanged() {
    let (d, root) = product();
    let base = d.path().canonicalize().unwrap();
    let services = || vec!["backend".to_string()];
    let refused = |state: Option<&std::path::Path>| approve(state, &root, "demo", services()).err();
    assert_eq!(
        refused(None).unwrap(),
        "--state-dir is required for local consent"
    );
    assert!(refused(Some(std::path::Path::new("relative")))
        .unwrap()
        .contains("absolute"));
    // PathBuf::join normalizes `..` for Windows verbatim prefixes. Preserve
    // the literal caller input so this actually exercises traversal refusal.
    let mut traversal = base.as_os_str().to_os_string();
    traversal.push(std::path::MAIN_SEPARATOR_STR);
    traversal.push("a/../b");
    let traversal = std::path::PathBuf::from(traversal);
    assert!(refused(Some(&traversal))
        .unwrap()
        .contains("parent traversal"));
    assert_eq!(
        refused(Some(&root.join("consent"))).unwrap(),
        "state directory must be outside product folder"
    );
    assert!(!root.join("consent").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let open = base.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(refused(Some(&open)).unwrap().contains("private"));
        assert!(std::fs::read_dir(&open).unwrap().next().is_none());
        // A symlink into the product folder resolves before the check.
        std::os::unix::fs::symlink(&root, base.join("alias")).unwrap();
        assert!(refused(Some(&base.join("alias/consent")))
            .unwrap()
            .contains("outside product folder"));
    }
    // Validation of folder and services happens before anything is created.
    let state = base.join("fresh");
    assert!(approve(Some(&state), &root, "demo", vec!["missing".into()]).is_err());
    assert!(approve(Some(&state), &root, "bad product", services()).is_err());
    assert!(!state.exists());
}
#[test]
fn a_state_directory_can_be_checked_without_creating_it() {
    let (d, root) = product();
    let base = d.path().canonicalize().unwrap();
    assert!(check_state_dir(&base.join("missing/consent"), &root).is_ok());
    assert!(!base.join("missing").exists());
    assert!(check_state_dir(&root.join("consent"), &root).is_err());
    assert!(check_state_dir(std::path::Path::new("relative"), &root).is_err());
}
#[test]
fn a_folder_must_be_apart_from_the_consent_directory() {
    let (d, root) = product();
    let base = d.path().canonicalize().unwrap();
    assert!(folder_apart(&base.join("consent"), &root).is_ok());
    assert!(
        folder_apart(&root.join("consent"), &root).is_err(),
        "inside the folder"
    );
    assert!(
        folder_apart(&root.join("backend"), &root).is_err(),
        "inside the folder"
    );
    assert!(folder_apart(&base, &root).is_err(), "contains the folder");
    assert!(folder_apart(&root, &root).is_err(), "is the folder");
    let nested = base.join("consent/project");
    std::fs::create_dir_all(&nested).unwrap();
    assert!(
        folder_apart(&base.join("consent"), &nested).is_err(),
        "folder inside consent"
    );
}
#[test]
fn pausing_is_saved_and_read_back() {
    let (d, root) = product();
    let state = d.path().canonicalize().unwrap().join("consent");
    approve(Some(&state), &root, "demo", vec!["backend".into()]).unwrap();
    let mut binding = read_binding(&state, &root).unwrap();
    binding.pause();
    assert!(!binding.intact());
    save_binding(&state, &binding).unwrap();
    let again = read_binding(&state, &root).unwrap();
    assert!(!again.enabled && !again.eligible(&root.join("backend")));
    assert!(read_binding(&state, &root.join("backend")).is_err());
}
