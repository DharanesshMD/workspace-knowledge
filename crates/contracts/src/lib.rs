//! Strict shared knowledge contracts. Raw conversation types never belong here.
pub mod change_context;
pub mod device;
pub mod retrieval;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Proposed,
    InProgress,
    Implemented,
    Verified,
    Superseded,
    Deprecated,
}
#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    Complete,
    Partial,
    Truncated,
}
/// Who wrote a card, when that (and not capture coverage) is what makes it publishable.
/// Self-declared by the submitter, like `capture_completeness`; the reviewer is the check.
#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    AssistantWritten,
}
#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    ConversationReport,
    FileObservation,
    Commit,
    ReviewedPr,
    TestResult,
    OwnerAttestation,
}
#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub reference: String,
}
#[derive(JsonSchema, Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardSubmission {
    #[schemars(range(min = 1, max = 1))]
    pub schema_version: u32,
    pub idempotency_key: String,
    #[schemars(range(min = 1))]
    pub policy_revision: u64,
    #[schemars(length(min = 1, max = 256))]
    pub title: String,
    pub intent: String,
    pub rationale: String,
    pub decisions: Vec<String>,
    pub constraints: Vec<String>,
    pub open_questions: Vec<String>,
    #[schemars(length(min = 1))]
    pub service_ids: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub lifecycle: Lifecycle,
    pub capture_completeness: Completeness,
    pub base_revision: Option<u64>,
    /// When the source work happened (Unix ms), if known. Defaults to submission time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<i64>,
    /// Set only for a card the developer's assistant wrote and the developer reviewed before
    /// submitting. Omitted from the wire when absent so older servers still accept ordinary cards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}
impl CardSubmission {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported schema version".into());
        }
        if let Some(at) = self.observed_at_ms {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "clock unavailable")?
                .as_millis() as i64;
            const YEAR_MS: i64 = 366 * 24 * 3600 * 1000;
            if at > now + 300_000 || at < now - YEAR_MS {
                return Err("observation time out of range".into());
            }
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
        if self.provenance.is_some() && self.capture_completeness == Completeness::Truncated {
            return Err("provenance cannot be set on a truncated card".into());
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

/// Versioned public submission shape. Runtime authorization, UTF-8 byte limits,
/// privacy screening and evidence verification remain mandatory after validation.
pub fn submission_schema() -> serde_json::Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(CardSubmission)).expect("schema serialization");
    schema["$id"] = serde_json::json!("urn:workspace-knowledge:card-submission:v1");
    schema["properties"]["lifecycle"] =
        serde_json::json!({"type":"string","enum":["proposed","in_progress"]});
    schema["properties"]["service_ids"]["items"] =
        serde_json::json!({"type":"string","pattern":"^[A-Za-z0-9_-]{1,128}$"});
    for field in ["title", "intent", "idempotency_key"] {
        schema["properties"][field]["pattern"] = serde_json::json!("\\S");
    }
    schema
}
