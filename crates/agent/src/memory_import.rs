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
pub(crate) fn scan_with(
    dir: &Path,
    b: &Binding,
    service: &str,
    home: &Path,
) -> Result<Value, String> {
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
    import_with(
        dir,
        &b,
        &c.service,
        &mut store,
        &home()?,
        files,
        daemon::now()?,
    )
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
                    if !store.enqueue(&id, &bytes, now, 86400)? {
                        store.ack(&id)?;
                        if !store.enqueue(&id, &bytes, now, 86400)? {
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
            &f.home,
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
        let again =
            import_with(&f.dir, &f.b, "backend", &mut f.store, &f.home, &[note], now).unwrap();
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
                &f.home,
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
            import_with(&f.dir, &f.b, "backend", &mut f.store, &f.home, &[], now),
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
}
