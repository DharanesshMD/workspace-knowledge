//! Strict shared knowledge contracts. Raw conversation types never belong here.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Proposed,
    InProgress,
    Implemented,
    Verified,
    Superseded,
    Deprecated,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    Complete,
    Partial,
    Truncated,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    ConversationReport,
    FileObservation,
    Commit,
    ReviewedPr,
    TestResult,
    OwnerAttestation,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub reference: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardSubmission {
    pub schema_version: u32,
    pub idempotency_key: String,
    pub policy_revision: u64,
    pub title: String,
    pub intent: String,
    pub rationale: String,
    pub decisions: Vec<String>,
    pub constraints: Vec<String>,
    pub open_questions: Vec<String>,
    pub service_ids: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub lifecycle: Lifecycle,
    pub capture_completeness: Completeness,
    pub base_revision: Option<u64>,
}
impl CardSubmission {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported schema version".into());
        }
        if self.policy_revision == 0
            || self.idempotency_key.trim().is_empty()
            || self.title.trim().is_empty()
            || self.intent.trim().is_empty()
        {
            return Err("required card fields are empty".into());
        }
        if self.service_ids.is_empty() || self.service_ids.iter().any(|s| !valid_id(s)) {
            return Err("invalid service scope".into());
        }
        if self.title.len() > 256
            || serde_json::to_vec(self).map_err(|_| "invalid card")?.len() > 65536
        {
            return Err("card exceeds size limit".into());
        }
        // Status promotion requires a separate trusted evidence verifier, not caller assertions.
        if !matches!(self.lifecycle, Lifecycle::Proposed | Lifecycle::InProgress) {
            return Err("lifecycle requires trusted verification".into());
        }
        if self
            .evidence
            .iter()
            .any(|e| e.reference.trim().is_empty() || e.reference.len() > 2048)
        {
            return Err("invalid evidence reference".into());
        }
        Ok(())
    }
}
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
pub fn validate_card(value: &serde_json::Value) -> Result<(), String> {
    let card: CardSubmission =
        serde_json::from_value(value.clone()).map_err(|_| "invalid card schema".to_string())?;
    card.validate()
}
