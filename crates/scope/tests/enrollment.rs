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
fn inode_only_legacy_approval_requires_fresh_consent() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::MetadataExt;
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("backend")).unwrap();
    let mut b = Binding::approve(d.path(), "product", vec!["backend".into()]).unwrap();
    let meta = std::fs::metadata(&b.root).unwrap();
    let legacy = serde_json::to_vec(&(
        &b.root,
        &b.product,
        &b.services,
        format!("{}:{}", meta.dev(), meta.ino()),
    ))
    .unwrap();
    b.policy_hash = format!("{:x}", Sha256::digest(legacy));
    assert!(
        !b.intact(),
        "inode-only approval cannot distinguish a recycled inode"
    );
}
#[test]
fn ordinary_workspace_changes_preserve_approval() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("backend")).unwrap();
    let b = Binding::approve(d.path(), "product", vec!["backend".into()]).unwrap();
    std::fs::create_dir(d.path().join("new-module")).unwrap();
    std::fs::write(d.path().join("new-file"), "work").unwrap();
    std::fs::remove_file(d.path().join("new-file")).unwrap();
    assert!(b.intact());
}
#[cfg(target_os = "linux")]
#[test]
fn rapid_root_recreation_never_reuses_consent() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("product");
    for _ in 0..64 {
        std::fs::create_dir_all(root.join("backend")).unwrap();
        let b = Binding::approve(&root, "product", vec!["backend".into()]).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::create_dir_all(root.join("backend")).unwrap();
        assert!(!b.intact(), "replacement directory inherited old consent");
        std::fs::remove_dir_all(&root).unwrap();
    }
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
