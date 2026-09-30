use wk_scope::enrollment::Binding;
#[test]
fn consent_binds_scope_and_pause() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("backend")).unwrap();
    std::fs::create_dir(d.path().join("personal")).unwrap();
    let mut b = Binding::approve(d.path(), "product", vec!["backend".into()]).unwrap();
    assert!(b.eligible(&d.path().join("backend")));
    b.services.push("personal".into());
    assert!(!b.eligible(&d.path().join("personal")));
    let mut b = Binding::approve(d.path(), "product", vec!["backend".into()]).unwrap();
    b.pause();
    assert!(!b.eligible(&d.path().join("backend")));
}
#[test]
fn rejects_invalid_enrollment() {
    let d = tempfile::tempdir().unwrap();
    assert!(Binding::approve(d.path(), "", vec![]).is_err());
    assert!(Binding::approve(d.path(), "product", vec!["../".into()]).is_err());
    assert!(Binding::approve(d.path(), "product", vec!["missing".into()]).is_err());
}
#[test]
fn denies_sensitive_alias_before_resolution() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("backend")).unwrap();
    std::fs::write(d.path().join("backend/plain"), "secret").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("plain", d.path().join("backend/.env")).unwrap();
        assert!(!wk_scope::approved(
            d.path(),
            &d.path().join("backend/.env"),
            &["backend".into()]
        ));
    }
}
#[cfg(unix)]
#[test]
fn replacing_approved_root_requires_fresh_consent() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("product");
    std::fs::create_dir_all(root.join("backend")).unwrap();
    let b = Binding::approve(&root, "product", vec!["backend".into()]).unwrap();
    std::fs::rename(&root, d.path().join("old")).unwrap();
    std::fs::create_dir_all(root.join("backend")).unwrap();
    assert!(!b.eligible(&root.join("backend")));
}
#[cfg(unix)]
#[test]
fn service_retarget_does_not_grant_personal_folder() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    std::fs::create_dir(root.join("backend")).unwrap();
    std::fs::create_dir(root.join("personal")).unwrap();
    std::fs::write(root.join("personal/notes"), "PRIVATE").unwrap();
    let b = Binding::approve(root, "product", vec!["backend".into()]).unwrap();
    std::fs::rename(root.join("backend"), root.join("old-backend")).unwrap();
    std::os::unix::fs::symlink("personal", root.join("backend")).unwrap();
    assert!(!b.eligible(&root.join("personal/notes")));
}
