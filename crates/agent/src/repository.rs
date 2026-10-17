//! Bounded local-only Git observation. Author fields and file bytes never enter the wire request.
use crate::state::Config;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;
use wk_contracts::change_context::{
    valid_revision, valid_targets, ChangeContextRequest, CodeTarget,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryEnrollment {
    pub repository_id: uuid::Uuid,
    pub root: PathBuf,
    pub git_dirs: Vec<PathBuf>,
    pub binding_revision: i64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalObservation {
    pub request: ChangeContextRequest,
    pub target_digests: BTreeMap<String, String>,
    pub operation_digest: String,
    pub gaps: Vec<String>,
    // Hashed, unverified local identity candidates. Never included in request serialization.
    pub candidate_references: BTreeSet<String>,
}
pub fn same_snapshot(a: &LocalObservation, b: &LocalObservation) -> bool {
    a.request.repository_id == b.request.repository_id
        && a.request.base_revision == b.request.base_revision
        && a.request.targets == b.request.targets
        && a.request.operation_class == b.request.operation_class
        && a.target_digests == b.target_digests
        && a.operation_digest == b.operation_digest
        && a.gaps == b.gaps
}
fn metadata(e: &RepositoryEnrollment) -> Result<PathBuf, String> {
    if e.root
        .canonicalize()
        .map_err(|_| "repository unavailable")?
        != e.root
        || e.repository_id.is_nil()
        || e.binding_revision < 1
        || e.git_dirs.is_empty()
        || e.git_dirs.len() > 8
    {
        return Err("repository enrollment invalid".into());
    }
    let dot = e.root.join(".git");
    let m = std::fs::symlink_metadata(&dot).map_err(|_| "repository metadata unavailable")?;
    if m.file_type().is_symlink() {
        return Err("repository metadata symlink denied".into());
    }
    let gd = if m.is_dir() {
        dot
    } else if m.is_file() {
        let text = crate::state::read(&dot, 4096)?;
        let path = text
            .trim()
            .strip_prefix("gitdir: ")
            .ok_or("invalid Git directory")?;
        e.root
            .join(path)
            .canonicalize()
            .map_err(|_| "Git directory unavailable")?
    } else {
        return Err("invalid Git directory".into());
    };
    let approved = |p: &Path| -> Result<(), String> {
        if std::fs::symlink_metadata(p)
            .map_err(|_| "Git directory unavailable")?
            .file_type()
            .is_symlink()
            || !p.is_dir()
        {
            return Err("Git directory denied".into());
        }
        let real = p.canonicalize().map_err(|_| "Git directory unavailable")?;
        if !e.git_dirs.contains(&real) {
            return Err("Git directory not approved".into());
        }
        let mut pending: Vec<PathBuf> = [
            "objects",
            "refs",
            "HEAD",
            "config",
            "index",
            "packed-refs",
            "shallow",
            "commondir",
        ]
        .iter()
        .map(|k| real.join(k))
        .collect();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut count = 0usize;
        while let Some(path) = pending.pop() {
            count += 1;
            if count > 100000 || std::time::Instant::now() > deadline {
                return Err("Git metadata exceeded bound".into());
            }
            match std::fs::symlink_metadata(&path) {
                Ok(m) => {
                    if m.file_type().is_symlink() {
                        return Err("Git metadata symlink denied".into());
                    }
                    if m.is_dir() {
                        for entry in
                            std::fs::read_dir(path).map_err(|_| "Git metadata unavailable")?
                        {
                            pending.push(entry.map_err(|_| "Git metadata unavailable")?.path());
                        }
                    } else if !m.is_file() {
                        return Err("Git metadata not regular".into());
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("Git metadata unavailable".into()),
            }
        }
        if real.join("objects/info/alternates").exists() {
            return Err("alternate object storage not approved".into());
        }
        Ok(())
    };
    approved(&gd)?;
    if gd.join("commondir").exists() {
        let text = crate::state::read(&gd.join("commondir"), 4096)?;
        approved(
            &gd.join(text.trim())
                .canonicalize()
                .map_err(|_| "common Git directory unavailable")?,
        )?;
    }
    Ok(gd)
}
fn target_path(c: &Config, e: &RepositoryEnrollment, t: &CodeTarget) -> Result<PathBuf, String> {
    let mut path = e.root.clone();
    for part in t.path.split('/') {
        if part.eq_ignore_ascii_case(".git") {
            return Err("Git metadata target denied".into());
        }
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    return Err("target symlink denied".into());
                }
                if m.is_dir() && path.join(".git").exists() {
                    return Err("nested repository not approved".into());
                }
                #[cfg(any(target_os = "windows", target_os = "macos"))]
                if let Some(parent) = path.parent() {
                    for entry in std::fs::read_dir(parent).map_err(|_| "target unavailable")? {
                        let name = entry.map_err(|_| "target unavailable")?.file_name();
                        if name.to_string_lossy().eq_ignore_ascii_case(part) && name != part {
                            return Err("target case ambiguity".into());
                        }
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("target unavailable".into()),
        }
    }
    if !path.starts_with(c.root.join(&c.service)) {
        return Err("target outside approved service".into());
    }
    let b = crate::state::base_binding(c)?;
    let mut parent = path.as_path();
    while !parent.exists() {
        parent = parent.parent().ok_or("target outside workspace")?;
    }
    if !b.eligible(parent) {
        return Err("workspace approval absent or paused".into());
    }
    Ok(path)
}
async fn git(e: &RepositoryEnrollment, args: &[&str], limit: usize) -> Result<Vec<u8>, String> {
    let executable = if cfg!(windows) {
        "git.exe"
    } else {
        "/usr/bin/git"
    };
    let mut cmd = tokio::process::Command::new(executable);
    cmd.env_clear()
        .current_dir(&e.root)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if cfg!(windows) {
        for key in ["PATH", "SystemRoot"] {
            if let Some(v) = std::env::var_os(key) {
                cmd.env(key, v);
            }
        }
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([
            "--literal-pathspecs",
            "-c",
            "credential.helper=",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "diff.external=",
            "-c",
            "core.pager=cat",
        ])
        .args(args);
    let mut child = cmd.spawn().map_err(|_| "Git unavailable")?;
    let mut out = Vec::new();
    let pipe = child.stdout.take().ok_or("Git unavailable")?;
    pipe.take(limit as u64 + 1)
        .read_to_end(&mut out)
        .await
        .map_err(|_| "Git read failed")?;
    if out.len() > limit {
        child.kill().await.ok();
        return Err("Git output exceeded bound".into());
    }
    if !child.wait().await.map_err(|_| "Git unavailable")?.success() {
        return Err("Git observation failed".into());
    }
    Ok(out)
}
pub async fn observe(
    c: &Config,
    e: &RepositoryEnrollment,
    targets: &[CodeTarget],
    operation_class: &str,
    tool_input: &serde_json::Value,
) -> Result<LocalObservation, String> {
    tokio::time::timeout(
        Duration::from_secs(5),
        observe_inner(c, e, targets, operation_class, tool_input),
    )
    .await
    .map_err(|_| "repository observation timed out".to_string())?
}
async fn observe_inner(
    c: &Config,
    e: &RepositoryEnrollment,
    targets: &[CodeTarget],
    operation_class: &str,
    tool_input: &serde_json::Value,
) -> Result<LocalObservation, String> {
    if c.root != e.root || !valid_targets(targets) {
        return Err("invalid repository targets".into());
    }
    let mut case = BTreeSet::new();
    if targets.iter().any(|t| !case.insert(t.path.to_lowercase())) {
        return Err("ambiguous target paths".into());
    }
    let gd = metadata(e)?;
    let paths = targets
        .iter()
        .map(|t| target_path(c, e, t))
        .collect::<Result<Vec<_>, _>>()?;
    let head = String::from_utf8(git(e, &["rev-parse", "--verify", "HEAD^{commit}"], 128).await?)
        .map_err(|_| "invalid Git object")?
        .trim()
        .to_string();
    if !valid_revision(&head) {
        return Err("invalid Git object".into());
    }
    let top = String::from_utf8(git(e, &["rev-parse", "--show-toplevel"], 8192).await?)
        .map_err(|_| "invalid Git root")?;
    if Path::new(top.trim())
        .canonicalize()
        .map_err(|_| "Git root unavailable")?
        != e.root
    {
        return Err("Git root not approved".into());
    }
    let mut args = vec!["ls-files", "--stage", "-z", "--"];
    args.extend(targets.iter().map(|t| t.path.as_str()));
    let index = git(e, &args, 65536).await?;
    if index
        .split(|b| *b == 0)
        .any(|record| record.starts_with(b"160000 "))
    {
        return Err("submodule target not approved".into());
    }
    let mut statusargs = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--",
    ];
    statusargs.extend(targets.iter().map(|t| t.path.as_str()));
    let status = git(e, &statusargs, 65536).await?;
    let mut gaps = Vec::new();
    if !status.is_empty() {
        gaps.push("dirty_targets".into());
    }
    if gd.join("shallow").exists() || e.git_dirs.iter().any(|d| d.join("shallow").exists()) {
        gaps.push("shallow_history".into());
    }
    let input = serde_json::to_vec(tool_input).map_err(|_| "invalid operation input")?;
    if input.len() > 65536 {
        return Err("operation input exceeded bound".into());
    }
    let mut operation = Sha256::new();
    operation.update(e.binding_revision.to_le_bytes());
    operation.update(&input);
    let mut digests = BTreeMap::new();
    let mut references = BTreeSet::new();
    let mut total = 0;
    for (t, path) in targets.iter().zip(paths) {
        let mut hash = Sha256::new();
        hash.update(&index);
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() => {
                let mut bytes = Vec::new();
                std::fs::File::open(&path)
                    .map_err(|_| "target unavailable")?
                    .take(4 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "target unavailable")?;
                total += bytes.len();
                if bytes.len() > 4 * 1024 * 1024 || total > 16 * 1024 * 1024 {
                    return Err("target content exceeded bound".into());
                }
                hash.update(b"present");
                hash.update(bytes);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => hash.update(b"absent"),
            _ => return Err("nonregular target denied".into()),
        }
        digests.insert(t.path.clone(), format!("{:x}", hash.finalize()));
        // Blame is a candidate observation only; never assert rationale or ownership.
        let range = format!("{},{}", t.start.unwrap_or(1), t.end.unwrap_or(1));
        let output = git(
            e,
            &[
                "blame",
                "--line-porcelain",
                "--no-textconv",
                "-L",
                &range,
                &head,
                "--",
                &t.path,
            ],
            256 * 1024,
        )
        .await;
        match output {
            Ok(bytes) => {
                let mut author = Vec::new();
                for line in bytes.split(|b| *b == b'\n') {
                    if line.starts_with(b"author ") || line.starts_with(b"author-mail ") {
                        author.extend_from_slice(line);
                        author.push(0);
                    }
                }
                if !author.is_empty() {
                    references.insert(format!("local:{:x}", Sha256::digest(author)));
                }
            }
            Err(_) => {
                if !gaps.contains(&"provenance_unavailable".into()) {
                    gaps.push("provenance_unavailable".into());
                }
            }
        }
    }
    let after = git(e, &["rev-parse", "--verify", "HEAD^{commit}"], 128).await?;
    if after != format!("{head}\n").as_bytes() {
        return Err("HEAD changed during observation".into());
    }
    let request = ChangeContextRequest {
        schema_version: 1,
        repository_id: e.repository_id,
        base_revision: head,
        targets: targets.to_vec(),
        operation_class: operation_class.into(),
        operation_handle: None,
        budget_bytes: 16384,
    };
    request.validate()?;
    Ok(LocalObservation {
        request,
        target_digests: digests,
        operation_digest: format!("{:x}", operation.finalize()),
        gaps,
        candidate_references: references,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Config;
    use serde_json::json;
    fn git(root: &Path, args: &[&str]) {
        assert!(std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" }
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success());
    }
    fn fixture() -> (tempfile::TempDir, Config, RepositoryEnrollment) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("backend")).unwrap();
        std::fs::write(root.join("backend/retry.rs"), "PRIVATE_CODE_CANARY\n").unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["add", "backend/retry.rs"]);
        git(
            &root,
            &[
                "-c",
                "user.name=FORGED_AUTHOR_CANARY",
                "-c",
                "user.email=private@example.invalid",
                "commit",
                "-qm",
                "PRIVATE_MESSAGE_CANARY",
            ],
        );
        let binding =
            wk_scope::enrollment::Binding::approve(&root, "demo", vec!["backend".into()]).unwrap();
        let binding_file = root.join("binding.json");
        crate::state::save(&binding_file, &binding).unwrap();
        let c = Config {
            root: root.clone(),
            binding_file,
            service: "backend".into(),
            server: "http://127.0.0.1:9999".into(),
            model: "test".into(),
            inference: "http://127.0.0.1:11434/v1/chat/completions".into(),
            summarizer: crate::state::Summarizer::Local,
            port: 0,
            account: "test".into(),
            device_id: uuid::Uuid::new_v4().to_string(),
            device: None,
            repository: None,
        };
        let e = RepositoryEnrollment {
            repository_id: uuid::Uuid::new_v4(),
            root: root.clone(),
            git_dirs: vec![root.join(".git")],
            binding_revision: 1,
        };
        (dir, c, e)
    }
    fn targets() -> Vec<CodeTarget> {
        vec![CodeTarget {
            path: "backend/retry.rs".into(),
            start: None,
            end: None,
        }]
    }
    #[tokio::test]
    async fn observes_exact_head_local_changes_and_keeps_private_inputs_off_wire() {
        let (_dir, c, e) = fixture();
        let input = json!({"new_string":"PRIVATE_PROMPT_CANARY"});
        let first = observe(&c, &e, &targets(), "edit", &input).await.unwrap();
        let head = std::process::Command::new("git")
            .current_dir(&e.root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(
            first.request.base_revision,
            String::from_utf8(head.stdout).unwrap().trim()
        );
        std::fs::write(e.root.join("backend/retry.rs"), b"changed\0binary").unwrap();
        let changed = observe(&c, &e, &targets(), "edit", &input).await.unwrap();
        assert!(!same_snapshot(&first, &changed));
        assert!(changed.gaps.contains(&"dirty_targets".into()));
        git(&e.root, &["add", "backend/retry.rs"]);
        let staged = observe(&c, &e, &targets(), "edit", &input).await.unwrap();
        assert!(!same_snapshot(&changed, &staged));
        let wire = serde_json::to_string(&staged.request).unwrap();
        for canary in [
            "PRIVATE_CODE_CANARY",
            "PRIVATE_PROMPT_CANARY",
            "FORGED_AUTHOR_CANARY",
            "private@example.invalid",
            "PRIVATE_MESSAGE_CANARY",
            &e.root.to_string_lossy(),
            &staged.operation_digest,
        ] {
            assert!(!wire.contains(canary));
        }
        let another = observe(
            &c,
            &e,
            &targets(),
            "edit",
            &json!({"new_string":"different"}),
        )
        .await
        .unwrap();
        assert!(!same_snapshot(&staged, &another));
    }
    #[tokio::test]
    async fn supports_missing_targets_but_rejects_unapproved_metadata_and_unsafe_paths() {
        let (_dir, c, mut e) = fixture();
        let t = vec![CodeTarget {
            path: "backend/new.rs".into(),
            start: None,
            end: None,
        }];
        let absent = observe(&c, &e, &t, "write", &json!({})).await.unwrap();
        std::fs::write(e.root.join("backend/new.rs"), "new").unwrap();
        assert!(!same_snapshot(
            &absent,
            &observe(&c, &e, &t, "write", &json!({})).await.unwrap()
        ));
        for path in ["../secret", "-option", ".git/config", "frontend/no.rs"] {
            assert!(observe(
                &c,
                &e,
                &[CodeTarget {
                    path: path.into(),
                    start: None,
                    end: None
                }],
                "edit",
                &json!({})
            )
            .await
            .is_err());
        }
        e.git_dirs.clear();
        assert!(observe(&c, &e, &targets(), "edit", &json!({}))
            .await
            .is_err());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_symlink_escape_submodule_and_unapproved_worktree_common_directory() {
        let (_dir, c, e) = fixture();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), e.root.join("backend/link")).unwrap();
        assert!(observe(
            &c,
            &e,
            &[CodeTarget {
                path: "backend/link/new.rs".into(),
                start: None,
                end: None
            }],
            "write",
            &json!({})
        )
        .await
        .is_err());
        let worktree = outside.path().join("worktree");
        git(
            &e.root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "synthetic",
                worktree.to_str().unwrap(),
            ],
        );
        std::fs::remove_file(worktree.join("binding.json")).ok();
        let binding =
            wk_scope::enrollment::Binding::approve(&worktree, "demo", vec!["backend".into()])
                .unwrap();
        let mut wc = c.clone();
        wc.root = binding.root.clone();
        wc.binding_file = worktree.join("binding.json");
        crate::state::save(&wc.binding_file, &binding).unwrap();
        let mut we = e.clone();
        we.root = binding.root;
        we.git_dirs = vec![];
        assert!(observe(&wc, &we, &targets(), "edit", &json!({}))
            .await
            .is_err());
        std::fs::create_dir(e.root.join("backend/nested")).unwrap();
        git(&e.root.join("backend/nested"), &["init", "-q"]);
        assert!(observe(
            &c,
            &e,
            &[CodeTarget {
                path: "backend/nested/new.rs".into(),
                start: None,
                end: None
            }],
            "write",
            &json!({})
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn shallow_history_and_replacement_objects_never_change_head_claim() {
        let (_dir, c, e) = fixture();
        let first = observe(&c, &e, &targets(), "read_context", &json!({}))
            .await
            .unwrap();
        std::fs::write(
            e.root.join(".git/shallow"),
            format!("{}\n", first.request.base_revision),
        )
        .unwrap();
        let shallow = observe(&c, &e, &targets(), "read_context", &json!({}))
            .await
            .unwrap();
        assert!(shallow.gaps.contains(&"shallow_history".into()));
        std::fs::create_dir_all(e.root.join(".git/refs/replace")).unwrap();
        std::fs::write(
            e.root
                .join(".git/refs/replace")
                .join(&first.request.base_revision),
            "f".repeat(40),
        )
        .unwrap();
        let result = observe(&c, &e, &targets(), "read_context", &json!({}))
            .await
            .unwrap();
        assert_eq!(result.request.base_revision, first.request.base_revision);
        std::fs::write(e.root.join(".git/HEAD"), "not an object id\n").unwrap();
        assert!(observe(&c, &e, &targets(), "edit", &json!({}))
            .await
            .is_err());
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[tokio::test]
    async fn rejects_case_ambiguous_targets() {
        let (_dir, c, e) = fixture();
        let t = vec![
            CodeTarget {
                path: "backend/Retry.rs".into(),
                start: None,
                end: None,
            },
            CodeTarget {
                path: "backend/retry.rs".into(),
                start: None,
                end: None,
            },
        ];
        assert!(observe(&c, &e, &t, "multi_edit", &json!({})).await.is_err());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn approved_git_directory_does_not_approve_external_object_symlinks() {
        let (_dir, c, e) = fixture();
        let outside = tempfile::tempdir().unwrap();
        std::fs::rename(e.root.join(".git/objects"), outside.path().join("objects")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("objects"), e.root.join(".git/objects"))
            .unwrap();
        assert!(observe(&c, &e, &targets(), "read_context", &json!({}))
            .await
            .is_err());
    }
}
