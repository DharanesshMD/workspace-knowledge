# Agent Memory Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `wk-agent memory scan|import` turns a developer's `CLAUDE.md`/`AGENTS.md` and Claude Code project notes into private local draft summaries that follow the existing review and submission path.

**Architecture:** Four focused modules in the `wk-agent` binary crate: a pure splitter, a source finder/validator, a content-free map file, and the command/drafting module. Sections are stored as encrypted `memory:` records in the existing local store; `process` drafts them with the existing local model client and card builder; `deliver` submits them as ordinary new summaries and records the returned ID. No server or contract change.

**Tech Stack:** Rust 2021 workspace, `wk-local` encrypted SQLite store, `wk-scope` enrollment binding, `wk-summarize` local inference, `serde_json`, `sha2`, `tokio`/`axum` for test fixtures.

**Spec:** `docs/superpowers/specs/2026-10-14-agent-memory-import-design.md`

**Commands** (run from the repository root; `./scripts/cargo` uses the checkout's toolchain):

- One module's tests: `./scripts/cargo test -p wk-agent --locked memory_split`
- Gate before every commit: `./scripts/cargo fmt --all -- --check && ./scripts/cargo clippy -p wk-agent --all-targets --locked -- -D warnings`
- Every commit uses `git commit -s`.

---

## File structure

| File | Responsibility |
|---|---|
| Create `crates/agent/src/memory_split.rs` | Pure: split note/instruction text into `Section`s |
| Create `crates/agent/src/memory_sources.rs` | Find candidate files, validate explicit `--file` paths, read with limits |
| Create `crates/agent/src/memory_map.rs` | Owner-only `memory-import.json` map and the pure re-import `decide` rule |
| Create `crates/agent/src/memory_import.rs` | `scan`/`import` commands, encrypted records, `draft_all`, `submitted`, `draft_item` |
| Modify `crates/agent/src/main.rs` | Module declarations, `Memory` subcommand and dispatch, CLI test |
| Modify `crates/agent/src/process.rs` | Call `draft_all` from `run`; memory branch in `deliver`; `list` uses `draft_item` |
| Modify `docs/adapters/setup.md`, `README.md`, `CHANGELOG.md` | User documentation |

Modules added before they are wired in carry `#[allow(dead_code)]` on their `mod` line, following the existing `mod repository;` precedent; Task 6 removes every such attribute.

---

### Task 1: Section splitter

**Files:**
- Create: `crates/agent/src/memory_split.rs`
- Modify: `crates/agent/src/main.rs` (module list near the top)

- [ ] **Step 1: Declare the module**

In `crates/agent/src/main.rs`, after `mod device;` add:

```rust
#[allow(dead_code)] // Wired into the CLI by the memory import task.
mod memory_split;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/agent/src/memory_split.rs` with only the tests:

```rust
//! Splits coding-agent memory files into reviewable sections. Pure: no I/O.

#[cfg(test)]
mod tests {
    use super::*;
    fn h(parts: &[&str], occurrence: usize) -> (Vec<String>, usize) {
        (parts.iter().map(|p| p.to_string()).collect(), occurrence)
    }
    fn headings(sections: &[Section]) -> Vec<(Vec<String>, usize)> {
        sections
            .iter()
            .map(|s| (s.heading.clone(), s.occurrence))
            .collect()
    }
    #[test]
    fn note_drops_frontmatter_and_keeps_name_and_description_as_hints() {
        let s = split(
            SourceKind::Note,
            "---\nname: retries\ndescription: \"Retry policy\"\nmetadata:\n  type: project\n---\nPayment retries reuse the idempotency key.\n",
        );
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].hints.as_deref(),
            Some("name: retries; description: Retry policy")
        );
        assert_eq!(s[0].body, "Payment retries reuse the idempotency key.");
        assert!(s[0].heading.is_empty());
    }
    #[test]
    fn instructions_split_at_the_repeated_level_under_a_title() {
        let s = split(
            SourceKind::Instructions,
            "# Backend\nIntro text.\n## Build\nRun cargo test.\n### Detail\nStays in build.\n## Style\nUse rustfmt.\n",
        );
        assert_eq!(
            headings(&s),
            vec![
                h(&["Backend"], 0),
                h(&["Backend", "Build"], 0),
                h(&["Backend", "Style"], 0)
            ]
        );
        assert!(s[1].body.contains("### Detail"));
    }
    #[test]
    fn headings_inside_fenced_code_do_not_split() {
        let s = split(
            SourceKind::Instructions,
            "## Build\n```sh\n## not a heading\n```\n## Test\nRun it.\n",
        );
        assert_eq!(headings(&s), vec![h(&["Build"], 0), h(&["Test"], 0)]);
        assert!(s[0].body.contains("## not a heading"));
    }
    #[test]
    fn repeated_headings_get_occurrences_and_empty_sections_are_dropped() {
        let s = split(
            SourceKind::Instructions,
            "## Notes\nOne.\n## Empty\n\n## Notes\nTwo.\n",
        );
        assert_eq!(headings(&s), vec![h(&["Notes"], 0), h(&["Notes"], 1)]);
    }
    #[test]
    fn text_without_headings_is_one_preamble_section() {
        let s = split(SourceKind::Instructions, "Always run the linter.\r\n");
        assert_eq!(headings(&s), vec![h(&[], 0)]);
        assert_eq!(s[0].body, "Always run the linter.");
    }
    #[test]
    fn empty_note_has_no_sections() {
        assert!(split(SourceKind::Note, "---\nname: x\n---\n\n").is_empty());
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `./scripts/cargo test -p wk-agent --locked memory_split`
Expected: compile errors `cannot find type Section`, `cannot find function split`, `SourceKind`.

- [ ] **Step 4: Implement**

Insert above the `#[cfg(test)]` block:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Instructions,
    Note,
}
impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Instructions => "instructions",
            SourceKind::Note => "claude_note",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading path; empty for a note or for text before any heading.
    pub heading: Vec<String>,
    /// Index among earlier sections with the same heading path.
    pub occurrence: usize,
    /// Note frontmatter `name`/`description`, used only as model hints.
    pub hints: Option<String>,
    pub body: String,
}
pub fn split(kind: SourceKind, text: &str) -> Vec<Section> {
    let text = text.replace("\r\n", "\n");
    match kind {
        SourceKind::Note => note(&text),
        SourceKind::Instructions => instructions(&text),
    }
}
fn note(text: &str) -> Vec<Section> {
    let (hints, body) = frontmatter(text);
    let body = body.trim();
    if body.is_empty() {
        return vec![];
    }
    vec![Section {
        heading: vec![],
        occurrence: 0,
        hints,
        body: body.to_string(),
    }]
}
fn frontmatter(text: &str) -> (Option<String>, String) {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return (None, text.to_string());
    }
    let mut meta = Vec::new();
    for line in lines.by_ref() {
        if line == "---" {
            let hints: Vec<String> = meta
                .iter()
                .filter_map(|line: &&str| {
                    let (key, value) = line.split_once(':')?;
                    let (key, value) = (key.trim(), value.trim().trim_matches('"'));
                    (matches!(key, "name" | "description") && !value.is_empty())
                        .then(|| format!("{key}: {value}"))
                })
                .collect();
            let body: Vec<&str> = lines.collect();
            return ((!hints.is_empty()).then(|| hints.join("; ")), body.join("\n"));
        }
        meta.push(line);
    }
    (None, text.to_string())
}
fn heading(line: &str) -> Option<(usize, String)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) || !line[level..].starts_with(' ') {
        return None;
    }
    let title = line[level..].trim().trim_end_matches('#').trim();
    (!title.is_empty()).then(|| (level, title.to_string()))
}
fn instructions(text: &str) -> Vec<Section> {
    let mut fenced = false;
    let lines: Vec<(&str, Option<(usize, String)>)> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
                return (line, None);
            }
            (line, if fenced { None } else { heading(line) })
        })
        .collect();
    let mut counts = [0usize; 7];
    for (_, found) in &lines {
        if let Some((level, _)) = found {
            counts[*level] += 1;
        }
    }
    // Split at the highest level used more than once, else the highest present.
    let split_level = (1..=6)
        .find(|level| counts[*level] >= 2)
        .or_else(|| (1..=6).find(|level| counts[*level] >= 1));
    let mut sections = Vec::new();
    let mut parents: Vec<(usize, String)> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    for (line, found) in &lines {
        match (found, split_level) {
            (Some((level, title)), Some(at)) if *level == at => {
                push(&mut sections, &current, &body);
                body.clear();
                current = parents.iter().map(|(_, t)| t.clone()).collect();
                current.push(title.clone());
            }
            (Some((level, title)), Some(at)) if *level < at => {
                push(&mut sections, &current, &body);
                body.clear();
                parents.retain(|(l, _)| l < level);
                parents.push((*level, title.clone()));
                current = parents.iter().map(|(_, t)| t.clone()).collect();
            }
            _ => body.push(*line),
        }
    }
    push(&mut sections, &current, &body);
    sections
}
fn push(sections: &mut Vec<Section>, heading: &[String], body: &[&str]) {
    let text = body.join("\n").trim().to_string();
    if text.is_empty() {
        return;
    }
    let occurrence = sections.iter().filter(|s| s.heading == heading).count();
    sections.push(Section {
        heading: heading.to_vec(),
        occurrence,
        hints: None,
        body: text,
    });
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `./scripts/cargo test -p wk-agent --locked memory_split`
Expected: `test result: ok. 6 passed`

- [ ] **Step 6: Gate and commit**

```bash
./scripts/cargo fmt --all -- --check && ./scripts/cargo clippy -p wk-agent --all-targets --locked -- -D warnings
git add crates/agent/src/memory_split.rs crates/agent/src/main.rs
git commit -s -m "feat: split agent memory files into reviewable sections"
```

---

### Task 2: Source discovery and validation

**Files:**
- Create: `crates/agent/src/memory_sources.rs`
- Modify: `crates/agent/src/main.rs`

- [ ] **Step 1: Declare the module**

After the `mod memory_split;` line add:

```rust
#[allow(dead_code)] // Wired into the CLI by the memory import task.
mod memory_sources;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/agent/src/memory_sources.rs`:

```rust
//! Finds and validates agent memory files. Reads only the agent's approved
//! service folder and this project's Claude Code memory folder.

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
        let summary: Vec<_> = found.iter().map(|c| (c.label.as_str(), c.rejected)).collect();
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
            approve_file(&home, &b, "backend", &dir.join("retries.md")).unwrap().kind,
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
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `./scripts/cargo test -p wk-agent --locked memory_sources`
Expected: compile errors for `notes_dir`, `repository`, `approve_file`, `Candidate`.

- [ ] **Step 4: Implement**

Insert above the `#[cfg(test)]` block:

```rust
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
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' { ch } else { '-' })
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
        let rejected = check(&path)
            .or_else(|| (!b.eligible(&path)).then_some("outside_approved_service"));
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
    let notes = notes_dir(home, &b.root)
        .canonicalize()
        .map_err(|_| "not_a_memory_source")?;
    let name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("not_a_memory_source")?;
    if canonical.parent() != Some(notes.as_path()) || !name.ends_with(".md") || name == "MEMORY.md" {
        return Err("not_a_memory_source");
    }
    Ok(Candidate {
        label: format!("claude-memory:{name}"),
        path: canonical,
        kind: SourceKind::Note,
        rejected: None,
    })
}
pub fn read(c: &Candidate) -> Result<String, &'static str> {
    if let Some(reason) = check(&c.path) {
        return Err(reason);
    }
    String::from_utf8(std::fs::read(&c.path).map_err(|_| "unavailable")?).map_err(|_| "not_utf8")
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `./scripts/cargo test -p wk-agent --locked memory_sources`
Expected: `test result: ok. 6 passed` (5 on non-Unix).

- [ ] **Step 6: Gate and commit**

```bash
./scripts/cargo fmt --all -- --check && ./scripts/cargo clippy -p wk-agent --all-targets --locked -- -D warnings
git add crates/agent/src/memory_sources.rs crates/agent/src/main.rs
git commit -s -m "feat: find and validate agent memory sources within scope"
```

---

### Task 3: Import map and re-import rule

**Files:**
- Create: `crates/agent/src/memory_map.rs`
- Modify: `crates/agent/src/main.rs`

- [ ] **Step 1: Declare the module**

After the `mod memory_sources;` line add:

```rust
#[allow(dead_code)] // Wired into the CLI by the memory import task.
mod memory_map;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/agent/src/memory_map.rs`:

```rust
//! Content-free record of imported memory sections, kept in an owner-only
//! state file because the encrypted store retains records for at most a day.

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(record: Option<&str>, draft: Option<&str>, summary: Option<&str>) -> Entry {
        Entry {
            label: "repo:backend/CLAUDE.md".into(),
            content: "c1".into(),
            generation: 2,
            record: record.map(Into::into),
            draft: draft.map(Into::into),
            summary: summary.map(Into::into),
        }
    }
    fn write(generation: u32, updates: Option<&str>, retire: &[&str]) -> Action {
        Action::Write {
            generation,
            updates: updates.map(Into::into),
            retire: retire.iter().map(|s| s.to_string()).collect(),
        }
    }
    #[test]
    fn new_sections_start_at_generation_one() {
        assert_eq!(decide(None, "c1", false), write(1, None, &[]));
    }
    #[test]
    fn unchanged_pending_or_submitted_sections_are_skipped() {
        assert_eq!(decide(Some(&entry(None, Some("draft:a"), None)), "c1", true), Action::Skip);
        assert_eq!(decide(Some(&entry(None, None, Some("s1"))), "c1", false), Action::Skip);
    }
    #[test]
    fn unchanged_expired_work_is_redrafted_under_a_new_generation() {
        assert_eq!(
            decide(Some(&entry(None, Some("draft:a"), None)), "c1", false),
            write(3, None, &[])
        );
        assert_eq!(
            decide(Some(&entry(None, Some("draft:a"), Some("s1"))), "c1", false),
            write(3, Some("s1"), &[])
        );
    }
    #[test]
    fn changed_sections_retire_unsubmitted_work_and_link_submitted_summaries() {
        assert_eq!(
            decide(Some(&entry(Some("memory:r"), Some("draft:a"), None)), "c2", true),
            write(3, None, &["memory:r", "draft:a"])
        );
        assert_eq!(
            decide(Some(&entry(None, None, Some("s1"))), "c2", false),
            write(3, Some("s1"), &[])
        );
    }
    #[test]
    fn map_round_trips_owner_only_and_rejects_unknown_versions() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(load(temp.path()).unwrap(), Map::default());
        let mut map = Map::default();
        map.entries.insert("k".into(), entry(None, None, Some("s1")));
        save(temp.path(), &map).unwrap();
        assert_eq!(load(temp.path()).unwrap(), map);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(temp.path().join(FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(temp.path().join(FILE), r#"{"version":2,"entries":{}}"#).unwrap();
        assert!(load(temp.path()).is_err());
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `./scripts/cargo test -p wk-agent --locked memory_map`
Expected: compile errors for `Entry`, `Action`, `decide`, `Map`, `load`, `save`, `FILE`.

- [ ] **Step 4: Implement**

Insert above the `#[cfg(test)]` block:

```rust
use crate::state;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub const FILE: &str = "memory-import.json";

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Map {
    pub version: u32,
    /// Keyed by the hashed section key; never holds section text.
    pub entries: BTreeMap<String, Entry>,
}
impl Default for Map {
    fn default() -> Self {
        Self {
            version: 1,
            entries: BTreeMap::new(),
        }
    }
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub label: String,
    /// Hash of the section heading, hints and body.
    pub content: String,
    pub generation: u32,
    /// Pending `memory:` record, until `process` drafts it.
    pub record: Option<String>,
    /// Unsubmitted `draft:` ID, cleared on submission.
    pub draft: Option<String>,
    /// Last submitted summary ID, linked from later updates.
    pub summary: Option<String>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Skip,
    Write {
        generation: u32,
        updates: Option<String>,
        retire: Vec<String>,
    },
}
/// `pending`: the entry's record or draft is still in the private store.
pub fn decide(entry: Option<&Entry>, content: &str, pending: bool) -> Action {
    let Some(e) = entry else {
        return Action::Write {
            generation: 1,
            updates: None,
            retire: vec![],
        };
    };
    if e.content == content {
        if pending || (e.record.is_none() && e.draft.is_none()) {
            return Action::Skip;
        }
        // Explicit re-run after the draft expired unsubmitted: new identity.
        return Action::Write {
            generation: e.generation + 1,
            updates: e.summary.clone(),
            retire: vec![],
        };
    }
    Action::Write {
        generation: e.generation + 1,
        updates: e.summary.clone(),
        retire: e.record.iter().chain(e.draft.iter()).cloned().collect(),
    }
}
pub fn load(dir: &Path) -> Result<Map, String> {
    let path = dir.join(FILE);
    if std::fs::symlink_metadata(&path).is_err() {
        return Ok(Map::default());
    }
    let map: Map = serde_json::from_str(&state::read(&path, 4 * 1024 * 1024)?)
        .map_err(|_| "invalid memory import map")?;
    if map.version != 1 {
        return Err("unsupported memory import map".into());
    }
    Ok(map)
}
/// `state::save` writes atomically through an owner-only temporary file.
pub fn save(dir: &Path, map: &Map) -> Result<(), String> {
    state::save(&dir.join(FILE), map)
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `./scripts/cargo test -p wk-agent --locked memory_map`
Expected: `test result: ok. 5 passed`

- [ ] **Step 6: Gate and commit**

```bash
./scripts/cargo fmt --all -- --check && ./scripts/cargo clippy -p wk-agent --all-targets --locked -- -D warnings
git add crates/agent/src/memory_map.rs crates/agent/src/main.rs
git commit -s -m "feat: track imported memory sections without storing their text"
```

---

### Task 4: `memory scan` and `memory import` commands

**Files:**
- Create: `crates/agent/src/memory_import.rs`
- Modify: `crates/agent/src/main.rs` (module list, `Command` enum, dispatch, `cli_tests`)

- [ ] **Step 1: Declare the module and CLI command**

After the `mod memory_map;` line add:

```rust
#[allow(dead_code)] // Drafting and submission are wired in by later memory tasks.
mod memory_import;
```

In `enum Command`, after the `Import { ... },` variant add:

```rust
    /// Import CLAUDE.md, AGENTS.md and Claude Code project notes as private drafts.
    Memory {
        /// scan | import
        action: String,
        #[arg(long)]
        yes: bool,
        /// Claude Code project note to include; repeat per file.
        #[arg(long)]
        file: Vec<PathBuf>,
    },
```

In the dispatch `match`, after the `Command::Import { ... } => ...` arm add:

```rust
        Command::Memory { action, yes, file } => {
            memory_import::command(&state, &action, yes, &file).await
        }
```

In `mod cli_tests`, add:

```rust
    #[test]
    fn memory_command_accepts_repeated_explicit_files() {
        let cli = Cli::try_parse_from([
            "wk-agent", "--state-dir", "/private/wk", "memory", "import", "--yes",
            "--file", "/notes/one.md", "--file", "/notes/two.md",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Memory { action, yes, file }
            if action == "import" && yes && file.len() == 2));
    }
```

- [ ] **Step 2: Write the failing tests**

Create `crates/agent/src/memory_import.rs` with the tests only:

```rust
//! `wk-agent memory scan|import`: turn agent memory into private draft
//! summaries. Nothing here publishes; submission stays explicit.

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) struct Fixture {
        pub _temp: tempfile::TempDir,
        pub dir: PathBuf,
        pub home: PathBuf,
        pub c: state::Config,
        pub b: Binding,
        pub store: LocalStore,
    }
    pub(super) fn fixture(inference: &str) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("project");
        std::fs::create_dir_all(root.join("backend")).unwrap();
        let b = Binding::approve(&root, "demo", vec!["backend".into()]).unwrap();
        let dir = state::private_dir(&base.join("state"), true).unwrap();
        let c = state::Config {
            root: b.root.clone(),
            binding_file: dir.join("binding.json"),
            service: "backend".into(),
            server: "http://127.0.0.1:4318".into(),
            model: "synthetic-local".into(),
            inference: inference.into(),
            port: 0,
            account: "test".into(),
            device_id: "test-device".into(),
            repository: None,
            device: None,
        };
        state::save(&c.binding_file, &b).unwrap();
        state::save(&dir.join("agent.json"), &c).unwrap();
        let home = base.join("home");
        std::fs::create_dir_all(memory_sources::notes_dir(&home, &b.root)).unwrap();
        let store = LocalStore::open(&dir.join("test.db"), [7; 32], 10_000_000).unwrap();
        Fixture { _temp: temp, dir, home, c, b, store }
    }
    fn rows(scan: &Value) -> Vec<(String, String)> {
        scan["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["label"].as_str().unwrap().into(), r["status"].as_str().unwrap().into()))
            .collect()
    }
    #[test]
    fn scan_reports_sources_without_reading_into_output() {
        let f = fixture("http://127.0.0.1:9/v1/chat/completions");
        std::fs::write(f.b.root.join("CLAUDE.md"), "# Root\nOutside the service.\n").unwrap();
        std::fs::write(f.b.root.join("backend/CLAUDE.md"), "## Build\nRun tests.\n## Style\nUse rustfmt.\n").unwrap();
        let notes = memory_sources::notes_dir(&f.home, &f.b.root);
        std::fs::write(notes.join("deploy.md"), "Deploys happen after the Tuesday sync.\n").unwrap();
        let scan = scan_with(&f.dir, &f.b, "backend", &f.home).unwrap();
        assert_eq!(
            rows(&scan),
            vec![
                ("repo:CLAUDE.md".into(), "rejected".into()),
                ("repo:backend/CLAUDE.md".into(), "new".into()),
                ("claude-memory:deploy.md".into(), "new".into())
            ]
        );
        assert!(!scan.to_string().contains("Tuesday"));
        assert!(!scan.to_string().contains(&*f.b.root.to_string_lossy()));
    }
    #[test]
    fn import_writes_private_records_and_skips_unchanged_sections() {
        let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
        std::fs::write(f.b.root.join("backend/CLAUDE.md"), "## Build\nRun tests.\n## Style\nUse rustfmt.\n").unwrap();
        let note = memory_sources::notes_dir(&f.home, &f.b.root).join("deploy.md");
        std::fs::write(&note, "Deploys happen after the Tuesday sync.\n").unwrap();
        let now = daemon::now().unwrap();
        let first = import_with(&f.dir, &f.b, "backend", &mut f.store, &f.home, &[note.clone()], now).unwrap();
        assert_eq!((first["written"].clone(), first["unchanged"].clone()), (json!(3), json!(0)));
        let records = f.store.pending_ids("memory:", "", now, 100).unwrap();
        assert_eq!(records.len(), 3);
        let raw = std::fs::read(f.dir.join("test.db")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Tuesday"), "records are encrypted");
        let map = std::fs::read_to_string(f.dir.join(memory_map::FILE)).unwrap();
        assert!(!map.contains("Tuesday") && !map.contains("rustfmt"), "map holds no section text");
        let again = import_with(&f.dir, &f.b, "backend", &mut f.store, &f.home, &[note], now).unwrap();
        assert_eq!((again["written"].clone(), again["unchanged"].clone()), (json!(0), json!(3)));
    }
    #[test]
    fn rejected_files_and_oversized_runs_write_nothing() {
        let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
        let now = daemon::now().unwrap();
        std::fs::create_dir_all(f.home.join(".claude")).unwrap();
        std::fs::write(f.home.join(".claude/CLAUDE.md"), "global").unwrap();
        assert_eq!(
            import_with(&f.dir, &f.b, "backend", &mut f.store, &f.home, &[f.home.join(".claude/CLAUDE.md")], now),
            Err("memory_source_rejected:not_a_memory_source".into())
        );
        let many: String = (0..=MAX_SECTIONS).map(|i| format!("## S{i}\nBody {i}.\n")).collect();
        std::fs::write(f.b.root.join("backend/AGENTS.md"), many).unwrap();
        assert_eq!(
            import_with(&f.dir, &f.b, "backend", &mut f.store, &f.home, &[], now),
            Err("too_many_memory_sections".into())
        );
        assert!(f.store.pending_ids("memory:", "", now, 100).unwrap().is_empty());
        assert!(!f.dir.join(memory_map::FILE).exists());
    }
    #[tokio::test]
    async fn unknown_actions_are_refused() {
        let f = fixture("http://127.0.0.1:9/v1/chat/completions");
        assert!(command(&f.dir, "publish", true, &[]).await.is_err());
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `./scripts/cargo test -p wk-agent --locked memory_import`
Expected: compile errors for `scan_with`, `import_with`, `command`, `MAX_SECTIONS` and missing imports.

- [ ] **Step 4: Implement**

Insert above the `#[cfg(test)]` block:

```rust
use crate::{
    daemon,
    memory_map::{self, Action, Entry},
    memory_sources::{self, Candidate},
    memory_split::{self, Section, SourceKind},
    network, state,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use wk_local::LocalStore;
use wk_scope::enrollment::Binding;

pub const MAX_SECTIONS: usize = 200;

/// Encrypted `memory:` record; consumed by `draft_all`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    version: u32,
    key: String,
    generation: u32,
    label: String,
    heading: Vec<String>,
    hints: Option<String>,
    body: String,
    policy_hash: String,
    updates: Option<String>,
}
fn hex(value: &impl Serialize) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|_| "invalid memory section")?)
    ))
}
fn section_key(label: &str, s: &Section) -> Result<String, String> {
    hex(&(label, &s.heading, s.occurrence))
}
fn content_hash(s: &Section) -> Result<String, String> {
    hex(&(&s.heading, &s.hints, &s.body))
}
fn record_id(key: &str, generation: u32) -> Result<String, String> {
    Ok(format!("memory:{}", hex(&(key, generation))?))
}
fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "home_unavailable".into())
}
pub async fn command(
    dir: &Path,
    action: &str,
    yes: bool,
    files: &[PathBuf],
) -> Result<Value, String> {
    match action {
        "scan" => scan(dir),
        "import" => import(dir, yes, files).await,
        _ => Err("memory action must be scan or import".into()),
    }
}
/// Read-only and offline: local approval only, no credential or network use.
fn scan(dir: &Path) -> Result<Value, String> {
    let c = state::load(dir)?;
    let b = state::base_binding(&c)?;
    scan_with(dir, &b, &c.service, &home()?)
}
pub(crate) fn scan_with(dir: &Path, b: &Binding, service: &str, home: &Path) -> Result<Value, String> {
    let map = memory_map::load(dir)?;
    let mut rows = Vec::new();
    let mut present = BTreeSet::new();
    let candidates = memory_sources::repository(b, service)
        .into_iter()
        .chain(memory_sources::notes(home, &b.root));
    for candidate in candidates {
        let mut row = json!({
            "label": candidate.label,
            "kind": candidate.kind.label(),
            "requires_file_approval": candidate.kind == SourceKind::Note,
        });
        let text = match candidate.rejected {
            Some(reason) => Err(reason),
            None => memory_sources::read(&candidate),
        };
        let sections = match text {
            Ok(text) => memory_split::split(candidate.kind, &text),
            Err(reason) => {
                row["status"] = json!("rejected");
                row["reason"] = json!(reason);
                rows.push(row);
                continue;
            }
        };
        present.insert(candidate.label.clone());
        let (mut known, mut same) = (0, 0);
        for s in &sections {
            if let Some(e) = map.entries.get(&section_key(&candidate.label, s)?) {
                known += 1;
                if e.content == content_hash(s)? {
                    same += 1;
                }
            }
        }
        row["sections"] = json!(sections.len());
        row["status"] = json!(if sections.is_empty() {
            "empty"
        } else if known == 0 {
            "new"
        } else if same == sections.len() {
            "unchanged"
        } else {
            "changed"
        });
        rows.push(row);
    }
    let removed: BTreeSet<&String> = map
        .entries
        .values()
        .map(|e| &e.label)
        .filter(|label| !present.contains(*label))
        .collect();
    Ok(json!({"sources": rows, "removed": removed, "published": false}))
}
async fn import(dir: &Path, yes: bool, files: &[PathBuf]) -> Result<Value, String> {
    if !yes {
        return Err("review `wk-agent memory scan` first and pass --yes".into());
    }
    if dir.join("capture-gap.json").exists() {
        return Err("capture gap requires clean-session reset".into());
    }
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let token = state::token(dir, "server-token")?;
    network::policy(&c, &b, &token).await?;
    let mut store = state::store(dir)?;
    import_with(dir, &b, &c.service, &mut store, &home()?, files, daemon::now()?)
}
/// Validate every source and count sections before writing anything.
pub(crate) fn import_with(
    dir: &Path,
    b: &Binding,
    service: &str,
    store: &mut LocalStore,
    home: &Path,
    files: &[PathBuf],
    now: i64,
) -> Result<Value, String> {
    let mut sources: Vec<Candidate> = memory_sources::repository(b, service)
        .into_iter()
        .filter(|c| c.rejected.is_none())
        .collect();
    for file in files {
        let approved = memory_sources::approve_file(home, b, service, file)
            .map_err(|reason| format!("memory_source_rejected:{reason}"))?;
        if !sources.iter().any(|s| s.path == approved.path) {
            sources.push(approved);
        }
    }
    let mut prepared = Vec::new();
    for source in &sources {
        let text = memory_sources::read(source)
            .map_err(|reason| format!("memory_source_rejected:{reason}"))?;
        for section in memory_split::split(source.kind, &text) {
            prepared.push((source.label.clone(), section));
        }
    }
    if prepared.len() > MAX_SECTIONS {
        return Err("too_many_memory_sections".into());
    }
    let mut map = memory_map::load(dir)?;
    let (mut written, mut unchanged) = (0, 0);
    for (label, section) in prepared {
        let key = section_key(&label, &section)?;
        let content = content_hash(&section)?;
        let entry = map.entries.get(&key);
        let mut pending = false;
        if let Some(e) = entry {
            for id in e.record.iter().chain(e.draft.iter()) {
                pending |= store.contains(id, now)?;
            }
        }
        match memory_map::decide(entry, &content, pending) {
            Action::Skip => unchanged += 1,
            Action::Write {
                generation,
                updates,
                retire,
            } => {
                for id in &retire {
                    store.ack(id)?;
                }
                let id = record_id(&key, generation)?;
                let record = Record {
                    version: 1,
                    key: key.clone(),
                    generation,
                    label: label.clone(),
                    heading: section.heading,
                    hints: section.hints,
                    body: section.body,
                    policy_hash: b.policy_hash.clone(),
                    updates,
                };
                let bytes = serde_json::to_vec(&record).map_err(|_| "invalid memory record")?;
                store.enqueue(&id, &bytes, now, 86400)?;
                let summary = map.entries.get(&key).and_then(|e| e.summary.clone());
                map.entries.insert(
                    key,
                    Entry {
                        label,
                        content,
                        generation,
                        record: Some(id),
                        draft: None,
                        summary,
                    },
                );
                written += 1;
            }
        }
    }
    memory_map::save(dir, &map)?;
    Ok(json!({
        "written": written,
        "unchanged": unchanged,
        "sources": sources.len(),
        "published": false,
        "next": "run `wk-agent process`, then review `wk-agent drafts`",
    }))
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `./scripts/cargo test -p wk-agent --locked memory_import`
Expected: `test result: ok. 4 passed`
Run: `./scripts/cargo test -p wk-agent --locked memory_command_accepts`
Expected: `test result: ok. 1 passed`

- [ ] **Step 6: Gate and commit**

```bash
./scripts/cargo fmt --all -- --check && ./scripts/cargo clippy -p wk-agent --all-targets --locked -- -D warnings
git add crates/agent/src/memory_import.rs crates/agent/src/main.rs
git commit -s -m "feat: add wk-agent memory scan and import commands"
```

---

### Task 5: Draft imported sections with the local model

**Files:**
- Modify: `crates/agent/src/memory_import.rs`
- Modify: `crates/agent/src/process.rs` (`run`, lines 41–43)

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `memory_import.rs`:

```rust
    use std::sync::{Arc, Mutex};
    async fn model() -> (String, tokio::task::JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
        let sources = Arc::new(Mutex::new(Vec::new()));
        let seen = sources.clone();
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(body["messages"][1]["content"].as_str().unwrap_or("").to_string());
                    let summary = json!({
                        "title": "Imported backend convention",
                        "intent": "Keep a recorded backend convention available to teammates",
                        "rationale": "The developer's agent memory recorded this convention",
                        "decisions": ["Follow the recorded convention for backend changes"],
                        "constraints": [], "open_questions": [], "sensitive": false
                    });
                    axum::Json(json!({"choices": [{"message": {"content": summary.to_string()}}]}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
        (url, tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }), sources)
    }
    #[tokio::test]
    async fn process_drafts_records_once_without_absolute_paths() {
        let (url, server, sources) = model().await;
        let mut f = fixture(&url);
        std::fs::write(f.b.root.join("backend/CLAUDE.md"), "## Build\nRun tests.\n## Retries\nReuse the idempotency key.\n").unwrap();
        let note = memory_sources::notes_dir(&f.home, &f.b.root).join("deploy.md");
        std::fs::write(&note, "---\nname: deploy\n---\nDeploys happen after the Tuesday sync.\n").unwrap();
        let now = daemon::now().unwrap();
        import_with(&f.dir, &f.b, "backend", &mut f.store, &f.home, &[note], now).unwrap();
        let policy = json!({"policy_revision": 1});
        assert_eq!(draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store).await.unwrap(), (3, 0));
        assert!(f.store.pending_ids("memory:", "", now, 100).unwrap().is_empty(), "records consumed");
        let drafts = f.store.pending_ids("draft:", "", now, 100).unwrap();
        assert_eq!(drafts.len(), 3);
        let base = f.dir.parent().unwrap().to_string_lossy().to_string();
        for id in &drafts {
            let draft: Value = serde_json::from_slice(&f.store.read_record(id, now).unwrap().unwrap()).unwrap();
            assert_eq!(draft["card"]["capture_completeness"], "complete");
            assert_eq!(draft["card"]["service_ids"], json!(["backend"]));
            assert!(!draft.to_string().contains(&base), "no absolute path in drafts");
        }
        let sent = sources.lock().unwrap().clone();
        assert_eq!(sent.len(), 3);
        assert!(sent.iter().all(|s| !s.contains(&base) && s.starts_with("Agent memory: ")));
        let map = memory_map::load(&f.dir).unwrap();
        assert!(map.entries.values().all(|e| e.record.is_none() && e.draft.is_some()));
        assert_eq!(draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store).await.unwrap(), (0, 0));
        server.abort();
    }
```

- [ ] **Step 2: Run the test to see it fail**

Run: `./scripts/cargo test -p wk-agent --locked process_drafts_records_once`
Expected: compile error `cannot find function draft_all`.

- [ ] **Step 3: Implement `draft_all`**

Add `process` to the `use crate::{...}` list in `memory_import.rs`, then add after `import_with`:

```rust
impl Record {
    /// Model input: the label and heading path, never an absolute path.
    fn source(&self) -> String {
        let mut text = format!("Agent memory: {}\n", self.label);
        if !self.heading.is_empty() {
            text += &format!("Section: {}\n", self.heading.join(" > "));
        }
        if let Some(hints) = &self.hints {
            text += &format!("Note: {hints}\n");
        }
        text + "\n" + &self.body
    }
}
/// Draft pending memory records. Mirrors `journal_turns`: approval is
/// re-checked around inference, and failed records stay pending until expiry.
pub(crate) async fn draft_all(
    dir: &Path,
    c: &state::Config,
    b: &Binding,
    policy: &Value,
    store: &mut LocalStore,
) -> Result<(usize, usize), String> {
    let ids = store.next_ids("memory", "memory:", daemon::now()?, 10)?;
    let revision = policy["policy_revision"].as_u64().ok_or("invalid policy")?;
    let mut map = memory_map::load(dir)?;
    let (mut drafts, mut held) = (0, 0);
    for id in ids {
        if process::processing_current(dir, c, b).is_err() {
            break;
        }
        let Some(bytes) = store.read_record(&id, daemon::now()?)? else {
            continue;
        };
        let record: Record =
            serde_json::from_slice(&bytes).map_err(|_| "invalid memory record")?;
        if record.policy_hash != b.policy_hash {
            // Approval changed since import; the developer must import again.
            store.ack(&id)?;
            held += 1;
            continue;
        }
        let hash = hex(&(
            &id,
            &record.heading,
            &record.hints,
            &record.body,
            &b.policy_hash,
            revision,
            c.model.as_str(),
            1,
        ))?;
        let key = format!("draft:{hash}");
        if store.seen(&key)? {
            store.ack(&id)?;
            continue;
        }
        match wk_summarize::extract(&c.inference, &c.model, &record.source()).await {
            Ok(extract) => {
                if process::processing_current(dir, c, b).is_err() {
                    break;
                }
                let card = wk_summarize::card(extract, hash, revision, vec![c.service.clone()], true)?;
                let draft = json!({
                    "card": card,
                    "memory": {
                        "key": record.key,
                        "generation": record.generation,
                        "label": record.label,
                        "heading": record.heading,
                        "updates": record.updates,
                    },
                    "policy_hash": b.policy_hash,
                    "model": c.model,
                });
                let encoded = serde_json::to_vec(&draft).map_err(|_| "invalid draft")?;
                store.update(&key, daemon::now()?, 3600, |old| {
                    Ok(old.map_or(encoded.clone(), |bytes| bytes.to_vec()))
                })?;
                store.ack(&id)?;
                if let Some(e) = map.entries.get_mut(&record.key) {
                    if e.generation == record.generation {
                        e.record = None;
                        e.draft = Some(key);
                    }
                }
                drafts += 1;
            }
            Err(_) => held += 1,
        }
    }
    memory_map::save(dir, &map)?;
    Ok((drafts, held))
}
```

- [ ] **Step 4: Call it from `process::run`**

In `crates/agent/src/process.rs`, inside `run`, after the `for id in ids { ... }` loop and before `let owner_sync = ...`, add:

```rust
    let (made, failed) = crate::memory_import::draft_all(dir, &c, &b, &policy, &mut store).await?;
    drafts += made;
    held += failed;
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `./scripts/cargo test -p wk-agent --locked memory_import`
Expected: `test result: ok. 5 passed`
Run: `./scripts/cargo test -p wk-agent --locked process`
Expected: existing `process` tests still pass.

- [ ] **Step 6: Gate and commit**

```bash
./scripts/cargo fmt --all -- --check && ./scripts/cargo clippy -p wk-agent --all-targets --locked -- -D warnings
git add crates/agent/src/memory_import.rs crates/agent/src/process.rs
git commit -s -m "feat: draft imported agent memory with the local model"
```

---

### Task 6: Show, submit and link memory drafts

**Files:**
- Modify: `crates/agent/src/memory_import.rs`
- Modify: `crates/agent/src/process.rs` (`list`, `deliver`)
- Modify: `crates/agent/src/main.rs` (remove the four `#[allow(dead_code)]` lines added in Tasks 1–4)

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `memory_import.rs`:

```rust
    #[test]
    fn draft_items_show_memory_origin_without_internal_keys() {
        let v = json!({"card": {"title": "T"}, "model": "m",
            "memory": {"key": "k", "generation": 2, "label": "claude-memory:deploy.md",
                       "heading": [], "updates": "summary-1"}});
        let item = draft_item("draft:x", &v);
        assert_eq!(item["memory"], json!({"label": "claude-memory:deploy.md", "heading": [], "updates": "summary-1"}));
        assert!(draft_item("draft:y", &json!({"card": {}, "model": "m"})).get("memory").is_none());
    }
    #[tokio::test]
    async fn submitted_summaries_link_the_next_changed_section() {
        let (url, server, _sources) = model().await;
        let mut f = fixture(&url);
        let file = f.b.root.join("backend/CLAUDE.md");
        std::fs::write(&file, "## Build\nRun tests.\n## Retries\nReuse the idempotency key.\n").unwrap();
        let now = daemon::now().unwrap();
        let policy = json!({"policy_revision": 1});
        import_with(&f.dir, &f.b, "backend", &mut f.store, &f.home, &[], now).unwrap();
        draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store).await.unwrap();
        let first = f.store.pending_ids("draft:", "", now, 100).unwrap();
        let retries = first.iter().find(|id| {
            let v: Value = serde_json::from_slice(&f.store.read_record(id, now).unwrap().unwrap()).unwrap();
            v["memory"]["heading"] == json!(["Retries"])
        }).unwrap().clone();
        let draft: Value = serde_json::from_slice(&f.store.read_record(&retries, now).unwrap().unwrap()).unwrap();
        submitted(&f.dir, &retries, &draft, &json!({"card": {"id": "summary-1"}})).unwrap();
        f.store.ack(&retries).unwrap();
        std::fs::write(&file, "## Build\nRun tests.\n## Retries\nReuse the idempotency key for refunds too.\n").unwrap();
        let changed = import_with(&f.dir, &f.b, "backend", &mut f.store, &f.home, &[], now).unwrap();
        assert_eq!((changed["written"].clone(), changed["unchanged"].clone()), (json!(1), json!(1)));
        assert_eq!(draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store).await.unwrap(), (1, 0));
        let new: Vec<String> = f.store.pending_ids("draft:", "", now, 100).unwrap()
            .into_iter().filter(|id| !first.contains(id)).collect();
        assert_eq!(new.len(), 1);
        let v: Value = serde_json::from_slice(&f.store.read_record(&new[0], now).unwrap().unwrap()).unwrap();
        assert_eq!(v["memory"]["updates"], "summary-1");
        std::fs::remove_file(&file).unwrap();
        let scan = scan_with(&f.dir, &f.b, "backend", &f.home).unwrap();
        assert_eq!(scan["removed"], json!(["repo:backend/CLAUDE.md"]));
        server.abort();
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `./scripts/cargo test -p wk-agent --locked memory_import`
Expected: compile errors for `draft_item` and `submitted`.

- [ ] **Step 3: Implement `draft_item` and `submitted`**

Add after `draft_all` in `memory_import.rs`:

```rust
/// One `wk-agent drafts` row. Memory drafts also show where they came from.
pub(crate) fn draft_item(id: &str, v: &Value) -> Value {
    let mut item = json!({"id": id, "card": v["card"], "model": v["model"]});
    if let Some(m) = v.get("memory") {
        item["memory"] = json!({"label": m["label"], "heading": m["heading"], "updates": m["updates"]});
    }
    item
}
/// After a team submission, remember the summary so later edits link to it.
pub(crate) fn submitted(dir: &Path, draft_id: &str, draft: &Value, response: &Value) -> Result<(), String> {
    let (Some(key), Some(summary)) = (
        draft["memory"]["key"].as_str(),
        response["card"]["id"].as_str(),
    ) else {
        return Ok(());
    };
    let mut map = memory_map::load(dir)?;
    if let Some(e) = map.entries.get_mut(key) {
        if e.draft.as_deref() == Some(draft_id) {
            e.summary = Some(summary.into());
            e.draft = None;
            e.record = None;
        }
    }
    memory_map::save(dir, &map)
}
```

- [ ] **Step 4: Use them in `process.rs`**

In `list`, replace

```rust
        result.push(json!({"id":id,"card":v["card"],"model":v["model"]}));
```

with

```rust
        result.push(crate::memory_import::draft_item(&id, &v));
```

In `deliver`, directly after `let v: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid private draft")?;` and the `if owner_only { crate::personal_sync::payload(&v["card"])?; }` block, add:

```rust
    if v.get("memory").is_some() {
        if v["policy_hash"] != b.policy_hash || v["card"]["policy_revision"] != p["policy_revision"] {
            return Err("source or policy changed; run memory import again".into());
        }
        if wk_summarize::sensitive(&v["card"].to_string()) {
            return Err("draft requires sensitive-content review".into());
        }
        let response = post_draft(dir, &c, &b, &t, &v, owner_only).await?;
        if !owner_only {
            s.ack(id)?;
            // The server already accepted the summary; a failed map update
            // only loses the `updates` link for a later edit.
            let _ = crate::memory_import::submitted(dir, id, &v, &response);
        }
        return Ok(response);
    }
```

In `main.rs`, delete the four `#[allow(dead_code)] // ...` lines above `mod memory_split;`, `mod memory_sources;`, `mod memory_map;` and `mod memory_import;`.

- [ ] **Step 5: Run the full agent suite**

Run: `./scripts/cargo test -p wk-agent --locked`
Expected: all tests pass, including 7 in `memory_import`.

- [ ] **Step 6: Gate and commit**

```bash
./scripts/cargo fmt --all -- --check && ./scripts/cargo clippy -p wk-agent --all-targets --locked -- -D warnings
git add crates/agent/src/memory_import.rs crates/agent/src/process.rs crates/agent/src/main.rs
git commit -s -m "feat: show memory draft origins and link updates to submitted summaries"
```

---

### Task 7: Documentation

**Files:**
- Modify: `docs/adapters/setup.md` (new section after `## Instrumented chat and explicit exports`)
- Modify: `README.md` (Features table)
- Modify: `CHANGELOG.md` (topmost Unreleased section)

- [ ] **Step 1: Add the setup section**

Insert before `## Claude Desktop Enterprise` in `docs/adapters/setup.md`:

```markdown
## Import existing agent memory

Start with what your coding agents already know. `wk-agent --state-dir /absolute/private/state memory scan` lists the `CLAUDE.md`, `.claude/CLAUDE.md` and `AGENTS.md` files in your approved service folder and this project's Claude Code notes (`~/.claude/projects/<project>/memory/*.md`), with a status for each. It is offline and prints no file content. Workspace-root files are reported as outside the approved service.

Import with `memory import --yes`, adding `--file ABSOLUTE_PATH` for each Claude Code note you choose. The global `~/.claude/CLAUDE.md`, Codex's cross-project memories and other projects' notes are refused. Each note or top-level section becomes one encrypted private record; `wk-agent process` drafts it with your local model and the usual privacy screening, then `wk-agent drafts` shows each draft with its source. Nothing is shared until you submit a reviewed draft.

Re-running the import skips unchanged sections, replaces unsubmitted drafts of changed sections and links a changed, already-submitted section to its earlier summary (`updates`), so a reviewer can supersede it. Deleted sources are reported, not withdrawn. Memory files can contain credentials or personal notes: review every draft before submission.
```

- [ ] **Step 2: Add a README feature row**

In the Features table, after the `| **My context** | ... |` row, add:

```markdown
| **Agent memory import** | Turn existing `CLAUDE.md`, `AGENTS.md` and Claude Code project notes into private drafts for review ([setup](docs/adapters/setup.md#import-existing-agent-memory)) |
```

- [ ] **Step 3: Add a changelog entry**

Under the topmost `## Unreleased` heading's list, add:

```markdown
- `wk-agent memory scan|import`: private drafts from `CLAUDE.md`, `AGENTS.md` and explicitly approved Claude Code project notes, with re-import that skips unchanged sections and links updates to submitted summaries.
```

- [ ] **Step 4: Check links and commit**

Run: `python3 -m unittest discover -s scripts/tests -v`
Expected: `OK`

```bash
git add docs/adapters/setup.md README.md CHANGELOG.md
git commit -s -m "docs: document agent memory import"
```

---

### Task 8: Full verification

- [ ] **Step 1: Workspace gates**

```bash
./scripts/cargo fmt --all -- --check
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
./scripts/cargo test --workspace --locked
./scripts/cargo build --workspace --locked
python3 -m unittest discover -s scripts/tests -v
python3 scripts/demo.py
```

Expected: every command succeeds; the Rust test total is the previous count plus the new memory tests, with zero failures.

- [ ] **Step 2: Manual smoke test with a real CLI binary**

With a disposable enrolled state directory from `scripts/demo.py`-style setup, run `wk-agent --state-dir STATE memory scan` and confirm the JSON lists sources and statuses only, with no file text and no absolute paths.

- [ ] **Step 3: Record results**

Append the commands, counts and any skipped checks to the PR description. Report skipped checks honestly.
