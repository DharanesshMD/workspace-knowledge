//! `wk-agent memory scan|import`: turn agent memory into private draft
//! summaries. Nothing here publishes; submission stays explicit.

use crate::{
    daemon,
    memory_map::{self, Action, Entry},
    memory_sources::{self, Candidate},
    memory_split::{self, Section, SourceKind},
    network, process, state,
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
/// Point the map entry at `draft` unless a concurrent import replaced this
/// generation. Locks only around the map write, never across inference.
fn link(dir: &Path, record: &Record, draft: &str) -> Result<bool, String> {
    memory_map::update(dir, |map| {
        Ok(match map.entries.get_mut(&record.key) {
            Some(e) if e.generation == record.generation => {
                e.record = None;
                e.draft = Some(draft.to_string());
                true
            }
            _ => false,
        })
    })
}
/// Draft pending memory records. Mirrors `journal_turns`: approval is
/// re-checked around inference, and failed records stay pending until expiry.
/// A draft is linked in the map before its record is consumed, so an
/// interrupted run leaves the record for the next run to relink.
pub(crate) async fn draft_all(
    dir: &Path,
    c: &state::Config,
    b: &Binding,
    policy: &Value,
    store: &mut LocalStore,
) -> Result<(usize, usize), String> {
    let revision = policy["policy_revision"].as_u64().ok_or("invalid policy")?;
    let ids = store.next_ids("memory", "memory:", daemon::now()?, 10)?;
    let (mut drafts, mut held) = (0, 0);
    for id in ids {
        if process::processing_current(dir, c, b).is_err() {
            break;
        }
        let Some(bytes) = store.read_record(&id, daemon::now()?)? else {
            continue;
        };
        let record = match serde_json::from_slice::<Record>(&bytes) {
            Ok(r) if r.version == 1 && r.policy_hash == b.policy_hash => r,
            // Unreadable, or approval changed since import: the developer must
            // import again. One bad record never blocks the others.
            _ => {
                store.ack(&id)?;
                held += 1;
                continue;
            }
        };
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
            // An interrupted run may have stored this draft without linking it.
            if store.contains(&key, daemon::now()?)? && !link(dir, &record, &key)? {
                store.ack(&key)?;
            }
            store.ack(&id)?;
            continue;
        }
        match wk_summarize::extract(&c.inference, &c.model, &record.source()).await {
            Ok(extract) => {
                if process::processing_current(dir, c, b).is_err() {
                    break;
                }
                let card =
                    wk_summarize::card(extract, hash, revision, vec![c.service.clone()], true)?;
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
                if !link(dir, &record, &key)? {
                    // A concurrent import replaced this generation; drop the stale draft.
                    store.ack(&key)?;
                    store.ack(&id)?;
                    continue;
                }
                store.ack(&id)?;
                drafts += 1;
            }
            Err(_) => held += 1,
        }
    }
    Ok((drafts, held))
}
/// One `wk-agent drafts` row. Memory drafts also show where they came from.
pub(crate) fn draft_item(id: &str, v: &Value) -> Value {
    let mut item = json!({"id": id, "card": v["card"], "model": v["model"]});
    if let Some(m) = v.get("memory") {
        item["memory"] =
            json!({"label": m["label"], "heading": m["heading"], "updates": m["updates"]});
    }
    item
}
/// After a team submission, remember the summary so later edits link to it.
pub(crate) fn submitted(
    dir: &Path,
    draft_id: &str,
    draft: &Value,
    response: &Value,
) -> Result<(), String> {
    let (Some(key), Some(summary)) = (
        draft["memory"]["key"].as_str(),
        response["card"]["id"].as_str(),
    ) else {
        return Ok(());
    };
    memory_map::update(dir, |map| {
        if let Some(e) = map.entries.get_mut(key) {
            if e.draft.as_deref() == Some(draft_id) {
                e.summary = Some(summary.into());
                e.draft = None;
                e.record = None;
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
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
    async fn model() -> (String, tokio::task::JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
        model_flagging(false).await
    }
    /// `sensitive`: the model flags every summary, so inference fails.
    async fn model_flagging(
        sensitive: bool,
    ) -> (String, tokio::task::JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
        let sources = Arc::new(Mutex::new(Vec::new()));
        let seen = sources.clone();
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(
                        body["messages"][1]["content"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                    );
                    let summary = json!({
                        "title": "Imported backend convention",
                        "intent": "Keep a recorded backend convention available to teammates",
                        "rationale": "The developer's agent memory recorded this convention",
                        "decisions": ["Follow the recorded convention for backend changes"],
                        "constraints": [], "open_questions": [], "sensitive": sensitive
                    });
                    axum::Json(json!({"choices": [{"message": {"content": summary.to_string()}}]}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        (
            url,
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
            sources,
        )
    }
    #[tokio::test]
    async fn process_drafts_records_once_without_absolute_paths() {
        let (url, server, sources) = model().await;
        let mut f = fixture(&url);
        std::fs::write(
            f.b.root.join("backend/CLAUDE.md"),
            "## Build\nRun tests.\n## Retries\nReuse the idempotency key.\n",
        )
        .unwrap();
        let note = memory_sources::notes_dir(&f.home, &f.b.root).join("deploy.md");
        std::fs::write(
            &note,
            "---\nname: deploy\n---\nDeploys happen after the Tuesday sync.\n",
        )
        .unwrap();
        let now = daemon::now().unwrap();
        import_with(
            &f.dir,
            &f.b,
            "backend",
            &mut f.store,
            Some(&f.home),
            &[note],
            now,
        )
        .unwrap();
        let policy = json!({"policy_revision": 1});
        assert_eq!(
            draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store)
                .await
                .unwrap(),
            (3, 0)
        );
        assert!(
            f.store
                .pending_ids("memory:", "", now, 100)
                .unwrap()
                .is_empty(),
            "records consumed"
        );
        let drafts = f.store.pending_ids("draft:", "", now, 100).unwrap();
        assert_eq!(drafts.len(), 3);
        let base = f.dir.parent().unwrap().to_string_lossy().to_string();
        for id in &drafts {
            let draft: Value =
                serde_json::from_slice(&f.store.read_record(id, now).unwrap().unwrap()).unwrap();
            assert_eq!(draft["card"]["capture_completeness"], "complete");
            assert_eq!(draft["card"]["service_ids"], json!(["backend"]));
            assert!(
                !draft.to_string().contains(&base),
                "no absolute path in drafts"
            );
        }
        let sent = sources.lock().unwrap().clone();
        assert_eq!(sent.len(), 3);
        assert!(sent
            .iter()
            .all(|s| !s.contains(&base) && s.starts_with("Agent memory: ")));
        let map = memory_map::load(&f.dir).unwrap();
        assert!(map
            .entries
            .values()
            .all(|e| e.record.is_none() && e.draft.is_some()));
        assert_eq!(
            draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store)
                .await
                .unwrap(),
            (0, 0)
        );
        server.abort();
    }
    #[tokio::test]
    async fn drafts_for_a_replaced_generation_are_dropped() {
        let (url, server, _) = model().await;
        let mut f = fixture(&url);
        std::fs::write(
            f.b.root.join("backend/CLAUDE.md"),
            "## Build\nRun tests.\n## Retries\nReuse the idempotency key.\n",
        )
        .unwrap();
        let now = daemon::now().unwrap();
        import(&mut f, &[], now).unwrap();
        // A concurrent import replaces this section while the model is drafting.
        let key = first_key(BACKEND, "## Build\nRun tests.\n");
        memory_map::update(&f.dir, |m| {
            let e = m.entries.get_mut(&key).unwrap();
            e.generation += 1;
            Ok(())
        })
        .unwrap();
        let policy = json!({"policy_revision": 1});
        assert_eq!(
            draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store)
                .await
                .unwrap(),
            (1, 0)
        );
        assert!(f
            .store
            .pending_ids("memory:", "", now, 100)
            .unwrap()
            .is_empty());
        let drafts = f.store.pending_ids("draft:", "", now, 100).unwrap();
        assert_eq!(drafts.len(), 1, "only the other section's draft remains");
        assert!(
            !f.store.contains(&record_id(&key, 1).unwrap(), now).unwrap(),
            "the replaced generation's record is consumed"
        );
        let map = memory_map::load(&f.dir).unwrap();
        let replaced = &map.entries[&key];
        assert_eq!(replaced.draft, None);
        assert!(
            replaced.record.is_some(),
            "the newer generation stays tracked"
        );
        let other = map.entries.iter().find(|(k, _)| **k != key).unwrap().1;
        assert_eq!(other.draft.as_deref(), Some(drafts[0].as_str()));
        server.abort();
    }
    #[tokio::test]
    async fn drafts_left_behind_by_an_interrupted_run_are_relinked() {
        let (url, server, sources) = model().await;
        let mut f = fixture(&url);
        std::fs::write(f.b.root.join("backend/CLAUDE.md"), "## Build\nRun tests.\n").unwrap();
        let now = daemon::now().unwrap();
        import(&mut f, &[], now).unwrap();
        let key = first_key(BACKEND, "## Build\nRun tests.\n");
        let id = record_id(&key, 1).unwrap();
        let bytes = f.store.read_record(&id, now).unwrap().unwrap();
        let policy = json!({"policy_revision": 1});
        assert_eq!(
            draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store)
                .await
                .unwrap(),
            (1, 0)
        );
        let draft = memory_map::load(&f.dir).unwrap().entries[&key]
            .draft
            .clone()
            .unwrap();
        // Simulate a run interrupted after storing the draft: the record is
        // still pending and the map still points at it.
        assert!(f.store.enqueue(&id, &bytes, now, RECORD_TTL).unwrap());
        memory_map::update(&f.dir, |m| {
            let e = m.entries.get_mut(&key).unwrap();
            e.record = Some(id.clone());
            e.draft = None;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store)
                .await
                .unwrap(),
            (0, 0)
        );
        assert!(!f.store.contains(&id, now).unwrap(), "record consumed");
        assert!(f.store.contains(&draft, now).unwrap(), "draft kept");
        let e = &memory_map::load(&f.dir).unwrap().entries[&key];
        assert_eq!(
            (e.record.as_deref(), e.draft.as_deref()),
            (None, Some(draft.as_str()))
        );
        assert_eq!(sources.lock().unwrap().len(), 1, "no second inference");
        server.abort();
    }
    fn one_section(f: &mut Fixture, now: i64) -> (String, String) {
        std::fs::write(f.b.root.join("backend/CLAUDE.md"), "## Build\nRun tests.\n").unwrap();
        import(f, &[], now).unwrap();
        let key = first_key(BACKEND, "## Build\nRun tests.\n");
        let id = record_id(&key, 1).unwrap();
        (key, id)
    }
    async fn draft(f: &mut Fixture) -> Result<(usize, usize), String> {
        let policy = json!({"policy_revision": 1});
        draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store).await
    }
    #[tokio::test]
    async fn failed_map_links_keep_the_record_for_the_next_run() {
        let (url, server, sources) = model().await;
        let mut f = fixture(&url);
        let now = daemon::now().unwrap();
        let (key, id) = one_section(&mut f, now);
        let map = std::fs::read(f.dir.join(memory_map::FILE)).unwrap();
        std::fs::write(f.dir.join(memory_map::FILE), "{not json").unwrap();
        assert!(draft(&mut f).await.is_err());
        assert!(f.store.contains(&id, now).unwrap(), "record still pending");
        let drafts = f.store.pending_ids("draft:", "", now, 100).unwrap();
        assert_eq!(drafts.len(), 1, "the unlinked draft is kept");
        std::fs::write(f.dir.join(memory_map::FILE), map).unwrap();
        assert_eq!(draft(&mut f).await.unwrap(), (0, 0));
        assert!(!f.store.contains(&id, now).unwrap(), "record consumed");
        let e = &memory_map::load(&f.dir).unwrap().entries[&key];
        assert_eq!(
            (e.record.as_deref(), e.draft.as_deref()),
            (None, Some(drafts[0].as_str()))
        );
        assert_eq!(sources.lock().unwrap().len(), 1, "no second inference");
        server.abort();
    }
    #[tokio::test]
    async fn undecodable_and_unknown_version_records_are_dropped_and_held() {
        let (url, server, _) = model().await;
        let mut f = fixture(&url);
        let now = daemon::now().unwrap();
        let (_, id) = one_section(&mut f, now);
        let mut future = record(&f, &id, now);
        future.version = 2;
        let junk = format!("memory:{}", "0".repeat(64));
        let versioned = format!("memory:{}", "1".repeat(64));
        assert!(f.store.enqueue(&junk, b"junk", now, RECORD_TTL).unwrap());
        let bytes = serde_json::to_vec(&future).unwrap();
        assert!(f
            .store
            .enqueue(&versioned, &bytes, now, RECORD_TTL)
            .unwrap());
        assert_eq!(draft(&mut f).await.unwrap(), (1, 2));
        assert!(f
            .store
            .pending_ids("memory:", "", now, 100)
            .unwrap()
            .is_empty());
        assert_eq!(
            f.store.pending_ids("draft:", "", now, 100).unwrap().len(),
            1
        );
        server.abort();
    }
    #[tokio::test]
    async fn records_imported_under_another_approval_are_dropped_and_held() {
        let (url, server, sources) = model().await;
        let mut f = fixture(&url);
        let now = daemon::now().unwrap();
        let (_, id) = one_section(&mut f, now);
        let mut stale = record(&f, &id, now);
        stale.policy_hash = "older-approval".into();
        f.store.ack(&id).unwrap();
        let bytes = serde_json::to_vec(&stale).unwrap();
        assert!(f.store.enqueue(&id, &bytes, now, RECORD_TTL).unwrap());
        assert_eq!(draft(&mut f).await.unwrap(), (0, 1));
        assert!(!f.store.contains(&id, now).unwrap(), "record dropped");
        assert!(f
            .store
            .pending_ids("draft:", "", now, 100)
            .unwrap()
            .is_empty());
        assert!(
            sources.lock().unwrap().is_empty(),
            "never sent to the model"
        );
        server.abort();
    }
    #[tokio::test]
    async fn model_failures_keep_the_record_and_write_no_draft() {
        let (url, server, _) = model_flagging(true).await;
        let mut f = fixture(&url);
        let now = daemon::now().unwrap();
        let (_, id) = one_section(&mut f, now);
        assert_eq!(draft(&mut f).await.unwrap(), (0, 1));
        assert!(f.store.contains(&id, now).unwrap(), "record still pending");
        assert!(f
            .store
            .pending_ids("draft:", "", now, 100)
            .unwrap()
            .is_empty());
        server.abort();
    }
    #[tokio::test]
    async fn invalid_policies_fail_before_any_record_is_read() {
        let (url, server, sources) = model().await;
        let mut f = fixture(&url);
        let now = daemon::now().unwrap();
        let (_, id) = one_section(&mut f, now);
        let bad = json!({});
        assert_eq!(
            draft_all(&f.dir, &f.c, &f.b, &bad, &mut f.store).await,
            Err("invalid policy".into())
        );
        assert!(f.store.contains(&id, now).unwrap());
        assert!(sources.lock().unwrap().is_empty());
        server.abort();
    }
    #[test]
    fn draft_items_show_memory_origin_without_internal_keys() {
        let v = json!({"card": {"title": "T"}, "model": "m",
            "memory": {"key": "k", "generation": 2, "label": "claude-memory:deploy.md",
                       "heading": [], "updates": "summary-1"}});
        let item = draft_item("draft:x", &v);
        assert_eq!(
            item["memory"],
            json!({"label": "claude-memory:deploy.md", "heading": [], "updates": "summary-1"})
        );
        assert!(draft_item("draft:y", &json!({"card": {}, "model": "m"}))
            .get("memory")
            .is_none());
    }
    #[tokio::test]
    async fn submitted_summaries_link_the_next_changed_section() {
        let (url, server, _sources) = model().await;
        let mut f = fixture(&url);
        let file = f.b.root.join("backend/CLAUDE.md");
        std::fs::write(
            &file,
            "## Build\nRun tests.\n## Retries\nReuse the idempotency key.\n",
        )
        .unwrap();
        let now = daemon::now().unwrap();
        let policy = json!({"policy_revision": 1});
        import_with(
            &f.dir,
            &f.b,
            "backend",
            &mut f.store,
            Some(&f.home),
            &[],
            now,
        )
        .unwrap();
        draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store)
            .await
            .unwrap();
        let first = f.store.pending_ids("draft:", "", now, 100).unwrap();
        let retries = first
            .iter()
            .find(|id| {
                let v: Value =
                    serde_json::from_slice(&f.store.read_record(id, now).unwrap().unwrap())
                        .unwrap();
                v["memory"]["heading"] == json!(["Retries"])
            })
            .unwrap()
            .clone();
        let draft: Value =
            serde_json::from_slice(&f.store.read_record(&retries, now).unwrap().unwrap()).unwrap();
        submitted(
            &f.dir,
            &retries,
            &draft,
            &json!({"card": {"id": "summary-1"}}),
        )
        .unwrap();
        f.store.ack(&retries).unwrap();
        std::fs::write(
            &file,
            "## Build\nRun tests.\n## Retries\nReuse the idempotency key for refunds too.\n",
        )
        .unwrap();
        let changed = import_with(
            &f.dir,
            &f.b,
            "backend",
            &mut f.store,
            Some(&f.home),
            &[],
            now,
        )
        .unwrap();
        assert_eq!(
            (changed["written"].clone(), changed["unchanged"].clone()),
            (json!(1), json!(1))
        );
        assert_eq!(
            draft_all(&f.dir, &f.c, &f.b, &policy, &mut f.store)
                .await
                .unwrap(),
            (1, 0)
        );
        let new: Vec<String> = f
            .store
            .pending_ids("draft:", "", now, 100)
            .unwrap()
            .into_iter()
            .filter(|id| !first.contains(id))
            .collect();
        assert_eq!(new.len(), 1);
        let v: Value =
            serde_json::from_slice(&f.store.read_record(&new[0], now).unwrap().unwrap()).unwrap();
        assert_eq!(v["memory"]["updates"], "summary-1");
        std::fs::remove_file(&file).unwrap();
        let scan = scan_with(&f.dir, &f.b, "backend", Some(&f.home)).unwrap();
        assert_eq!(scan["removed"], json!(["repo:backend/CLAUDE.md"]));
        server.abort();
    }
    fn linked_entry(f: &Fixture) -> memory_map::Map {
        memory_map::update(&f.dir, |m| {
            m.entries.insert(
                "k".into(),
                Entry {
                    label: "repo:backend/CLAUDE.md".into(),
                    content: "c".into(),
                    generation: 1,
                    record: None,
                    draft: Some("draft:current".into()),
                    summary: None,
                },
            );
            Ok(())
        })
        .unwrap();
        memory_map::load(&f.dir).unwrap()
    }
    #[test]
    fn submissions_of_a_replaced_draft_leave_the_map_unchanged() {
        let f = fixture("http://127.0.0.1:9");
        let before = linked_entry(&f);
        let draft = json!({"memory": {"key": "k"}});
        let response = json!({"card": {"id": "summary-1"}});
        submitted(&f.dir, "draft:other", &draft, &response).unwrap();
        assert_eq!(memory_map::load(&f.dir).unwrap(), before);
    }
    #[test]
    fn responses_without_a_summary_id_leave_the_map_unchanged() {
        let f = fixture("http://127.0.0.1:9");
        let before = linked_entry(&f);
        let draft = json!({"memory": {"key": "k"}});
        assert_eq!(
            submitted(&f.dir, "draft:current", &draft, &json!({"card": {}})),
            Ok(())
        );
        assert_eq!(memory_map::load(&f.dir).unwrap(), before);
    }
}
