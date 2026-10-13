//! Bounded public device contracts; no local paths, credentials or diagnostic strings.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeviceRegistration {
    pub id: uuid::Uuid,
    pub public_key: String,
    pub product: String,
    pub services: Vec<String>,
    pub approval_hash: String,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeviceSettings {
    pub paused: bool,
    pub owner_sync: bool,
    pub activity: bool,
}
macro_rules! enumeration {($name:ident {$($v:ident),+})=>{#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)] #[serde(rename_all="snake_case")] pub enum $name {$($v),+}}}
enumeration!(ConfigStatus { Applied, Rejected });
enumeration!(Queue {
    Unknown,
    Empty,
    Small,
    Large
});
enumeration!(Reason {
    None,
    LocalApprovalMissing,
    RemotePause,
    VersionUnsupported,
    CaptureGap,
    AuthRequired,
    InferenceUnavailable
});
enumeration!(Client {
    Cursor,
    Claude,
    Gemini,
    Codex,
    Antigravity
});
enumeration!(Surface {
    Cli,
    Editor,
    Desktop
});
enumeration!(CapabilityStatus {
    Unknown,
    Partial,
    Experimental,
    Blocked,
    Supported
});
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub client: Client,
    pub surface: Surface,
    pub version: Option<String>,
    pub status: CapabilityStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceReport {
    pub collector_version: String,
    pub config_revision: i64,
    pub config_status: ConfigStatus,
    pub local_pause: bool,
    pub local_owner_sync: bool,
    pub local_activity: bool,
    pub queue: Queue,
    pub reason: Reason,
    pub capabilities: Vec<Capability>,
}
fn version(v: &str) -> bool {
    v.len() <= 64
        && v.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
}
pub fn hex32(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || b"abcdef".contains(&b))
}
impl DeviceRegistration {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_nil()
            || !hex32(&self.public_key)
            || !hex32(&self.approval_hash)
            || !crate::valid_id(&self.product)
            || self.services.is_empty()
            || self.services.len() > 20
            || self.services.iter().any(|s| !crate::valid_id(s))
        {
            return Err("invalid device registration".into());
        }
        Ok(())
    }
}
impl DeviceReport {
    pub fn validate(&self) -> Result<(), String> {
        if !version(&self.collector_version)
            || self.config_revision < 1
            || self.capabilities.len() > 15
            || self
                .capabilities
                .iter()
                .any(|c| c.version.as_ref().is_some_and(|v| !version(v)))
            || serde_json::to_vec(self)
                .map_err(|_| "invalid report")?
                .len()
                > 4096
        {
            return Err("invalid device report".into());
        }
        Ok(())
    }
}
/// Exact request bytes are hashed, so JSON reserialization invalidates a proof.
#[allow(clippy::too_many_arguments)]
pub fn signing_bytes(
    origin: &str,
    method: &str,
    path: &str,
    time: i64,
    nonce: uuid::Uuid,
    device: uuid::Uuid,
    token: &str,
    body: &[u8],
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!([
        "wk-device-v1",
        origin,
        method,
        path,
        time,
        nonce,
        device,
        format!("{:x}", Sha256::digest(token.as_bytes())),
        format!("{:x}", Sha256::digest(body))
    ]))
    .expect("serializable envelope")
}
