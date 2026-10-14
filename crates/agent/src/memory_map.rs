//! Content-free record of imported memory sections, kept in an owner-only
//! state file because the encrypted store retains records for at most a day.

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
        assert_eq!(
            decide(Some(&entry(None, Some("draft:a"), None)), "c1", true),
            Action::Skip
        );
        assert_eq!(
            decide(Some(&entry(None, None, Some("s1"))), "c1", false),
            Action::Skip
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
    fn map_round_trips_owner_only_and_rejects_unknown_versions() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(load(temp.path()).unwrap(), Map::default());
        let mut map = Map::default();
        map.entries
            .insert("k".into(), entry(None, None, Some("s1")));
        save(temp.path(), &map).unwrap();
        assert_eq!(load(temp.path()).unwrap(), map);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(temp.path().join(FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(temp.path().join(FILE), r#"{"version":2,"entries":{}}"#).unwrap();
        assert!(load(temp.path()).is_err());
    }
}
