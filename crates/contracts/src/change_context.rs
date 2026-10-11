//! Bounded source-backed references. No raw conversation or edit inputs on this wire.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeTarget {
    pub path: String,
    pub start: Option<u32>,
    pub end: Option<u32>,
}
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.starts_with(['/', '~', '-'])
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}
pub fn valid_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl CodeTarget {
    pub fn validate(&self) -> bool {
        valid_path(&self.path)
            && match (self.start, self.end) {
                (None, None) => true,
                (Some(a), Some(b)) => a > 0 && b >= a && b - a <= 10000,
                _ => false,
            }
    }
}
pub fn valid_targets(targets: &[CodeTarget]) -> bool {
    let mut seen = BTreeSet::new();
    !targets.is_empty()
        && targets.len() <= 20
        && targets
            .iter()
            .all(|t| t.validate() && seen.insert((&t.path, t.start, t.end)))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeContextRequest {
    pub schema_version: u32,
    pub repository_id: uuid::Uuid,
    pub base_revision: String,
    pub targets: Vec<CodeTarget>,
    pub operation_class: String,
    pub operation_handle: Option<uuid::Uuid>,
    pub budget_bytes: u32,
}
impl ChangeContextRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.repository_id.is_nil()
            || !valid_revision(&self.base_revision)
            || !valid_targets(&self.targets)
            || !(1024..=16384).contains(&self.budget_bytes)
            || !["read_context", "edit", "write", "delete", "multi_edit"]
                .contains(&self.operation_class.as_str())
            || self.operation_handle.is_some_and(|id| id.is_nil())
        {
            return Err("invalid change context request".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    ContextAvailable,
    NeedsReview,
    Denied,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub kind: String,
    pub id: String,
    pub revision: i64,
    pub code_revision: Option<String>,
    pub assurance: String,
    pub freshness: String,
    pub observed_at_ms: i64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextItem {
    pub role: String,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    pub contact_url: Option<String>,
    pub text: String,
    pub matched_by: String,
    pub sources: Vec<SourceRef>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeContextBrief {
    pub schema_version: u32,
    pub repository_id: uuid::Uuid,
    pub base_revision: String,
    pub targets: Vec<CodeTarget>,
    pub decision: Decision,
    pub reasons: Vec<String>,
    pub gaps: Vec<String>,
    pub coverage: String,
    pub observation_assurance: String,
    pub items: Vec<ContextItem>,
    pub truncated: bool,
    pub brief_id: Option<uuid::Uuid>,
    pub expires_at_ms: Option<i64>,
}
