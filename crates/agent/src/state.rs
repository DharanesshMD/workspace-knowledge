use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use wk_scope::enrollment::Binding;
#[derive(Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub root: PathBuf,
    pub binding_file: PathBuf,
    pub service: String,
    pub server: String,
    pub model: String,
    pub inference: String,
    pub port: u16,
    pub account: String,
    pub device_id: String,
    #[serde(default)]
    pub device: Option<crate::device::Connection>,
}
pub fn private_dir(path: &Path, create: bool) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("state path must be absolute".into());
    }
    if !path.exists() && create {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(path).map_err(|_| "cannot create private state")?;
        #[cfg(windows)]
        windows_acl(path, true)?;
    }
    if std::fs::symlink_metadata(path)
        .map_err(|_| "state unavailable")?
        .file_type()
        .is_symlink()
    {
        return Err("state symlink denied".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let m = std::fs::metadata(path).map_err(|_| "state unavailable")?;
        if !m.is_dir()
            || m.permissions().mode() & 0o077 != 0
            || m.uid() != unsafe { libc::geteuid() }
        {
            return Err("state must be an owned private directory (0700)".into());
        }
    }
    #[cfg(windows)]
    windows_acl(path, false)?;
    path.canonicalize().map_err(|_| "state unavailable".into())
}
#[cfg(windows)]
fn windows_acl(path: &Path, create: bool) -> Result<(), String> {
    let script = if create {
        "$ErrorActionPreference='Stop';$sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User;$acl=New-Object System.Security.AccessControl.DirectorySecurity;$acl.SetOwner($sid);$acl.SetAccessRuleProtection($true,$false);$r=New-Object System.Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow');$acl.AddAccessRule($r);Set-Acl -LiteralPath $env:WK_STATE_PATH -AclObject $acl"
    } else {
        "$ErrorActionPreference='Stop';$sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value;$a=Get-Acl -LiteralPath $env:WK_STATE_PATH;if(!$a.AreAccessRulesProtected){exit 1};foreach($r in $a.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier])){if($r.AccessControlType -eq 'Allow' -and $r.IdentityReference.Value -ne $sid){exit 1}}"
    };
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("WK_STATE_PATH", path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| "private ACL unavailable")?;
    if !status.success() {
        return Err("state directory ACL is not private".into());
    }
    Ok(())
}
pub fn read(path: &Path, max: u64) -> Result<String, String> {
    if std::fs::symlink_metadata(path)
        .map_err(|_| "file unavailable")?
        .file_type()
        .is_symlink()
    {
        return Err("state symlink denied".into());
    }
    let mut out = String::new();
    std::fs::File::open(path)
        .map_err(|_| "file unavailable")?
        .take(max + 1)
        .read_to_string(&mut out)
        .map_err(|_| "invalid file encoding")?;
    if out.len() as u64 > max {
        return Err("file too large".into());
    }
    Ok(out)
}
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut f = tempfile::NamedTempFile::new_in(path.parent().ok_or("invalid path")?)
        .map_err(|_| "state unavailable")?;
    f.write_all(&serde_json::to_vec(value).map_err(|_| "invalid state")?)
        .map_err(|_| "state write failed")?;
    f.as_file().sync_all().map_err(|_| "state sync failed")?;
    f.persist(path).map_err(|_| "state commit failed")?;
    Ok(())
}
pub fn load(state: &Path) -> Result<Config, String> {
    serde_json::from_str(&read(&state.join("agent.json"), 65536)?)
        .map_err(|_| "invalid agent config".into())
}
pub fn base_binding(c: &Config) -> Result<Binding, String> {
    let b: Binding = serde_json::from_str(&read(&c.binding_file, 65536)?)
        .map_err(|_| "invalid local approval")?;
    let approved = Binding {
        root: b.root.clone(),
        product: b.product.clone(),
        services: b.services.clone(),
        enabled: true,
        policy_hash: b.policy_hash.clone(),
    };
    if b.root != c.root
        || !b.services.contains(&c.service)
        || !approved.eligible(&b.root.join(&c.service))
    {
        return Err("workspace approval absent, changed or paused".into());
    }
    Ok(b)
}
pub fn binding(c: &Config) -> Result<Binding, String> {
    let b = base_binding(c)?;
    if !b.enabled {
        return Err("workspace approval paused".into());
    }
    if c.device.is_some() {
        crate::device::validate(c, &b)?;
        if crate::device::remote(c)?.settings.paused {
            return Err("remote capture pause requested".into());
        }
    }
    Ok(b)
}
pub fn entry(state: &Path, purpose: &str) -> Result<keyring::Entry, String> {
    if !cfg!(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    )) {
        return Err("native credential store unsupported".into());
    }
    let id = format!(
        "{:x}:{purpose}",
        Sha256::digest(state.to_string_lossy().as_bytes())
    );
    keyring::Entry::new("workspace-knowledge", &id)
        .map_err(|_| "native credential store unavailable".into())
}
pub fn secret(state: &Path, purpose: &str) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
    Ok(zeroize::Zeroizing::new(
        entry(state, purpose)?
            .get_secret()
            .map_err(|_| "native credential unavailable")?,
    ))
}
pub fn token(state: &Path, purpose: &str) -> Result<zeroize::Zeroizing<String>, String> {
    if purpose == "server-token"
        && load(state)?.device.is_some()
        && !crate::device::credential_current(state)?
    {
        return Err("collector authentication expired; run device login".into());
    }
    Ok(zeroize::Zeroizing::new(
        String::from_utf8(secret(state, purpose)?.to_vec())
            .map_err(|_| "invalid stored credential")?,
    ))
}
pub fn store(state: &Path) -> Result<wk_local::LocalStore, String> {
    let key = secret(state, "capture-key")?;
    let key: [u8; 32] = key
        .as_slice()
        .try_into()
        .map_err(|_| "invalid capture key")?;
    wk_local::LocalStore::open(&state.join("capture.db"), key, 100 * 1024 * 1024)
}
