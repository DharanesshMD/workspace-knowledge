use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub service: String,
    pub profile: String,
    pub mount: String,
    pub path: String,
    pub address: String,
    pub workspace: PathBuf,
    pub keys: BTreeSet<String>,
}
pub fn segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub fn key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        // Production enablement requires a separately reviewed deployment policy, not a CLI bypass.
        if !["development", "test", "staging"].contains(&self.profile.as_str()) {
            return Err(Error::Denied);
        }
        if !segment(&self.service)
            || !segment(&self.mount)
            || !self.path.split('/').all(segment)
            || self.keys.is_empty()
            || !self.keys.iter().all(|k| {
                key(k) && !["WK_BAO_TOKEN", "BAO_TOKEN", "VAULT_TOKEN"].contains(&k.as_str())
            })
            || !self.workspace.is_absolute()
        {
            return Err(Error::Invalid);
        }
        // Canonical service/profile suffix prevents accidentally binding a named profile to another one.
        if self.path.split('/').rev().take(2).collect::<Vec<_>>()
            != [self.profile.as_str(), self.service.as_str()]
        {
            return Err(Error::Invalid);
        }
        let url = reqwest::Url::parse(&self.address).map_err(|_| Error::Invalid)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn schema(&self) -> Result<serde_json::Value> {
        self.validate()?;
        Ok(
            serde_json::json!({"service": self.service, "profile": self.profile, "keys": self.keys, "values": "never exposed", "authorization": "OpenBao token policy enforced on each operation"}),
        )
    }
}
