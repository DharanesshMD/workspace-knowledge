//! Private adapter evidence. Model output cannot establish these properties.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Client {
    Codex,
    Antigravity,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Verified,
    Unknown,
    Violated,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Terminal {
    Open,
    Complete,
    Cancelled,
    Held,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub client: Client,
    pub client_version: String,
    pub parser_version: u32,
    pub session: String,
    pub agent: String,
    pub turn: String,
    pub prompt: String,
    pub response: String,
    pub messages_complete: bool,
    pub scope: Scope,
    pub terminal: Terminal,
    pub digest: String,
    pub files_read: Vec<String>,
    pub files_changed: Vec<String>,
    pub hold_reason: Option<String>,
}
pub fn eligible(e: &Evidence) -> bool {
    e.messages_complete
        && e.scope == Scope::Verified
        && e.terminal == Terminal::Complete
        && e.hold_reason.is_none()
}
