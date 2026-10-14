//! Finds and validates agent memory files. Reads only the agent's approved
//! service folder and this project's Claude Code memory folder.

use crate::memory_split::SourceKind;
use std::path::{Path, PathBuf};
use wk_scope::enrollment::Binding;

pub const INSTRUCTION_FILES: [&str; 3] = ["CLAUDE.md", ".claude/CLAUDE.md", "AGENTS.md"];
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
/// The notes folder must be a real folder: a symlink could point anywhere.
fn symlinked(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.file_type().is_symlink())
}
/// Instruction files in the agent's service folder. Workspace-root files are
/// listed as rejected so `scan` can explain why they are not imported.
pub fn repository(b: &Binding, service: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    for name in INSTRUCTION_FILES {
        let root_file = b.root.join(name);
        if std::fs::symlink_metadata(&root_file).is_ok() {
            out.push(Candidate {
                label: format!("repo:{name}"),
                path: root_file,
                kind: SourceKind::Instructions,
                rejected: Some("outside_approved_service"),
            });
        }
        let path = b.root.join(service).join(name);
        if std::fs::symlink_metadata(&path).is_err() {
            continue;
        }
        let rejected =
            check(&path).or_else(|| (!b.eligible(&path)).then_some("outside_approved_service"));
        out.push(Candidate {
            label: format!("repo:{service}/{name}"),
            path,
            kind: SourceKind::Instructions,
            rejected,
        });
    }
    out
}
/// This project's Claude Code notes. Suggestions only: each needs `--file`.
pub fn notes(home: &Path, root: &Path) -> Vec<Candidate> {
    let dir = notes_dir(home, root);
    if symlinked(&dir) {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut out: Vec<Candidate> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            (name.ends_with(".md") && name != "MEMORY.md").then(|| {
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
    let canonical = path.canonicalize().map_err(|_| "unavailable")?;
    if canonical != path {
        return Err("symlink");
    }
    if let Some(found) = repository(b, service)
        .into_iter()
        .find(|c| c.path == canonical && c.rejected.is_none())
    {
        return Ok(found);
    }
    let notes = notes_dir(home, &b.root);
    if symlinked(&notes) {
        return Err("symlink");
    }
    let notes = notes.canonicalize().map_err(|_| "not_a_memory_source")?;
    let name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("not_a_memory_source")?;
    if canonical.parent() != Some(notes.as_path()) || !name.ends_with(".md") || name == "MEMORY.md"
    {
        return Err("not_a_memory_source");
    }
    Ok(Candidate {
        label: format!("claude-memory:{name}"),
        path: canonical,
        kind: SourceKind::Note,
        rejected: None,
    })
}
/// Reads through one handle that never follows a symlink and never reads
/// more than one byte past the limit.
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
        .custom_flags(libc::O_NOFOLLOW)
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
            Err("symlink")
        );
        assert_eq!(
            approve_file(&home, &b, "backend", &target.join("x.md")),
            Err("symlink")
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
}
