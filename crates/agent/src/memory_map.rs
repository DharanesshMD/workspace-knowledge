//! Content-free record of imported memory sections, kept in an owner-only
//! state file because the encrypted store retains records for at most a day.

use crate::state;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

pub const FILE: &str = "memory-import.json";
const LOCK: &str = "memory-import.lock";

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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
pub fn decide(entry: Option<&Entry>, content: &str, pending: bool) -> Result<Action, String> {
    let Some(e) = entry else {
        return Ok(Action::Write {
            generation: 1,
            updates: None,
            retire: vec![],
        });
    };
    let submitted = e.record.is_none() && e.draft.is_none() && e.summary.is_some();
    if e.content == content && (pending || submitted) {
        return Ok(Action::Skip);
    }
    let generation = e
        .generation
        .checked_add(1)
        .ok_or("invalid memory import map")?;
    // Unchanged content here means an explicit re-run after the work expired
    // unsubmitted: redraft under a new identity, with nothing left to retire.
    let retire = if e.content == content {
        vec![]
    } else {
        e.record.iter().chain(e.draft.iter()).cloned().collect()
    };
    Ok(Action::Write {
        generation,
        updates: e.summary.clone(),
        retire,
    })
}
pub fn load(dir: &Path) -> Result<Map, String> {
    let path = dir.join(FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::default()),
        Err(_) => return Err("memory import map unavailable".into()),
    }
    let map: Map = serde_json::from_str(&state::read(&path, 4 * 1024 * 1024)?)
        .map_err(|_| "invalid memory import map")?;
    if map.version != 1 {
        return Err("unsupported memory import map".into());
    }
    Ok(map)
}
/// The only writer: holds `memory-import.lock` across load, `f` and save so
/// separate processes never lose each other's updates. Saves only if `f` succeeds.
pub fn update<R>(dir: &Path, f: impl FnOnce(&mut Map) -> Result<R, String>) -> Result<R, String> {
    let _lock = acquire_lock(dir)?;
    let mut map = load(dir)?;
    let out = f(&mut map)?;
    save(dir, &map)?;
    Ok(out)
}
fn acquire_lock(dir: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(dir.join(LOCK))
        .map_err(|_| "memory import map unavailable")?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(_) => return Err("memory import map unavailable".into()),
        }
        if Instant::now() >= deadline {
            return Err("memory import map unavailable".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
/// `state::save` writes atomically through an owner-only temporary file.
fn save(dir: &Path, map: &Map) -> Result<(), String> {
    state::save(&dir.join(FILE), map)
}

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
    fn write(generation: u32, updates: Option<&str>, retire: &[&str]) -> Result<Action, String> {
        Ok(Action::Write {
            generation,
            updates: updates.map(Into::into),
            retire: retire.iter().map(|s| s.to_string()).collect(),
        })
    }
    #[test]
    fn new_sections_start_at_generation_one() {
        assert_eq!(decide(None, "c1", false), write(1, None, &[]));
    }
    #[test]
    fn unchanged_pending_or_submitted_sections_are_skipped() {
        assert_eq!(
            decide(Some(&entry(None, Some("draft:a"), None)), "c1", true),
            Ok(Action::Skip)
        );
        assert_eq!(
            decide(Some(&entry(None, None, Some("s1"))), "c1", false),
            Ok(Action::Skip)
        );
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
    fn unchanged_expired_record_is_rewritten_without_retiring_it() {
        assert_eq!(
            decide(Some(&entry(Some("memory:r"), None, None)), "c1", false),
            write(3, None, &[])
        );
    }
    #[test]
    fn unchanged_entry_with_no_work_and_no_summary_is_rewritten() {
        assert_eq!(
            decide(Some(&entry(None, None, None)), "c1", false),
            write(3, None, &[])
        );
    }
    #[test]
    fn changed_sections_retire_unsubmitted_work_and_link_submitted_summaries() {
        assert_eq!(
            decide(
                Some(&entry(Some("memory:r"), Some("draft:a"), None)),
                "c2",
                true
            ),
            write(3, None, &["memory:r", "draft:a"])
        );
        assert_eq!(
            decide(Some(&entry(None, None, Some("s1"))), "c2", false),
            write(3, Some("s1"), &[])
        );
    }
    #[test]
    fn changed_sections_retire_stale_ids_even_when_not_pending() {
        assert_eq!(
            decide(
                Some(&entry(Some("memory:r"), Some("draft:a"), Some("s1"))),
                "c2",
                false
            ),
            write(3, Some("s1"), &["memory:r", "draft:a"])
        );
    }
    #[test]
    fn generation_overflow_is_an_error() {
        let mut e = entry(None, Some("draft:a"), None);
        e.generation = u32::MAX;
        assert_eq!(
            decide(Some(&e), "c2", false),
            Err("invalid memory import map".into())
        );
        assert_eq!(
            decide(Some(&e), "c1", false),
            Err("invalid memory import map".into())
        );
    }
    #[test]
    fn map_round_trips_owner_only() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(load(temp.path()).unwrap(), Map::default());
        let mut map = Map::default();
        map.entries
            .insert("k".into(), entry(None, None, Some("s1")));
        update(temp.path(), |m| {
            *m = Map {
                version: 1,
                entries: map.entries.clone(),
            };
            Ok(())
        })
        .unwrap();
        assert_eq!(load(temp.path()).unwrap(), map);
        #[cfg(unix)]
        for name in [FILE, LOCK] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(temp.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }
    #[test]
    fn unknown_versions_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(FILE), r#"{"version":2,"entries":{}}"#).unwrap();
        assert_eq!(
            load(temp.path()),
            Err("unsupported memory import map".into())
        );
    }
    #[test]
    fn corrupt_maps_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(FILE), "{not json").unwrap();
        assert_eq!(load(temp.path()), Err("invalid memory import map".into()));
    }
    #[test]
    fn sequential_updates_both_persist() {
        let temp = tempfile::tempdir().unwrap();
        for key in ["a", "b"] {
            update(temp.path(), |m| {
                m.entries.insert(key.into(), entry(None, None, Some("s1")));
                Ok(())
            })
            .unwrap();
        }
        let keys: Vec<_> = load(temp.path()).unwrap().entries.into_keys().collect();
        assert_eq!(keys, ["a", "b"]);
    }
    #[test]
    fn failed_updates_do_not_write() {
        let temp = tempfile::tempdir().unwrap();
        let result: Result<(), String> = update(temp.path(), |m| {
            m.entries.insert("a".into(), entry(None, None, Some("s1")));
            Err("boom".into())
        });
        assert_eq!(result, Err("boom".into()));
        assert!(!temp.path().join(FILE).exists());
    }
    #[test]
    fn updates_wait_for_and_then_give_up_on_a_held_lock() {
        let temp = tempfile::tempdir().unwrap();
        let held = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(temp.path().join(LOCK))
            .unwrap();
        held.lock().unwrap();
        let result = update(temp.path(), |_| Ok(()));
        assert_eq!(result, Err("memory import map unavailable".into()));
        assert!(!temp.path().join(FILE).exists());
        drop(held);
        update(temp.path(), |_| Ok(())).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn unreadable_state_directory_is_an_error_not_an_empty_map() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("state");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = load(&dir);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(result, Err("memory import map unavailable".into()));
        }
    }
}
