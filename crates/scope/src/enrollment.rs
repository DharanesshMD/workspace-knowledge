use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub root: PathBuf,
    pub product: String,
    pub services: Vec<String>,
    pub enabled: bool,
    pub policy_hash: String,
}
fn identity(root: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(root).map_err(|_| "workspace unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("{}:{}", meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        Ok(format!(
            "{:?}",
            meta.created()
                .map_err(|_| "workspace identity unavailable")?
        ))
    }
}
impl Binding {
    fn fingerprint(&self) -> Result<String, String> {
        let bytes = serde_json::to_vec(&(
            &self.root,
            &self.product,
            &self.services,
            identity(&self.root)?,
        ))
        .map_err(|_| "invalid binding")?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
    pub fn approve(root: &Path, product: &str, services: Vec<String>) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|_| "workspace unavailable")?;
        if !root.is_dir()
            || product.is_empty()
            || product.len() > 128
            || !product
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            || services.is_empty()
        {
            return Err("invalid workspace enrollment".into());
        }
        if services.iter().any(|s| {
            !super::safe_relative(Path::new(s))
                || !root.join(s).is_dir()
                || !super::approved(&root, &root.join(s), std::slice::from_ref(s))
        }) {
            return Err("invalid service directory".into());
        }
        let mut b = Self {
            root,
            product: product.into(),
            services,
            enabled: true,
            policy_hash: String::new(),
        };
        b.policy_hash = b.fingerprint()?;
        Ok(b)
    }
    /// Enabled, and the recorded fingerprint still matches the root, product and services.
    pub fn intact(&self) -> bool {
        self.enabled && self.fingerprint().is_ok_and(|h| h == self.policy_hash)
    }
    pub fn eligible(&self, path: &Path) -> bool {
        self.intact() && super::approved(&self.root, path, &self.services)
    }
    pub fn pause(&mut self) {
        self.enabled = false;
    }
}
