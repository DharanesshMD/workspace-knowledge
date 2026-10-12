//! Opt-in, version-pinned access to one callback-bound Cursor transcript.
use crate::state::{self, Config};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
pub const VERSION: &str = "3.23.23";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorConsent {
    pub policy_hash: String,
    pub version: String,
    pub surface: String,
    pub transcript_root: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceBinding {
    pub conversation_id: String,
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
    pub first_seen: i64,
    pub expires_at: i64,
    pub policy_hash: String,
}
pub fn installed_version() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("native_platform_unsupported".into());
    }
    let v: Value = serde_json::from_str(&state::read(
        Path::new("/Applications/Cursor.app/Contents/Resources/app/package.json"),
        1024 * 1024,
    )?)
    .map_err(|_| "cursor_version_unavailable")?;
    if v["version"] != VERSION {
        return Err("cursor_version_unverified".into());
    }
    Ok(())
}
pub fn load(dir: &Path) -> Result<CursorConsent, String> {
    installed_version()?;
    serde_json::from_str(&state::read(&dir.join("cursor-consent.json"), 65536)?)
        .map_err(|_| "native_consent_unavailable".into())
}
pub fn enroll(dir: &Path) -> Result<(), String> {
    installed_version()?;
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let home = std::env::var_os("HOME").ok_or("home_unavailable")?;
    let root = PathBuf::from(home).join(".cursor/projects");
    // Validate the directory using the same descriptor walk as actual reads.
    let _ = open_owned(&root, true)?;
    state::save(
        &dir.join("cursor-consent.json"),
        &CursorConsent {
            policy_hash: b.policy_hash,
            version: VERSION.into(),
            surface: "agents-local".into(),
            transcript_root: root,
        },
    )
}
fn slug(root: &Path) -> String {
    root.to_string_lossy()
        .trim_start_matches('/')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}
