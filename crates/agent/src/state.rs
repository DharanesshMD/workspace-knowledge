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
    #[serde(default)]
    pub summarizer: Summarizer,
    pub port: u16,
    pub account: String,
    pub device_id: String,
    #[serde(default)]
    pub device: Option<crate::device::Connection>,
    #[serde(default)]
    pub repository: Option<crate::repository::RepositoryEnrollment>,
}
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Summarizer {
    /// Drafts come from a model on this machine.
    #[default]
    Local,
    /// Drafts are built from the conversation's structure; no model.
    Rules,
}
impl Summarizer {
    pub fn as_str(self) -> &'static str {
        match self {
            Summarizer::Local => "local",
            Summarizer::Rules => "rules",
        }
    }
}
/// Pick the mode and the inference URL to store. Local needs a URL; rules must not have one.
pub fn resolve_summarizer(
    flag: Option<Summarizer>,
    inference: Option<String>,
) -> Result<(Summarizer, String), String> {
    match (flag, inference) {
        (None, None) | (Some(Summarizer::Rules), None) => Ok((Summarizer::Rules, String::new())),
        (None, Some(url)) | (Some(Summarizer::Local), Some(url)) => {
            wk_summarize::local_url(&url)?;
            Ok((Summarizer::Local, url))
        }
        (Some(Summarizer::Local), None) => Err("--summarizer local needs --inference".into()),
        (Some(Summarizer::Rules), Some(_)) => {
            Err("--summarizer rules cannot be combined with --inference".into())
        }
    }
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
        // A Rust child of PowerShell 7 must not give its Core module paths to 5.1.
        .env_remove("PSModulePath")
        .env("WK_STATE_PATH", path)
        .stdout(std::process::Stdio::null())
        .stderr(if cfg!(test) {
            std::process::Stdio::inherit()
        } else {
            std::process::Stdio::null()
        })
        .status()
        .map_err(|_| "private ACL unavailable")?;
    if !status.success() {
        return Err("state directory ACL is not private".into());
    }
    Ok(())
}
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn new_private_state_can_be_reopened_at_its_canonical_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state");
        let canonical = private_dir(&path, true).unwrap();
        assert_eq!(private_dir(&canonical, false).unwrap(), canonical);
    }

    #[test]
    fn existing_inherited_state_is_rejected_without_rewriting_its_acl() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state");
        std::fs::create_dir(&path).unwrap();
        assert!(private_dir(&path, true).is_err());
        assert!(private_dir(&path, false).is_err());
    }
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

#[cfg(test)]
mod summarizer_tests {
    use super::*;
    #[test]
    fn old_configs_without_a_summarizer_are_local() {
        let old = r#"{"root":"/w","binding_file":"/b.json","service":"backend","server":"http://127.0.0.1:4318","model":"m","inference":"http://127.0.0.1:11434/v1/chat/completions","port":4319,"account":"workspace","device_id":"d"}"#;
        let c: Config = serde_json::from_str(old).unwrap();
        assert_eq!(c.summarizer, Summarizer::Local);
    }
    #[test]
    fn summarizer_choice_follows_the_flags() {
        use Summarizer::*;
        let url = "http://127.0.0.1:11434/v1/chat/completions";
        assert_eq!(
            resolve_summarizer(None, None).unwrap(),
            (Rules, String::new())
        );
        assert_eq!(
            resolve_summarizer(None, Some(url.into())).unwrap(),
            (Local, url.to_string())
        );
        assert_eq!(resolve_summarizer(Some(Rules), None).unwrap().0, Rules);
        assert_eq!(
            resolve_summarizer(Some(Local), Some(url.into())).unwrap().0,
            Local
        );
        assert!(
            resolve_summarizer(Some(Local), None).is_err(),
            "local needs an inference URL"
        );
        assert!(
            resolve_summarizer(Some(Rules), Some("http://127.0.0.1:1/x".into())).is_err(),
            "rules must not carry an inference URL"
        );
        assert!(
            resolve_summarizer(None, Some("https://example.com/v1".into())).is_err(),
            "inference must stay on this machine"
        );
    }
    #[test]
    fn summarizer_serializes_lowercase_and_names_itself() {
        assert_eq!(
            serde_json::to_string(&Summarizer::Rules).unwrap(),
            "\"rules\""
        );
        assert_eq!(Summarizer::Local.as_str(), "local");
        assert_eq!(Summarizer::Rules.as_str(), "rules");
    }
}
