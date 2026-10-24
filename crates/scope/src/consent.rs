//! Local consent records: where a private binding file lives and how an approval is written.
//! Shared by `wk workspace approve` and the `wk-mcp` approval page so both apply the same rules.
use crate::enrollment::Binding;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BINDING_BYTES: u64 = 65536;

/// The binding file for `root` inside the consent directory `state`.
pub fn binding_file(state: &Path, root: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|_| "workspace unavailable")?;
    Ok(state.join(format!(
        "{:x}.json",
        Sha256::digest(root.as_os_str().to_string_lossy().as_bytes())
    )))
}
/// `path` with its longest existing prefix canonicalized, resolved before making any directories.
/// A dangling symlink counts as existing, so it fails to resolve instead of being created through.
pub fn resolve_prefix(path: &Path, what: &str) -> Result<PathBuf, String> {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while fs::symlink_metadata(ancestor).is_err() {
        suffix.push(ancestor.file_name().ok_or(format!("invalid {what}"))?);
        ancestor = ancestor.parent().ok_or(format!("invalid {what}"))?;
    }
    let mut resolved = ancestor
        .canonicalize()
        .map_err(|_| format!("{what} unavailable"))?;
    for part in suffix.iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}
/// The consent directory must be absolute, free of `..`, and outside the product folder.
fn checked_state(state: Option<&Path>, root: &Path) -> Result<PathBuf, String> {
    let state = state.ok_or("--state-dir is required for local consent")?;
    if !state.is_absolute()
        || state
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("state directory must be an absolute path without parent traversal".into());
    }
    let root = root.canonicalize().map_err(|_| "workspace unavailable")?;
    let resolved = resolve_prefix(state, "state directory")?;
    if resolved.starts_with(&root) {
        return Err("state directory must be outside product folder".into());
    }
    Ok(resolved)
}
/// The private consent directory for `root`. With `create` a missing directory is made (mode 0700
/// on Unix); an existing one must already be private and is never changed.
pub fn state_dir(state: Option<&Path>, root: &Path, create: bool) -> Result<PathBuf, String> {
    let resolved = checked_state(state, root)?;
    if !state.is_some_and(Path::exists) {
        if !create {
            return Err("state directory unavailable".into());
        }
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&resolved)
            .map_err(|_| "cannot create private state directory")?;
    }
    if !resolved.is_dir() {
        return Err("state directory unavailable".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&resolved)
            .map_err(|_| "state directory unavailable")?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err("existing state directory must be private (mode 0700); permissions were not changed".into());
        }
    }
    Ok(resolved)
}
/// Check that `state` could hold consent for `root` without creating anything: a directory that
/// does not exist yet is fine, one that exists must be private and outside `root`.
pub fn check_state_dir(state: &Path, root: &Path) -> Result<(), String> {
    checked_state(Some(state), root)?;
    if state.exists() {
        state_dir(Some(state), root, false)?;
    }
    Ok(())
}
/// Refuse a folder that is inside, or contains, the consent directory: consent files must never
/// be reachable from the folder being captured, nor the folder from the consent files.
pub fn folder_apart(state: &Path, root: &Path) -> Result<(), String> {
    let root = root.canonicalize().map_err(|_| "workspace unavailable")?;
    let state = resolve_prefix(state, "state directory")?;
    if state.starts_with(&root) || root.starts_with(&state) {
        return Err(
            "the project folder and the consent directory must not contain each other".into(),
        );
    }
    Ok(())
}
fn limited(path: &Path) -> Result<String, String> {
    let f = fs::File::open(path).map_err(|_| "input file unavailable")?;
    let mut s = String::new();
    f.take(MAX_BINDING_BYTES + 1)
        .read_to_string(&mut s)
        .map_err(|_| "invalid input encoding")?;
    if s.len() as u64 > MAX_BINDING_BYTES {
        return Err("input exceeds limit".into());
    }
    Ok(s)
}
pub fn read_binding(state: &Path, root: &Path) -> Result<Binding, String> {
    serde_json::from_str(&limited(&binding_file(state, root)?)?)
        .map_err(|_| "invalid workspace binding".into())
}
pub fn save_binding(state: &Path, binding: &Binding) -> Result<(), String> {
    let mut file = tempfile::NamedTempFile::new_in(state).map_err(|_| "cannot save consent")?;
    let bytes = serde_json::to_vec_pretty(binding).map_err(|_| "invalid binding")?;
    file.write_all(&bytes).map_err(|_| "cannot save consent")?;
    file.as_file()
        .sync_all()
        .map_err(|_| "cannot sync consent")?;
    file.persist(binding_file(state, &binding.root)?)
        .map_err(|_| "cannot commit consent")?;
    Ok(())
}
/// A recorded approval.
pub struct Approved {
    pub binding: Binding,
    pub binding_file: PathBuf,
}
impl Approved {
    /// What `wk workspace approve` prints.
    pub fn summary(&self) -> Value {
        json!({
            "local_approved": true,
            "binding_file": self.binding_file,
            "root": self.binding.root,
            "services": self.binding.services,
            "team_authorized": false,
            "live_capture_enabled": false
        })
    }
}
/// Record local consent for `services` (subdirectories of `root`) under the consent directory
/// `state`, which is created if missing. Validates the folder, product and services first, then
/// the consent directory, and writes the private binding file only when all of them pass.
pub fn approve(
    state: Option<&Path>,
    root: &Path,
    product: &str,
    services: Vec<String>,
) -> Result<Approved, String> {
    let binding = Binding::approve(root, product, services)?;
    let state = state_dir(state, root, true)?;
    save_binding(&state, &binding)?;
    let binding_file = binding_file(&state, &binding.root)?;
    Ok(Approved {
        binding,
        binding_file,
    })
}
