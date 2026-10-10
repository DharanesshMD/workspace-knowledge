//! Typed stdin-only administration: identity always comes from ordinary OIDC.
use crate::auth::Oidc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use wk_knowledge::{
    admin::{MemberChange, ProductPolicy},
    Knowledge,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminRequest {
    token: String,
    product: String,
    command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum Command {
    Members {},
    MemberPut { member: MemberChange },
    MemberRemove { user: String },
    PolicyPut { policy: ProductPolicy },
    DeviceRevoke { id: uuid::Uuid },
}
pub fn parse<T: serde::de::DeserializeOwned, R: std::io::Read>(reader: R) -> Result<T, String> {
    let mut bytes = Vec::new();
    reader
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "administrative input unavailable")?;
    if bytes.len() > 65536 {
        return Err("administrative input too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid administrative input".into())
}
pub async fn execute(k: &Knowledge, oidc: &Oidc, input: AdminRequest) -> Result<Value, String> {
    let (issuer, subject) = oidc.verify(&input.token).map_err(|_| "unauthenticated")?;
    let a = k
        .identity(issuer, &subject)
        .await
        .map_err(|_| "unauthenticated")?;
    match input.command {
        Command::Members {} => Ok(
            json!({"members":k.admin_members(&a,&input.product).await.map_err(|e|e.to_string())?}),
        ),
        Command::MemberPut { member } => {
            k.admin_member_put(&a, &input.product, member)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"changed":true}))
        }
        Command::MemberRemove { user } => {
            k.admin_member_remove(&a, &input.product, &user)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"removed":true}))
        }
        Command::PolicyPut { policy } => {
            k.admin_policy_put(&a, &input.product, policy)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"changed":true}))
        }
        Command::DeviceRevoke { id } => {
            k.admin_device_revoke(&a, &input.product, id)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"revoked":true}))
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminAssignment {
    pub org: String,
    pub user: String,
    pub product: String,
    pub services: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyBootstrap {
    pub org: String,
    pub user: String,
    pub product: String,
    pub policy: ProductPolicy,
}
