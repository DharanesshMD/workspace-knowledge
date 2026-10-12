use crate::{cursor_source::SourceBinding, native_enrollment::Enrollment};
use serde_json::Value;
use std::path::Path;

pub use crate::cursor_source::read_bound;
pub fn bind(e: &Enrollment, p: &Value, now: i64) -> Result<SourceBinding, String> {
    let home = std::env::var_os("HOME").ok_or("home_unavailable")?;
    bind_at(
        e,
        p,
        now,
        &std::path::PathBuf::from(home).join(".gemini/antigravity/brain"),
    )
}
fn bind_at(e: &Enrollment, p: &Value, now: i64, root: &Path) -> Result<SourceBinding, String> {
    if p["workspacePaths"] != serde_json::json!([e.canonical_workspace]) {
        return Err("workspace_mismatch".into());
    }
    let id = p["conversationId"].as_str().ok_or("conversation_missing")?;
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err("conversation_invalid".into());
    }
    let expected = root
        .join(id)
        .join(".system_generated/logs/transcript_full.jsonl");
    if p["transcriptPath"].as_str() != expected.to_str() {
        return Err("source_path_mismatch".into());
    }
    let file = crate::cursor_source::open_owned(&expected, false)?;
    let (device, inode, size) = crate::cursor_source::identity(&file)?;
    if size > 4 * 1024 * 1024 {
        return Err("source_too_large".into());
    }
    Ok(SourceBinding {
        conversation_id: id.into(),
        path: expected,
        device,
        inode,
        first_seen: now,
        expires_at: now.checked_add(86400).ok_or("invalid_expiry")?,
        policy_hash: e.policy_hash.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wk_adapters::native_evidence::Client;
    #[test]
    fn source_is_bound_to_one_conversation_and_replacement_is_denied() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let e = Enrollment {
            client: Client::Antigravity,
            version: "2.19.1".into(),
            surface: "desktop".into(),
            device_id: "d".into(),
            canonical_workspace: root.clone(),
            service: "app".into(),
            policy_hash: "p".into(),
            enabled: true,
        };
        let path = root.join("chat/.system_generated/logs/transcript_full.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{}\n").unwrap();
        let payload = serde_json::json!({"conversationId":"chat","workspacePaths":[root],"transcriptPath":path});
        if cfg!(not(unix)) {
            assert_eq!(
                bind_at(&e, &payload, 10, &root).unwrap_err(),
                "native_platform_unsupported"
            );
            return;
        }
        let source = bind_at(&e, &payload, 10, &root).unwrap();
        assert_eq!(read_bound(&source, 11).unwrap(), b"{}\n");
        assert_eq!(
            read_bound(&source, source.expires_at).unwrap_err(),
            "source_expired"
        );
        let moved = root.join("old");
        std::fs::rename(&path, &moved).unwrap();
        std::fs::write(&path, b"new").unwrap();
        assert!(read_bound(&source, 11).is_err());
        let mut bad = payload.clone();
        bad["workspacePaths"] = serde_json::json!([root, "/"]);
        assert!(bind_at(&e, &bad, 10, &root).is_err());
        bad = payload;
        bad["conversationId"] = serde_json::json!("../chat");
        assert!(bind_at(&e, &bad, 10, &root).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_and_oversized_source_are_denied() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let file = root.join("file");
        std::fs::write(&file, b"x").unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(crate::cursor_source::open_owned(&link, false).is_err());
        let f = crate::cursor_source::open_owned(&file, false).unwrap();
        let (dev, ino, _) = crate::cursor_source::identity(&f).unwrap();
        let source = SourceBinding {
            conversation_id: "chat".into(),
            path: file.clone(),
            device: dev,
            inode: ino,
            first_seen: 0,
            expires_at: 100,
            policy_hash: "p".into(),
        };
        std::fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_len(4 * 1024 * 1024 + 1)
            .unwrap();
        assert!(read_bound(&source, 1).is_err());
    }
}
