//! Finds and validates agent memory files. Reads only the agent's approved
//! service folder and this project's Claude Code memory folder.

use crate::memory_split::SourceKind;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use wk_scope::enrollment::Binding;

pub const INSTRUCTION_FILES: [&str; 4] = [
    "CLAUDE.md",
    ".claude/CLAUDE.md",
    "AGENTS.md",
    ".github/copilot-instructions.md",
];
/// VS Code Copilot path-specific instructions: `*.instructions.md` files
/// directly in this folder (not its subfolders).
const SCOPED_DIR: &str = ".github/instructions";
const SCOPED_SUFFIX: &str = ".instructions.md";
pub const MAX_FILE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub label: String,
    pub path: PathBuf,
    pub kind: SourceKind,
    pub rejected: Option<&'static str>,
}
/// Claude Code's project folder name: every character other than ASCII
/// letters, digits and `-` becomes `-`. Lossy, so it is only a suggestion.
pub fn encode_project(root: &Path) -> String {
    root.to_string_lossy()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}
pub fn notes_dir(home: &Path, root: &Path) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(encode_project(root))
        .join("memory")
}
fn check(path: &Path) -> Option<&'static str> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Some("unavailable");
    };
    if meta.file_type().is_symlink() {
        return Some("symlink");
    }
    if !meta.is_file() {
        return Some("not_a_file");
    }
    (meta.len() > MAX_FILE_BYTES).then_some("too_large")
}
/// This project's notes folder, only if no folder below the home folder is a
/// symlink (a symlink could point anywhere). The home folder itself may be a
/// symlink, but a dotfiles-symlinked `~/.claude` makes notes unavailable.
fn real_notes_dir(home: &Path, root: &Path) -> Option<PathBuf> {
    let expected = notes_dir(&home.canonicalize().ok()?, root);
    (expected.canonicalize().ok()? == expected).then_some(expected)
}
fn note_name(name: &str) -> bool {
    name.ends_with(".md") && name != "MEMORY.md"
}
fn control_chars(name: &str) -> bool {
    name.chars().any(char::is_control)
}
/// Copilot `*.instructions.md` names directly in `root/folder/.github/instructions`.
/// A symlinked folder on the way could point anywhere, so it is never listed.
fn scoped_names(root: &Path, folder: &str) -> BTreeSet<String> {
    let mut dir = root.to_path_buf();
    for part in Path::new(folder).join(SCOPED_DIR).components() {
        dir.push(part);
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
            return BTreeSet::new();
        }
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name.len() > SCOPED_SUFFIX.len()
                && name.ends_with(SCOPED_SUFFIX)
                && !control_chars(name)
        })
        .collect()
}
/// Instruction files in the agent's service folder. Workspace-root files are
/// listed as rejected so `scan` can explain why they are not imported.
pub fn repository(b: &Binding, service: &str) -> Vec<Candidate> {
    if !b.services.iter().any(|s| s == service) {
        return vec![];
    }
    let mut out = Vec::new();
    let mut add = |name: &str, kind: SourceKind, at_root: bool, in_service: bool| {
        if at_root {
            out.push(Candidate {
                label: format!("repo:{name}"),
                path: b.root.join(name),
                kind,
                rejected: Some("outside_approved_service"),
            });
        }
        if in_service {
            let path = b.root.join(service).join(name);
            let rejected =
                check(&path).or_else(|| (!b.eligible(&path)).then_some("outside_approved_service"));
            out.push(Candidate {
                label: format!("repo:{service}/{name}"),
                path,
                kind,
                rejected,
            });
        }
    };
    let present = |path: PathBuf| std::fs::symlink_metadata(path).is_ok();
    for name in INSTRUCTION_FILES {
        add(
            name,
            SourceKind::Instructions,
            present(b.root.join(name)),
            present(b.root.join(service).join(name)),
        );
    }
    let (at_root, in_service) = (scoped_names(&b.root, ""), scoped_names(&b.root, service));
    for name in at_root.union(&in_service) {
        add(
            &format!("{SCOPED_DIR}/{name}"),
            SourceKind::ScopedInstructions,
            at_root.contains(name),
            in_service.contains(name),
        );
    }
    out
}
/// This project's Claude Code notes. Suggestions only: each needs `--file`.
pub fn notes(home: &Path, root: &Path) -> Vec<Candidate> {
    let Some(dir) = real_notes_dir(home, root) else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut out: Vec<Candidate> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            (note_name(&name) && !control_chars(&name)).then(|| {
                let path = dir.join(&name);
                Candidate {
                    label: format!("claude-memory:{name}"),
                    rejected: check(&path),
                    path,
                    kind: SourceKind::Note,
                }
            })
        })
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}
/// Approve one explicitly named file, failing closed with a fixed reason.
pub fn approve_file(
    home: &Path,
    b: &Binding,
    service: &str,
    path: &Path,
) -> Result<Candidate, &'static str> {
    if !path.is_absolute() {
        return Err("path_not_absolute");
    }
    if let Some(reason) = check(path) {
        return Err(reason);
    }
    // Decided by resolved location: check() refused a symlinked final name,
    // eligible() and real_notes_dir() refuse symlinked folders.
    let canonical = path.canonicalize().map_err(|_| "unavailable")?;
    let name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("not_a_memory_source")?;
    if control_chars(name) {
        return Err("invalid_name");
    }
    if let Some(found) = repository(b, service)
        .into_iter()
        .find(|c| c.rejected.is_none() && c.path.canonicalize().is_ok_and(|p| p == canonical))
    {
        return Ok(found);
    }
    let notes = real_notes_dir(home, &b.root).ok_or("not_a_memory_source")?;
    if canonical.parent() != Some(notes.as_path()) || !note_name(name) {
        return Err("not_a_memory_source");
    }
    Ok(Candidate {
        label: format!("claude-memory:{name}"),
        path: notes.join(name),
        kind: SourceKind::Note,
        rejected: None,
    })
}
/// Reads through one handle that never follows a symlink, never waits on a
/// FIFO and never reads more than one byte past the limit. `O_NOFOLLOW`
/// protects only the final name; parent folders are validated at approval.
pub fn read(c: &Candidate) -> Result<String, &'static str> {
    use std::io::Read;
    if let Some(reason) = check(&c.path) {
        return Err(reason);
    }
    let file = open_no_follow(&c.path)?;
    if !file.metadata().map_err(|_| "unavailable")?.is_file() {
        return Err("not_a_file");
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "unavailable")?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("too_large");
    }
    String::from_utf8(bytes).map_err(|_| "not_utf8")
}
#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<std::fs::File, &'static str> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP) => "symlink",
            _ => "unavailable",
        })
}
#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> Result<std::fs::File, &'static str> {
    std::fs::File::open(path).map_err(|_| "unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PathBuf, Binding) {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("project");
        std::fs::create_dir_all(root.join("backend/.claude")).unwrap();
        let b = Binding::approve(&root, "demo", vec!["backend".into()]).unwrap();
        let home = base.join("home");
        std::fs::create_dir_all(notes_dir(&home, &b.root)).unwrap();
        (temp, home, b)
    }
    #[test]
    fn encodes_project_folder_like_claude_code() {
        assert_eq!(
            encode_project(Path::new("/Users/dev/.claude")),
            "-Users-dev--claude"
        );
        assert_eq!(encode_project(Path::new("/w/my_app.v2")), "-w-my-app-v2");
    }
    #[test]
    fn repository_reads_service_files_and_rejects_root_files() {
        let (_t, _home, b) = fixture();
        std::fs::write(b.root.join("CLAUDE.md"), "root").unwrap();
        std::fs::write(b.root.join("backend/CLAUDE.md"), "service").unwrap();
        std::fs::write(b.root.join("backend/.claude/CLAUDE.md"), "service").unwrap();
        let found = repository(&b, "backend");
        let summary: Vec<_> = found
            .iter()
            .map(|c| (c.label.as_str(), c.rejected))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("repo:CLAUDE.md", Some("outside_approved_service")),
                ("repo:backend/CLAUDE.md", None),
                ("repo:backend/.claude/CLAUDE.md", None)
            ]
        );
    }
    #[test]
    fn repository_lists_copilot_instruction_files_sorted_and_non_recursive() {
        let (_t, home, b) = fixture();
        let scoped = b.root.join("backend/.github/instructions");
        std::fs::create_dir_all(scoped.join("nested")).unwrap();
        std::fs::write(b.root.join("backend/.github/copilot-instructions.md"), "s").unwrap();
        std::fs::write(scoped.join("rust.instructions.md"), "s").unwrap();
        std::fs::write(scoped.join("api.instructions.md"), "s").unwrap();
        std::fs::write(scoped.join("notes.md"), "s").unwrap();
        std::fs::write(scoped.join(".instructions.md"), "s").unwrap();
        std::fs::write(scoped.join("nested/deep.instructions.md"), "s").unwrap();
        std::fs::create_dir_all(b.root.join(".github/instructions")).unwrap();
        std::fs::write(b.root.join(".github/copilot-instructions.md"), "r").unwrap();
        std::fs::write(
            b.root.join(".github/instructions/root.instructions.md"),
            "r",
        )
        .unwrap();
        let summary: Vec<_> = repository(&b, "backend")
            .into_iter()
            .map(|c| (c.label, c.kind, c.rejected))
            .collect();
        let (i, s) = (SourceKind::Instructions, SourceKind::ScopedInstructions);
        let out = Some("outside_approved_service");
        assert_eq!(
            summary,
            vec![
                ("repo:.github/copilot-instructions.md".into(), i, out),
                (
                    "repo:backend/.github/copilot-instructions.md".into(),
                    i,
                    None
                ),
                (
                    "repo:backend/.github/instructions/api.instructions.md".into(),
                    s,
                    None
                ),
                (
                    "repo:.github/instructions/root.instructions.md".into(),
                    s,
                    out
                ),
                (
                    "repo:backend/.github/instructions/rust.instructions.md".into(),
                    s,
                    None
                ),
            ]
        );
        let found =
            approve_file(&home, &b, "backend", &scoped.join("api.instructions.md")).unwrap();
        assert_eq!(
            (found.label.as_str(), found.kind),
            ("repo:backend/.github/instructions/api.instructions.md", s)
        );
        assert_eq!(
            approve_file(
                &home,
                &b,
                "backend",
                &scoped.join("nested/deep.instructions.md")
            ),
            Err("not_a_memory_source")
        );
        assert_eq!(
            approve_file(
                &home,
                &b,
                "backend",
                &b.root.join(".github/instructions/root.instructions.md")
            ),
            Err("not_a_memory_source")
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_copilot_instruction_files_and_folders_are_refused() {
        let (_t, home, b) = fixture();
        let elsewhere = b.root.parent().unwrap().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("x.instructions.md"), "secret").unwrap();
        let github = b.root.join("backend/.github");
        std::fs::create_dir_all(&github).unwrap();
        std::os::unix::fs::symlink(&elsewhere, github.join("instructions")).unwrap();
        assert!(repository(&b, "backend").is_empty());
        std::fs::remove_file(github.join("instructions")).unwrap();
        std::fs::create_dir(github.join("instructions")).unwrap();
        let link = github.join("instructions/link.instructions.md");
        std::os::unix::fs::symlink(elsewhere.join("x.instructions.md"), &link).unwrap();
        std::fs::write(github.join("instructions/bad\nname.instructions.md"), "s").unwrap();
        let summary: Vec<_> = repository(&b, "backend")
            .into_iter()
            .map(|c| (c.label, c.rejected))
            .collect();
        assert_eq!(
            summary,
            vec![(
                "repo:backend/.github/instructions/link.instructions.md".into(),
                Some("symlink")
            )]
        );
        assert_eq!(approve_file(&home, &b, "backend", &link), Err("symlink"));
    }
    #[test]
    fn notes_skip_the_index_and_explicit_files_must_be_this_projects_notes() {
        let (_t, home, b) = fixture();
        let dir = notes_dir(&home, &b.root);
        std::fs::write(dir.join("MEMORY.md"), "- index").unwrap();
        std::fs::write(dir.join("retries.md"), "note").unwrap();
        let labels: Vec<_> = notes(&home, &b.root).into_iter().map(|c| c.label).collect();
        assert_eq!(labels, vec!["claude-memory:retries.md"]);
        assert_eq!(
            approve_file(&home, &b, "backend", &dir.join("retries.md"))
                .unwrap()
                .kind,
            SourceKind::Note
        );
        assert_eq!(
            approve_file(&home, &b, "backend", &dir.join("MEMORY.md")),
            Err("not_a_memory_source")
        );
        std::fs::write(home.join(".claude/CLAUDE.md"), "global").unwrap();
        assert_eq!(
            approve_file(&home, &b, "backend", &home.join(".claude/CLAUDE.md")),
            Err("not_a_memory_source")
        );
        let other = home.join(".claude/projects/-other/memory");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("x.md"), "other").unwrap();
        assert_eq!(
            approve_file(&home, &b, "backend", &other.join("x.md")),
            Err("not_a_memory_source")
        );
    }
    #[test]
    fn root_instruction_files_cannot_be_approved_explicitly() {
        let (_t, home, b) = fixture();
        std::fs::write(b.root.join("AGENTS.md"), "root").unwrap();
        assert_eq!(
            approve_file(&home, &b, "backend", &b.root.join("AGENTS.md")),
            Err("not_a_memory_source")
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_large_files_and_relative_paths_are_rejected() {
        let (_t, home, b) = fixture();
        let dir = notes_dir(&home, &b.root);
        std::fs::write(dir.join("real.md"), "note").unwrap();
        std::os::unix::fs::symlink(dir.join("real.md"), dir.join("link.md")).unwrap();
        assert_eq!(
            approve_file(&home, &b, "backend", &dir.join("link.md")),
            Err("symlink")
        );
        std::fs::write(dir.join("big.md"), "x".repeat(MAX_FILE_BYTES as usize + 1)).unwrap();
        assert_eq!(
            approve_file(&home, &b, "backend", &dir.join("big.md")),
            Err("too_large")
        );
        assert_eq!(
            approve_file(&home, &b, "backend", Path::new("real.md")),
            Err("path_not_absolute")
        );
    }
    #[test]
    fn read_rejects_non_utf8() {
        let (_t, home, b) = fixture();
        let path = notes_dir(&home, &b.root).join("bad.md");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        let candidate = approve_file(&home, &b, "backend", &path).unwrap();
        assert_eq!(read(&candidate), Err("not_utf8"));
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_notes_folders_are_refused() {
        let (_t, home, b) = fixture();
        let dir = notes_dir(&home, &b.root);
        let target = home.join("elsewhere");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("x.md"), "secret").unwrap();
        std::fs::remove_dir(&dir).unwrap();
        std::os::unix::fs::symlink(&target, &dir).unwrap();
        assert!(notes(&home, &b.root).is_empty());
        assert_eq!(
            approve_file(&home, &b, "backend", &dir.join("x.md")),
            Err("not_a_memory_source")
        );
        assert_eq!(
            approve_file(&home, &b, "backend", &target.join("x.md")),
            Err("not_a_memory_source")
        );
    }
    #[cfg(unix)]
    #[test]
    fn read_refuses_symlinked_files() {
        let (_t, home, b) = fixture();
        let dir = notes_dir(&home, &b.root);
        std::fs::write(dir.join("real.md"), "note").unwrap();
        std::os::unix::fs::symlink(dir.join("real.md"), dir.join("link.md")).unwrap();
        let candidate = Candidate {
            label: "claude-memory:link.md".into(),
            path: dir.join("link.md"),
            kind: SourceKind::Note,
            rejected: None,
        };
        assert_eq!(read(&candidate), Err("symlink"));
        assert_eq!(open_no_follow(&dir.join("link.md")).err(), Some("symlink"));
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_above_the_notes_folder_are_refused() {
        let (_t, home, b) = fixture();
        let project = notes_dir(&home, &b.root).parent().unwrap().to_path_buf();
        let target = home.join("elsewhere");
        std::fs::create_dir_all(target.join("memory")).unwrap();
        std::fs::write(target.join("memory/x.md"), "secret").unwrap();
        std::fs::remove_dir_all(&project).unwrap();
        std::os::unix::fs::symlink(&target, &project).unwrap();
        assert!(notes(&home, &b.root).is_empty());
        assert_eq!(
            approve_file(&home, &b, "backend", &project.join("memory/x.md")),
            Err("not_a_memory_source")
        );
        assert_eq!(
            approve_file(&home, &b, "backend", &target.join("memory/x.md")),
            Err("not_a_memory_source")
        );
    }
    // Unix only: the alpha supports macOS/Linux developers, and Windows verbatim paths differ.
    #[cfg(unix)]
    #[test]
    fn non_canonical_spellings_resolve_to_the_canonical_note() {
        let (_t, home, b) = fixture();
        let dir = notes_dir(&home, &b.root);
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("retries.md"), "note").unwrap();
        let found = approve_file(&home, &b, "backend", &dir.join("sub/../retries.md")).unwrap();
        assert_eq!(found.path, dir.join("retries.md"));
        assert_eq!(found.label, "claude-memory:retries.md");
    }
    #[cfg(unix)]
    #[test]
    fn fifos_are_rejected_without_blocking() {
        use std::os::unix::ffi::OsStrExt;
        let (_t, home, b) = fixture();
        let fifo = notes_dir(&home, &b.root).join("pipe.md");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert_eq!(approve_file(&home, &b, "backend", &fifo), Err("not_a_file"));
        let candidate = Candidate {
            label: "claude-memory:pipe.md".into(),
            path: fifo.clone(),
            kind: SourceKind::Note,
            rejected: None,
        };
        assert_eq!(read(&candidate), Err("not_a_file"));
        // The open itself must not wait for a writer.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let opened = open_no_follow(&fifo).map(|f| f.metadata().unwrap().is_file());
            tx.send(opened).unwrap();
        });
        let opened = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("opening a FIFO blocked");
        assert_eq!(opened, Ok(false));
    }
    #[cfg(unix)]
    #[test]
    fn control_characters_in_note_names_are_refused() {
        let (_t, home, b) = fixture();
        let dir = notes_dir(&home, &b.root);
        std::fs::write(dir.join("bad\nname.md"), "note").unwrap();
        std::fs::write(dir.join("good.md"), "note").unwrap();
        let labels: Vec<_> = notes(&home, &b.root).into_iter().map(|c| c.label).collect();
        assert_eq!(labels, vec!["claude-memory:good.md"]);
        assert_eq!(
            approve_file(&home, &b, "backend", &dir.join("bad\nname.md")),
            Err("invalid_name")
        );
    }
    #[test]
    fn unapproved_services_have_no_repository_files() {
        let (_t, _home, b) = fixture();
        std::fs::write(b.root.join("CLAUDE.md"), "root").unwrap();
        std::fs::create_dir_all(b.root.join("frontend")).unwrap();
        std::fs::write(b.root.join("frontend/CLAUDE.md"), "other").unwrap();
        assert!(repository(&b, "frontend").is_empty());
    }
    #[test]
    fn read_rejects_files_that_grew_after_approval() {
        let (_t, home, b) = fixture();
        let path = notes_dir(&home, &b.root).join("grows.md");
        std::fs::write(&path, "small").unwrap();
        let candidate = approve_file(&home, &b, "backend", &path).unwrap();
        std::fs::write(&path, "x".repeat(MAX_FILE_BYTES as usize + 1)).unwrap();
        assert_eq!(read(&candidate), Err("too_large"));
    }
}
