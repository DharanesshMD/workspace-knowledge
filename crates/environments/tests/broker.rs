use wk_environments::{import, process, profiles::Profile};
fn profile() -> Profile {
    Profile {
        service: "backend".into(),
        profile: "development".into(),
        mount: "secret".into(),
        path: "team/backend/development".into(),
        address: "https://localhost:8200".into(),
        workspace: std::env::temp_dir(),
        keys: ["PASSWORD".into()].into(),
    }
}
#[test]
fn denies_production_and_path_confusion() {
    let mut p = profile();
    p.profile = "production".into();
    assert!(p.validate().is_err());
    let mut p = profile();
    p.path = "team/../backend/development".into();
    assert!(p.validate().is_err());
    let mut p = profile();
    p.address = "http://localhost:8200".into();
    assert!(p.validate().is_err());
}
#[test]
fn import_needs_explicit_consent_and_literal_keys() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(".env");
    std::fs::write(&p, "PASSWORD=synthetic").unwrap();
    assert!(import::read(d.path(), &p, false).is_err());
    assert!(import::parse("PASSWORD=x\nPASSWORD=y").is_err());
    assert_eq!(
        import::parse("PASSWORD=$(echo no-shell)").unwrap()["PASSWORD"],
        "$(echo no-shell)"
    );
}
#[test]
fn echo_redaction_handles_overlap() {
    let values = [("A".into(), "abc".into()), ("B".into(), "abcdef".into())].into();
    assert_eq!(
        process::redact(b"abcdef abc".to_vec(), &values),
        b"[REDACTED] [REDACTED]"
    );
}
#[test]
fn schema_has_names_no_values() {
    let schema = profile().schema().unwrap();
    assert_eq!(schema["keys"][0], "PASSWORD");
    assert!(schema.get("token").is_none());
}
#[cfg(unix)]
#[test]
fn rejects_symlink_escape() {
    use std::os::unix::fs::symlink;
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    symlink(b.path(), a.path().join("linked")).unwrap();
    assert!(import::confined(a.path(), &a.path().join("linked/file"), false).is_err());
}
#[cfg(unix)]
#[test]
fn shell_metacharacters_are_not_interpolated() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("should-not-exist");
    let arg = format!("$(touch {})", file.display());
    let values = Default::default();
    assert_eq!(
        process::run(d.path(), "/usr/bin/printf", &["%s".into(), arg], &values).unwrap(),
        0
    );
    assert!(!file.exists());
}
