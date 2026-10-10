use crate::state;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wk_adapters::native_evidence::Client;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub client: Client,
    pub version: String,
    pub surface: String,
    pub device_id: String,
    pub canonical_workspace: PathBuf,
    pub service: String,
    pub policy_hash: String,
    pub enabled: bool,
}

pub fn name(client: Client) -> &'static str {
    match client {
        Client::Codex => "codex",
        Client::Antigravity => "antigravity",
    }
}
pub fn parse_client(s: &str) -> Result<Client, String> {
    match s {
        "codex" => Ok(Client::Codex),
        "antigravity" => Ok(Client::Antigravity),
        _ => Err("native_client_unsupported".into()),
    }
}
fn version(client: Client) -> (&'static str, &'static str) {
    match client {
        Client::Codex => ("0.162.0-alpha.2", "cli"),
        Client::Antigravity => ("2.19.1", "desktop"),
    }
}
fn installed(client: Client) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("native_platform_unsupported".into());
    }
    let out = match client {
        Client::Codex => std::process::Command::new("codex")
            .arg("--version")
            .output(),
        Client::Antigravity => std::process::Command::new("/usr/bin/plutil")
            .args([
                "-extract",
                "CFBundleShortVersionString",
                "raw",
                "-o",
                "-",
                "/Applications/Antigravity.app/Contents/Info.plist",
            ])
            .output(),
    }
    .map_err(|_| "native_version_unavailable")?;
    if !out.status.success() || out.stdout.len() > 1024 {
        return Err("native_version_unavailable".into());
    }
    let value = String::from_utf8(out.stdout).map_err(|_| "native_version_unavailable")?;
    Ok(value
        .trim()
        .strip_prefix("codex-cli ")
        .unwrap_or(value.trim())
        .into())
}
fn validate(
    e: &Enrollment,
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    installed: &str,
) -> Result<(), String> {
    let (v, surface) = version(e.client);
    let service = c
        .root
        .join(&c.service)
        .canonicalize()
        .map_err(|_| "workspace_unavailable")?;
    if !e.enabled
        || e.version != v
        || installed != v
        || e.surface != surface
        || e.device_id != c.device_id
        || e.service != c.service
        || e.policy_hash != b.policy_hash
        || e.canonical_workspace != service
        || !b.eligible(&service)
    {
        return Err("native_enrollment_changed".into());
    }
    Ok(())
}
pub fn load_current(dir: &Path, client: Client) -> Result<Enrollment, String> {
    let e: Enrollment = serde_json::from_str(&state::read(
        &dir.join(format!("native-{}.json", name(client))),
        65536,
    )?)
    .map_err(|_| "native_enrollment_unavailable")?;
    if e.client != client {
        return Err("native_client_mismatch".into());
    }
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    validate(&e, &c, &b, &e.version)?;
    Ok(e)
}
pub fn load(dir: &Path, client: Client) -> Result<Enrollment, String> {
    let e = load_current(dir, client)?;
    if installed(client)? != e.version {
        return Err("native_version_unverified".into());
    }
    Ok(e)
}
pub fn enroll(dir: &Path, client: Client) -> Result<(), String> {
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let (v, surface) = version(client);
    let e = Enrollment {
        client,
        version: v.into(),
        surface: surface.into(),
        device_id: c.device_id.clone(),
        canonical_workspace: c
            .root
            .join(&c.service)
            .canonicalize()
            .map_err(|_| "workspace_unavailable")?,
        service: c.service.clone(),
        policy_hash: b.policy_hash.clone(),
        enabled: true,
    };
    validate(&e, &c, &b, &installed(client)?)?;
    state::save(&dir.join(format!("native-{}.json", name(client))), &e)
}
pub fn disabled(dir: &Path, client: Client) -> Result<bool, String> {
    let path = dir.join(format!("native-{}.json", name(client)));
    if !path.exists() {
        return Ok(false);
    }
    let e: Enrollment = serde_json::from_str(&state::read(&path, 65536)?)
        .map_err(|_| "invalid_native_enrollment")?;
    Ok(!e.enabled)
}
pub fn disable(dir: &Path, client: Client) -> Result<(), String> {
    let path = dir.join(format!("native-{}.json", name(client)));
    if path.exists() {
        let mut e: Enrollment = serde_json::from_str(&state::read(&path, 65536)?)
            .map_err(|_| "invalid_native_enrollment")?;
        e.enabled = false;
        state::save(&path, &e)?;
    }
    Ok(())
}

