use std::fs;
use wk_scope::approved;
#[test]
fn only_approved_service_is_eligible() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("product");
    fs::create_dir_all(root.join("backend")).unwrap();
    fs::create_dir_all(root.join("personal")).unwrap();
    let services = vec!["backend".to_string()];
    assert!(approved(&root, &root.join("backend"), &services));
    assert!(!approved(&root, &root.join("personal"), &services));
    assert!(!approved(&root, &root, &services));
}
#[test]
fn rejects_sibling_prefix_and_parent_components() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("product");
    fs::create_dir_all(root.join("backend")).unwrap();
    fs::create_dir_all(d.path().join("product-personal")).unwrap();
    let s = vec!["backend".to_string()];
    assert!(!approved(&root, &d.path().join("product-personal"), &s));
    assert!(!approved(
        &root,
        &root.join("backend/../../product-personal"),
        &s
    ));
    assert!(!approved(
        &root,
        &root.join("backend"),
        &["../product-personal".into()]
    ));
}
#[test]
fn excludes_sensitive_and_unreadable_paths() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    fs::create_dir(root.join("backend")).unwrap();
    for name in [".env", ".env.local", "key.pem", "id_rsa"] {
        let p = root.join("backend").join(name);
        fs::write(&p, "CANARY").unwrap();
        assert!(!approved(root, &p, &["backend".into()]));
    }
    assert!(!approved(
        root,
        &root.join("backend/missing"),
        &["backend".into()]
    ));
}
#[cfg(unix)]
#[test]
fn rejects_outside_symlink_and_symlink_service() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("product");
    fs::create_dir_all(root.join("backend")).unwrap();
    let outside = d.path().join("personal");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("notes.txt"), "PRIVATE").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("backend/link")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("linked-service")).unwrap();
    assert!(!approved(
        &root,
        &root.join("backend/link/notes.txt"),
        &["backend".into()]
    ));
    assert!(!approved(
        &root,
        &root.join("linked-service/notes.txt"),
        &["linked-service".into()]
    ));
}
#[cfg(unix)]
#[test]
fn rejects_service_alias_and_nested_repositories() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    std::fs::create_dir_all(root.join("personal")).unwrap();
    std::fs::create_dir_all(root.join("backend/vendor/.git")).unwrap();
    std::fs::write(root.join("backend/vendor/file"), "private nested repo").unwrap();
    std::os::unix::fs::symlink("personal", root.join("alias")).unwrap();
    assert!(!approved(root, &root.join("alias"), &["alias".into()]));
    assert!(!approved(
        root,
        &root.join("backend/vendor/file"),
        &["backend".into()]
    ));
}
