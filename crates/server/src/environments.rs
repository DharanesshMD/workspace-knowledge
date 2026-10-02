//! Operator-provided schemas only. No vault addresses, tokens or values are accepted.
use serde::{Deserialize, Serialize};
#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    pub required: bool,
    pub description: String,
}
#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    pub org: String,
    pub product: String,
    pub id: String,
    pub name: String,
    pub service_ids: Vec<String>,
    pub variables: Vec<Variable>,
}
pub fn load() -> Result<Vec<Schema>, String> {
    use std::io::Read;
    let Ok(path) = std::env::var("WK_ENV_SCHEMA_FILE") else {
        return Ok(vec![]);
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "schema file unavailable")?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "schema read failed")?;
    if bytes.len() > 65536 {
        return Err("schema file too large".into());
    }
    let schemas: Vec<Schema> =
        serde_json::from_slice(&bytes).map_err(|_| "invalid environment schema")?;
    for s in &schemas {
        if !wk_contracts::valid_id(&s.product)
            || !wk_contracts::valid_id(&s.id)
            || s.service_ids.is_empty()
            || s.service_ids.iter().any(|s| !wk_contracts::valid_id(s))
            || s.variables.len() > 100
            || s.variables.iter().any(|v| {
                v.name.is_empty()
                    || v.name.len() > 128
                    || !v
                        .name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || v.description.len() > 512
            })
        {
            return Err("invalid schema fields".into());
        }
    }
    Ok(schemas)
}