pub fn validate_hook(c: &Config, consent: &CursorConsent, p: &Value) -> Result<String, String> {
    let b = state::binding(c)?;
    if consent.policy_hash != b.policy_hash
        || consent.version != VERSION
        || consent.surface != "agents-local"
    {
        return Err("native_consent_changed".into());
    }
    if p.get("cursor_version").is_some_and(|v| v != VERSION) {
        return Err("cursor_version_unverified".into());
    }
    let roots = p["workspace_roots"].as_array().ok_or("workspace_missing")?;
    if roots.len() != 1 {
        return Err("workspace_ambiguous".into());
    }
    let root = PathBuf::from(roots[0].as_str().ok_or("workspace_missing")?);
    if root != c.root && root != c.root.join(&c.service) {
        return Err("workspace_mismatch".into());
    }
    let id = p["conversation_id"]
        .as_str()
        .ok_or("conversation_missing")?;
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("conversation_invalid".into());
    }
    Ok(id.into())
}
pub fn bind(
    c: &Config,
    consent: &CursorConsent,
    p: &Value,
    now: i64,
) -> Result<Option<SourceBinding>, String> {
    let id = validate_hook(c, consent, p)?;
    let Some(raw) = p["transcript_path"].as_str() else {
        return Ok(None);
    };
    let root = Path::new(
        p["workspace_roots"][0]
            .as_str()
            .ok_or("workspace_missing")?,
    );
    let expected = consent
        .transcript_root
        .join(slug(root))
        .join("agent-transcripts")
        .join(&id)
        .join(format!("{id}.jsonl"));
    let path = PathBuf::from(raw);
    if path != expected
        || path.components().any(|c| {
            !matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err("source_path_mismatch".into());
    }
    let f = open_owned(&path, false)?;
    let (device, inode, _) = identity(&f)?;
    Ok(Some(SourceBinding {
        conversation_id: id,
        path,
        device,
        inode,
        first_seen: now,
        expires_at: now.checked_add(86400).ok_or("invalid_expiry")?,
        policy_hash: consent.policy_hash.clone(),
    }))
}
#[cfg(unix)]
fn owned(uid: u32, current: u32) -> bool {
    uid == current
}
#[cfg(unix)]
pub(crate) fn identity(f: &std::fs::File) -> Result<(u64, u64, u64), String> {
    use std::os::unix::fs::MetadataExt;
    let m = f.metadata().map_err(|_| "source_unavailable")?;
    if !m.is_file() || !owned(m.uid(), unsafe { libc::geteuid() }) {
        return Err("source_type_or_owner".into());
    }
    Ok((m.dev(), m.ino(), m.len()))
}
#[cfg(not(unix))]
pub(crate) fn identity(_: &std::fs::File) -> Result<(u64, u64, u64), String> {
    Err("native_platform_unsupported".into())
}
#[cfg(unix)]
pub(crate) fn open_owned(path: &Path, directory: bool) -> Result<std::fs::File, String> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    };
    if !cfg!(target_os = "macos") && !cfg!(test) {
        return Err("native_platform_unsupported".into());
    }
    if !path.is_absolute() {
        return Err("source_path_invalid".into());
    }
    let mut current = std::fs::File::open("/").map_err(|_| "source_unavailable")?;
    let parts = path.components().skip(1).collect::<Vec<_>>();
    for (i, part) in parts.iter().enumerate() {
        let std::path::Component::Normal(name) = part else {
            return Err("source_path_invalid".into());
        };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| "source_path_invalid")?;
        let is_dir = i + 1 < parts.len() || directory;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if is_dir { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err("source_unavailable".into());
        }
        current = unsafe { std::fs::File::from_raw_fd(fd) };
        let m = current.metadata().map_err(|_| "source_unavailable")?;
        // Root-owned OS ancestors are permitted; all user-owned descendants must
        // remain owned by this user. A shared writable ancestor is not trusted.
        if m.uid() != 0 && !owned(m.uid(), unsafe { libc::geteuid() }) {
            return Err("source_owner".into());
        }
        if m.mode() & 0o022 != 0 && m.mode() & 0o1000 == 0 {
            return Err("source_writable_ancestor".into());
        }
    }
    let m = current.metadata().map_err(|_| "source_unavailable")?;
    if !owned(m.uid(), unsafe { libc::geteuid() })
        || (directory && !m.is_dir())
        || (!directory && !m.is_file())
    {
        return Err("source_type_or_owner".into());
    }
    Ok(current)
}
#[cfg(not(unix))]
pub(crate) fn open_owned(_: &Path, _: bool) -> Result<std::fs::File, String> {
    Err("native_platform_unsupported".into())
}
pub fn read_bound(source: &SourceBinding, now: i64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if now >= source.expires_at {
        return Err("source_expired".into());
    }
    let mut file = open_owned(&source.path, false)?;
    let before = identity(&file)?;
    if (before.0, before.1) != (source.device, source.inode) {
        return Err("source_replaced".into());
    }
    if before.2 > 4 * 1024 * 1024 {
        return Err("source_too_large".into());
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "source_unavailable")?;
    let after = identity(&file)?;
    if before != after || bytes.len() as u64 != after.2 {
        return Err("source_changed_during_read".into());
    }
    let current = open_owned(&source.path, false)?;
    if identity(&current)? != after {
        return Err("source_replaced".into());
    }
    Ok(bytes)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (
        tempfile::TempDir,
        crate::state::Config,
        CursorConsent,
        serde_json::Value,
    ) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap().join("workspace");
        std::fs::create_dir_all(root.join("frontend")).unwrap();
        let binding =
            wk_scope::enrollment::Binding::approve(&root, "demo", vec!["frontend".into()]).unwrap();
        let binding_file = d.path().join("binding.json");
        crate::state::save(&binding_file, &binding).unwrap();
        let c = crate::state::Config {
            root: root.clone(),
            binding_file,
            service: "frontend".into(),
            server: "http://127.0.0.1:1".into(),
            model: "synthetic".into(),
            inference: "http://127.0.0.1:1".into(),
            port: 1,
            account: "test".into(),
            device_id: "test".into(),

            repository: None,

            device: None,
        };
        let consent = CursorConsent {
            policy_hash: binding.policy_hash,
            version: VERSION.into(),
            surface: "agents-local".into(),
            transcript_root: d.path().canonicalize().unwrap().join("projects"),
        };
        let source = consent
            .transcript_root
            .join(slug(&root))
            .join("agent-transcripts/test-id/test-id.jsonl");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"synthetic\n").unwrap();
        let payload = json!({"conversation_id":"test-id","workspace_roots":[root],"cursor_version":VERSION,"transcript_path":source});
        (d, c, consent, payload)
    }
    #[test]
    fn expired_source_is_not_opened() {
        let s = SourceBinding {
            conversation_id: "test".into(),
            path: "/nonexistent/do-not-open".into(),
            device: 1,
            inode: 1,
            first_seen: 1,
            expires_at: 2,
            policy_hash: "p".into(),
        };
        assert_eq!(read_bound(&s, 2).unwrap_err(), "source_expired");
    }
    #[test]
    fn binding_requires_enrollment_matching_workspace_and_session() {
        let (_d, c, consent, p) = fixture();
        assert!(bind(&c, &consent, &p, 100).unwrap().is_some());
        let mut q = p.clone();
        q["transcript_path"] = serde_json::Value::Null;
        assert!(bind(&c, &consent, &q, 100).unwrap().is_none());
        for (field, value) in [
            ("conversation_id", json!("../bad")),
            ("cursor_version", json!("0.1")),
            ("workspace_roots", json!([c.root, "/other"])),
            ("transcript_path", json!("/outside/test-id.jsonl")),
        ] {
            let mut q = p.clone();
            q[field] = value;
            assert!(bind(&c, &consent, &q, 100).is_err(), "{field}");
        }
        let mut wrong = consent.clone();
        wrong.policy_hash = "changed".into();
        assert!(bind(&c, &wrong, &p, 100).is_err());
        let mut q = p.clone();
        q["transcript_path"] = json!(p["transcript_path"]
            .as_str()
            .unwrap()
            .replace("test-id.jsonl", "other.jsonl"));
        assert!(bind(&c, &consent, &q, 100).is_err());
    }
    #[test]
    fn source_replacement_and_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;
        let (_d, c, consent, p) = fixture();
        let source = bind(&c, &consent, &p, 100).unwrap().unwrap();
        assert_eq!(read_bound(&source, 101).unwrap(), b"synthetic\n");
        let moved = source.path.with_extension("old");
        std::fs::rename(&source.path, &moved).unwrap();
        std::fs::write(&source.path, b"replacement").unwrap();
        assert!(read_bound(&source, 101).is_err());
        std::fs::remove_file(&source.path).unwrap();
        symlink(&moved, &source.path).unwrap();
        assert!(bind(&c, &consent, &p, 100).is_err());
        std::fs::remove_file(&source.path).unwrap();
        std::fs::create_dir(&source.path).unwrap();
        assert!(bind(&c, &consent, &p, 100).is_err());
        std::fs::remove_dir(&source.path).unwrap();
        std::fs::rename(&moved, &source.path).unwrap();
        let parent = source.path.parent().unwrap();
        let other = parent.with_extension("saved");
        std::fs::rename(parent, &other).unwrap();
        symlink(&other, parent).unwrap();
        assert!(bind(&c, &consent, &p, 100).is_err());
    }
    #[test]
    fn file_owner_must_match_current_user() {
        assert!(!owned(99, 100));
        assert!(owned(100, 100));
    }
}
