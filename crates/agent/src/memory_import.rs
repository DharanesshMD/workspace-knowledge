//! `wk-agent memory scan|import`: turn agent memory into private draft
//! summaries. Nothing here publishes; submission stays explicit.

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

pub(crate) const MAX_SECTIONS: usize = 200;
/// The store's maximum retention; `process` drafts records well within it.
const RECORD_TTL: i64 = 86_400;

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
/// Without a home folder there are no Claude Code notes; repository files still work.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}
/// An explicit `--file`. Without a home folder only a service-folder
/// instruction file, named by its exact path, can be approved.
fn approve(
    home: Option<&Path>,
    b: &Binding,
    service: &str,
    file: &Path,
) -> Result<Candidate, String> {
    match home {
        Some(home) => memory_sources::approve_file(home, b, service, file),
        None => memory_sources::repository(b, service)
            .into_iter()
            .find(|c| c.rejected.is_none() && c.path == file)
            .ok_or("not_a_memory_source"),
    }
    .map_err(|reason| format!("memory_source_rejected:{reason}"))
}
pub(crate) async fn command(
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
    scan_with(dir, &b, &c.service, home().as_deref())
}
pub(crate) fn scan_with(
    dir: &Path,
    b: &Binding,
    service: &str,
    home: Option<&Path>,
) -> Result<Value, String> {
    let map = memory_map::load(dir)?;
    let mut rows = Vec::new();
    let mut present = BTreeSet::new();
    let candidates = memory_sources::repository(b, service)
        .into_iter()
        .chain(home.map_or_else(Vec::new, |home| memory_sources::notes(home, &b.root)));
    for candidate in candidates {
        // Still on disk, so not removed, even if now rejected.
        present.insert(candidate.label.clone());
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
    import_with(
        dir,
        &b,
        &c.service,
        &mut store,
        home().as_deref(),
        files,
        daemon::now()?,
    )
}
/// Read every source and count sections before writing anything. Unreadable
/// service-folder files are skipped and counted; a named `--file` that cannot
/// be approved or read fails the whole run.
pub(crate) fn import_with(
    dir: &Path,
    b: &Binding,
    service: &str,
    store: &mut LocalStore,
    home: Option<&Path>,
    files: &[PathBuf],
    now: i64,
) -> Result<Value, String> {
    // Workspace-root files are always rejected by design and not counted.
    let service_prefix = format!("repo:{service}/");
    let mut sources: Vec<(Candidate, String)> = Vec::new();
    let mut rejected = 0;
    for candidate in memory_sources::repository(b, service) {
        if !candidate.label.starts_with(&service_prefix) {
            continue;
        }
        match candidate
            .rejected
            .map_or_else(|| memory_sources::read(&candidate), Err)
        {
            Ok(text) => sources.push((candidate, text)),
            Err(_) => rejected += 1,
        }
    }
    for file in files {
        let approved = approve(home, b, service, file)?;
        if sources.iter().any(|(s, _)| s.path == approved.path) {
            continue;
        }
        let text = memory_sources::read(&approved)
            .map_err(|reason| format!("memory_source_rejected:{reason}"))?;
        sources.push((approved, text));
    }
    let mut prepared = Vec::new();
    for (source, text) in &sources {
        for section in memory_split::split(source.kind, text) {
            prepared.push((source.label.clone(), section));
        }
    }
    if prepared.len() > MAX_SECTIONS {
        return Err("too_many_memory_sections".into());
    }
    let (written, unchanged) = memory_map::update(dir, |map| {
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
            match memory_map::decide(entry, &content, pending)? {
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
                    // An interrupted earlier run may have left this identity behind
                    // without recording it; replace that stale, undrafted record.
                    if !store.enqueue(&id, &bytes, now, RECORD_TTL)? {
                        store.ack(&id)?;
                        if !store.enqueue(&id, &bytes, now, RECORD_TTL)? {
                            return Err("memory record identity unavailable".into());
                        }
                    }
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
        Ok((written, unchanged))
    })?;
    Ok(json!({
        "written": written,
        "unchanged": unchanged,
        "rejected": rejected,
        "sources": sources.len(),
        "published": false,
        "next": "run `wk-agent process`, then review `wk-agent drafts`",
    }))
}

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
        Fixture {
            _temp: temp,
            dir,
            home,
            c,
            b,
            store,
        }
    }
    fn rows(scan: &Value) -> Vec<(String, String)> {
        scan["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["label"].as_str().unwrap().into(),
                    r["status"].as_str().unwrap().into(),
                )
            })
            .collect()
    }
    #[test]
    fn scan_reports_sources_without_reading_into_output() {
        let f = fixture("http://127.0.0.1:9/v1/chat/completions");
        std::fs::write(f.b.root.join("CLAUDE.md"), "# Root\nOutside the service.\n").unwrap();
        std::fs::write(
            f.b.root.join("backend/CLAUDE.md"),
            "## Build\nRun tests.\n## Style\nUse rustfmt.\n",
        )
        .unwrap();
        let notes = memory_sources::notes_dir(&f.home, &f.b.root);
        std::fs::write(
            notes.join("deploy.md"),
            "Deploys happen after the Tuesday sync.\n",
        )
        .unwrap();
        let scan = scan_with(&f.dir, &f.b, "backend", Some(f.home.as_path())).unwrap();
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
        assert!(!scan.to_string().contains(&*f.home.to_string_lossy()));
    }
    #[test]
    fn import_writes_private_records_and_skips_unchanged_sections() {
        let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
        std::fs::write(
            f.b.root.join("backend/CLAUDE.md"),
            "## Build\nRun tests.\n## Style\nUse rustfmt.\n",
        )
        .unwrap();
        let note = memory_sources::notes_dir(&f.home, &f.b.root).join("deploy.md");
        std::fs::write(&note, "Deploys happen after the Tuesday sync.\n").unwrap();
        let now = daemon::now().unwrap();
        let first = import_with(
            &f.dir,
            &f.b,
            "backend",
            &mut f.store,
            Some(f.home.as_path()),
            std::slice::from_ref(&note),
            now,
        )
        .unwrap();
        assert_eq!(
            (first["written"].clone(), first["unchanged"].clone()),
            (json!(3), json!(0))
        );
        let records = f.store.pending_ids("memory:", "", now, 100).unwrap();
        assert_eq!(records.len(), 3);
        let raw = std::fs::read(f.dir.join("test.db")).unwrap();
        assert!(
            !String::from_utf8_lossy(&raw).contains("Tuesday"),
            "records are encrypted"
        );
        let map = std::fs::read_to_string(f.dir.join(memory_map::FILE)).unwrap();
        assert!(
            !map.contains("Tuesday") && !map.contains("rustfmt"),
            "map holds no section text"
        );
        let again = import_with(
            &f.dir,
            &f.b,
            "backend",
            &mut f.store,
            Some(f.home.as_path()),
            &[note],
            now,
        )
        .unwrap();
        assert_eq!(
            (again["written"].clone(), again["unchanged"].clone()),
            (json!(0), json!(3))
        );
    }
    #[test]
    fn rejected_files_and_oversized_runs_write_nothing() {
        let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
        let now = daemon::now().unwrap();
        std::fs::create_dir_all(f.home.join(".claude")).unwrap();
        std::fs::write(f.home.join(".claude/CLAUDE.md"), "global").unwrap();
        assert_eq!(
            import_with(
                &f.dir,
                &f.b,
                "backend",
                &mut f.store,
                Some(f.home.as_path()),
                &[f.home.join(".claude/CLAUDE.md")],
                now
            ),
            Err("memory_source_rejected:not_a_memory_source".into())
        );
        let many: String = (0..=MAX_SECTIONS)
            .map(|i| format!("## S{i}\nBody {i}.\n"))
            .collect();
        std::fs::write(f.b.root.join("backend/AGENTS.md"), many).unwrap();
        assert_eq!(
            import_with(
                &f.dir,
                &f.b,
                "backend",
                &mut f.store,
                Some(f.home.as_path()),
                &[],
                now
            ),
            Err("too_many_memory_sections".into())
        );
        assert!(f
            .store
            .pending_ids("memory:", "", now, 100)
            .unwrap()
            .is_empty());
        assert!(!f.dir.join(memory_map::FILE).exists());
    }
    #[tokio::test]
    async fn unknown_actions_are_refused() {
        let f = fixture("http://127.0.0.1:9/v1/chat/completions");
        assert!(command(&f.dir, "publish", true, &[]).await.is_err());
    }

    const BACKEND: &str = "repo:backend/CLAUDE.md";
    fn import(f: &mut Fixture, files: &[PathBuf], now: i64) -> Result<Value, String> {
        import_with(
            &f.dir,
            &f.b,
            "backend",
            &mut f.store,
            Some(f.home.as_path()),
            files,
            now,
        )
    }
    fn first_key(label: &str, text: &str) -> String {
        let sections = memory_split::split(SourceKind::Instructions, text);
        section_key(label, &sections[0]).unwrap()
    }
    fn record(f: &Fixture, id: &str, now: i64) -> Record {
        serde_json::from_slice(&f.store.read_record(id, now).unwrap().unwrap()).unwrap()
    }
    #[test]
    fn unreadable_service_files_are_skipped_and_counted() {
        let now = daemon::now().unwrap();
        let oversized = vec![b'a'; memory_sources::MAX_FILE_BYTES as usize + 1];
        for agents in [oversized, vec![0xff, 0xfe, b'\n']] {
            let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
            std::fs::write(f.b.root.join("CLAUDE.md"), "# Root\nNot counted.\n").unwrap();
            std::fs::write(
                f.b.root.join("backend/CLAUDE.md"),
                "## Build\nRun tests.\n## Style\nUse rustfmt.\n",
            )
            .unwrap();
            std::fs::write(f.b.root.join("backend/AGENTS.md"), agents).unwrap();
            let out = import(&mut f, &[], now).unwrap();
            assert_eq!(
                (
                    out["written"].clone(),
                    out["rejected"].clone(),
                    out["sources"].clone()
                ),
                (json!(2), json!(1), json!(1))
            );
            assert_eq!(
                f.store.pending_ids("memory:", "", now, 100).unwrap().len(),
                2
            );
        }
    }
    #[test]
    fn missing_home_still_imports_and_scans_repository_files() {
        let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
        let claude = f.b.root.join("backend/CLAUDE.md");
        std::fs::write(&claude, "## Build\nRun tests.\n").unwrap();
        let note = memory_sources::notes_dir(&f.home, &f.b.root).join("deploy.md");
        std::fs::write(&note, "Deploys happen after the Tuesday sync.\n").unwrap();
        let now = daemon::now().unwrap();
        let none = |f: &mut Fixture, files: &[PathBuf]| {
            import_with(&f.dir, &f.b, "backend", &mut f.store, None, files, now)
        };
        assert_eq!(
            none(&mut f, std::slice::from_ref(&note)),
            Err("memory_source_rejected:not_a_memory_source".into())
        );
        let out = none(&mut f, std::slice::from_ref(&claude)).unwrap();
        assert_eq!(out["written"], json!(1));
        let scan = scan_with(&f.dir, &f.b, "backend", None).unwrap();
        assert_eq!(rows(&scan), vec![(BACKEND.into(), "unchanged".into())]);
    }
    #[test]
    fn changed_sections_retire_old_records_and_link_the_summary() {
        let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
        let path = f.b.root.join("backend/CLAUDE.md");
        std::fs::write(&path, "## Build\nRun tests.\n").unwrap();
        let now = daemon::now().unwrap();
        import(&mut f, &[], now).unwrap();
        let key = first_key(BACKEND, "## Build\nRun tests.\n");
        let old = record_id(&key, 1).unwrap();
        assert!(f.store.contains(&old, now).unwrap());
        memory_map::update(&f.dir, |m| {
            m.entries.get_mut(&key).unwrap().summary = Some("s1".into());
            Ok(())
        })
        .unwrap();
        std::fs::write(&path, "## Build\nRun all tests.\n").unwrap();
        let out = import(&mut f, &[], now).unwrap();
        assert_eq!(out["written"], json!(1));
        assert!(!f.store.contains(&old, now).unwrap());
        let new = record_id(&key, 2).unwrap();
        let r = record(&f, &new, now);
        assert_eq!((r.generation, r.updates.as_deref()), (2, Some("s1")));
        let map = memory_map::load(&f.dir).unwrap();
        let e = &map.entries[&key];
        assert_eq!(
            (e.generation, e.record.as_deref(), e.summary.as_deref()),
            (2, Some(new.as_str()), Some("s1"))
        );
    }
    #[test]
    fn stale_records_with_the_same_identity_are_replaced() {
        let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
        std::fs::write(f.b.root.join("backend/CLAUDE.md"), "## Build\nRun tests.\n").unwrap();
        let now = daemon::now().unwrap();
        let id = record_id(&first_key(BACKEND, "## Build\nRun tests.\n"), 1).unwrap();
        assert!(f.store.enqueue(&id, b"junk", now, RECORD_TTL).unwrap());
        assert_eq!(import(&mut f, &[], now).unwrap()["written"], json!(1));
        let r = record(&f, &id, now);
        assert_eq!(
            (r.label.as_str(), r.heading.clone(), r.body.as_str()),
            (BACKEND, vec!["Build".to_string()], "Run tests.")
        );
    }
    #[test]
    fn scan_reports_unchanged_changed_and_removed_sources() {
        let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
        let path = f.b.root.join("backend/CLAUDE.md");
        std::fs::write(&path, "## Build\nRun tests.\n").unwrap();
        let note = memory_sources::notes_dir(&f.home, &f.b.root).join("deploy.md");
        std::fs::write(&note, "Deploys happen after the Tuesday sync.\n").unwrap();
        let now = daemon::now().unwrap();
        import(&mut f, std::slice::from_ref(&note), now).unwrap();
        let scan = || scan_with(&f.dir, &f.b, "backend", Some(f.home.as_path())).unwrap();
        assert_eq!(
            rows(&scan()),
            vec![
                (BACKEND.into(), "unchanged".into()),
                ("claude-memory:deploy.md".into(), "unchanged".into())
            ]
        );
        std::fs::write(&path, "## Build\nRun all tests.\n").unwrap();
        assert_eq!(rows(&scan())[0], (BACKEND.into(), "changed".into()));
        std::fs::remove_file(&note).unwrap();
        let after = scan();
        assert_eq!(after["removed"], json!(["claude-memory:deploy.md"]));
    }
    #[test]
    fn rejected_sources_that_still_exist_are_not_removed() {
        let mut f = fixture("http://127.0.0.1:9/v1/chat/completions");
        let path = f.b.root.join("backend/CLAUDE.md");
        std::fs::write(&path, "## Build\nRun tests.\n").unwrap();
        let now = daemon::now().unwrap();
        import(&mut f, &[], now).unwrap();
        std::fs::write(&path, vec![0xff, 0xfe]).unwrap();
        let scan = scan_with(&f.dir, &f.b, "backend", Some(f.home.as_path())).unwrap();
        assert_eq!(rows(&scan), vec![(BACKEND.into(), "rejected".into())]);
        assert_eq!(scan["removed"], json!([]));
    }
    #[tokio::test]
    async fn import_without_confirmation_writes_nothing() {
        let f = fixture("http://127.0.0.1:9/v1/chat/completions");
        assert!(command(&f.dir, "import", false, &[]).await.is_err());
        assert!(!f.dir.join(memory_map::FILE).exists());
    }
}