fn emitter_matches(expected: &Path, actual: &Path) -> bool {
    expected
        .canonicalize()
        .ok()
        .zip(actual.canonicalize().ok())
        .is_some_and(|(a, b)| a == b)
}
fn executable(client: Client) -> Result<PathBuf, String> {
    match client {
        Client::Antigravity => Ok(PathBuf::from(
            "/Applications/Antigravity.app/Contents/MacOS/Antigravity",
        )),
        Client::Codex => {
            std::env::split_paths(&std::env::var_os("PATH").ok_or("path_unavailable")?)
                .map(|p| p.join("codex"))
                .find(|p| p.is_file())
                .ok_or("native_executable_unavailable".into())
        }
    }
}
#[cfg(target_os = "macos")]
fn process_executable(pid: u32) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStrExt;
    let mut buffer = [0u8; 4096];
    let length = unsafe {
        libc::proc_pidpath(
            pid as libc::c_int,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
        )
    };
    if length <= 0 {
        return Err("emitter_unavailable".into());
    }
    let end = buffer
        .iter()
        .position(|b| *b == 0)
        .ok_or("emitter_unavailable")?;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(&buffer[..end])))
}
#[cfg(not(target_os = "macos"))]
fn process_executable(_: u32) -> Result<PathBuf, String> {
    Err("native_platform_unsupported".into())
}
/// Bind hook delivery to the enrolled process, not an unrelated installed version.
pub fn verify_emitter(dir: &Path, client: Client) -> Result<(), String> {
    load(dir, client)?;
    let expected = executable(client)?;
    let mut pid = std::process::id();
    for _ in 0..8 {
        let parent = std::process::Command::new("/bin/ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .output()
            .map_err(|_| "emitter_unavailable")?;
        pid = String::from_utf8(parent.stdout)
            .map_err(|_| "emitter_unavailable")?
            .trim()
            .parse()
            .map_err(|_| "emitter_unavailable")?;
        if pid <= 1 {
            break;
        }
        let path = process_executable(pid)?;
        let actual = path.as_path();
        if emitter_matches(&expected, actual) {
            return Ok(());
        }
        if actual.file_name() == expected.file_name() {
            return Err("native_emitter_unverified".into());
        }
    }
    Err("native_emitter_unverified".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn process_identity_uses_executable_path_not_short_comm_name() {
        assert_eq!(
            process_executable(std::process::id())
                .unwrap()
                .canonicalize()
                .unwrap(),
            std::env::current_exe().unwrap().canonicalize().unwrap()
        );
    }
    #[test]
    fn different_emitter_path_is_not_verified_by_an_installed_pinned_binary() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let approved = root.join("approved");
        let other = root.join("other");
        std::fs::write(&approved, "approved").unwrap();
        std::fs::write(&other, "other").unwrap();
        assert!(emitter_matches(&approved, &approved));
        assert!(!emitter_matches(&approved, &other));
        assert!(!emitter_matches(&approved, &root.join("missing")));
    }
    #[test]
    fn enrollment_rejects_changed_version_workspace_policy_or_device() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let c = state::Config {
            root: root.clone(),
            binding_file: root.join("binding"),
            service: "app".into(),
            server: "http://127.0.0.1".into(),
            model: "test".into(),
            inference: "http://127.0.0.1".into(),
            port: 4319,
            account: "test".into(),
            device_id: "d".into(),

            device: None,
        };
        std::fs::create_dir(root.join("app")).unwrap();
        let b = wk_scope::enrollment::Binding::approve(&root, "test", vec!["app".into()]).unwrap();
        let e = Enrollment {
            client: Client::Codex,
            version: "0.162.0-alpha.2".into(),
            surface: "cli".into(),
            device_id: "d".into(),
            canonical_workspace: root.join("app"),
            service: "app".into(),
            policy_hash: b.policy_hash.clone(),
            enabled: true,
        };
        assert!(validate(&e, &c, &b, "0.162.0-alpha.2").is_ok());
        assert!(validate(&e, &c, &b, "new").is_err());
        for field in [
            "enabled",
            "policy",
            "workspace",
            "device",
            "surface",
            "service",
        ] {
            let mut bad = e.clone();
            match field {
                "enabled" => bad.enabled = false,
                "policy" => bad.policy_hash = "changed".into(),
                "workspace" => bad.canonical_workspace = root.clone(),
                "device" => bad.device_id = "other".into(),
                "surface" => bad.surface = "desktop".into(),
                _ => bad.service = "other".into(),
            };
            assert!(
                validate(&bad, &c, &b, "0.162.0-alpha.2").is_err(),
                "{field}"
            );
        }
    }
}
