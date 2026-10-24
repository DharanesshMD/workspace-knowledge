//! Which project folder the bridge serves, where local consent lives, and whether the folder is
//! approved. Everything here reads local files; approving is `approval.rs` and needs a human.
use serde_json::Value;
use std::path::{Path, PathBuf};
use wk_scope::consent::{read_binding, state_dir};

/// Where consent records are kept: `WK_CONSENT_DIR`, else the documented per-user default.
/// The caller validates the result with the same rules as `wk --state-dir`.
pub fn consent_dir(env: Option<&str>, home: Option<&str>) -> Result<PathBuf, String> {
    if let Some(dir) = env.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = home
        .filter(|h| !h.is_empty())
        .ok_or("cannot find the consent directory; set WK_CONSENT_DIR to an absolute path")?;
    Ok(Path::new(home).join(".local/share/workspace-knowledge/consent"))
}
pub fn consent_dir_from_env() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"));
    consent_dir(
        std::env::var("WK_CONSENT_DIR").ok().as_deref(),
        home.ok().as_deref(),
    )
}
/// The path named by a `file://` URI, if it is one.
fn file_root(uri: &str) -> Option<PathBuf> {
    reqwest::Url::parse(uri)
        .ok()
        .filter(|u| u.scheme() == "file")
        .and_then(|u| u.to_file_path().ok())
}
/// The folder this session works in: the first `file://` root the client listed, otherwise the
/// bridge's own working directory. It must be an existing directory and is canonicalized.
pub fn resolve_folder(roots: Option<&[String]>, cwd: Option<PathBuf>) -> Result<PathBuf, String> {
    let chosen = roots
        .and_then(|roots| roots.iter().find_map(|uri| file_root(uri)))
        .or(cwd)
        .ok_or("cannot tell which project folder this session is in")?;
    let folder = chosen
        .canonicalize()
        .map_err(|_| "the project folder is unavailable")?;
    if !folder.is_dir() {
        return Err("the project folder is not a directory".into());
    }
    Ok(folder)
}
/// The services the server grants the signed-in user (`/v1/policy`) that exist as approvable
/// subdirectories of `folder`, in the server's order.
pub fn candidate_services(folder: &Path, policy: &Value) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for service in policy["services"].as_array().into_iter().flatten() {
        let Some(id) = service.as_str().filter(|s| wk_contracts::valid_id(s)) else {
            continue;
        };
        // The very test `wk workspace approve` applies to each service directory.
        if !found.iter().any(|f| f == id)
            && folder.join(id).is_dir()
            && wk_scope::approved(folder, &folder.join(id), &[id.to_string()])
        {
            found.push(id.to_string());
        }
    }
    found
}
/// What the consent directory says about one folder, re-read from disk each time.
#[derive(Debug, PartialEq, Eq)]
pub enum Consent {
    /// No binding for this folder.
    None,
    /// A binding exists for another product; approving would replace it.
    OtherProduct(String),
    /// The binding was paused (`wk workspace pause`).
    Paused,
    /// The binding does not verify: edited, or the folder was replaced.
    Changed,
    /// Approved for this product; only services still passing the scope check are listed.
    Approved {
        services: Vec<String>,
        binding_file: PathBuf,
    },
}
pub fn consent(consent: &Path, folder: &Path, product: &str) -> Consent {
    let Ok(state) = state_dir(Some(consent), folder, false) else {
        return Consent::None;
    };
    let Ok(binding) = read_binding(&state, folder) else {
        return Consent::None;
    };
    if binding.root != folder {
        return Consent::Changed;
    }
    if binding.product != product {
        return Consent::OtherProduct(binding.product);
    }
    if !binding.enabled {
        return Consent::Paused;
    }
    if !binding.intact() {
        return Consent::Changed;
    }
    let services: Vec<String> = binding
        .services
        .iter()
        .filter(|s| binding.eligible(&folder.join(s)))
        .cloned()
        .collect();
    match wk_scope::consent::binding_file(&state, folder) {
        Ok(binding_file) if !services.is_empty() => Consent::Approved {
            services,
            binding_file,
        },
        _ => Consent::Changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_consent_directory_comes_from_the_environment_or_the_documented_default() {
        assert_eq!(
            consent_dir(Some("/var/consent"), Some("/home/me")).unwrap(),
            Path::new("/var/consent")
        );
        assert_eq!(
            consent_dir(None, Some("/home/me")).unwrap(),
            Path::new("/home/me/.local/share/workspace-knowledge/consent")
        );
        assert_eq!(
            consent_dir(Some(""), Some("/home/me")).unwrap(),
            Path::new("/home/me/.local/share/workspace-knowledge/consent")
        );
        assert!(consent_dir(None, None)
            .unwrap_err()
            .contains("WK_CONSENT_DIR"));
        // A relative override is returned as given; the shared validation refuses it.
        let d = tempfile::tempdir().unwrap();
        let relative = consent_dir(Some("consent"), None).unwrap();
        assert!(wk_scope::consent::check_state_dir(&relative, d.path()).is_err());
    }
    #[test]
    fn the_first_file_root_wins_otherwise_the_working_directory() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, cwd) = (d.path().join("a"), d.path().join("b"), d.path().join("cwd"));
        for dir in [&a, &b, &cwd] {
            std::fs::create_dir(dir).unwrap();
        }
        let uri = |p: &Path| reqwest::Url::from_file_path(p).unwrap().to_string();
        let roots = vec!["https://example.com/x".to_string(), uri(&a), uri(&b)];
        let canonical = |p: &Path| p.canonicalize().unwrap();
        assert_eq!(
            resolve_folder(Some(&roots), Some(cwd.clone())).unwrap(),
            canonical(&a)
        );
        // Roots the client did not advertise, or without any file:// entry, fall back to cwd.
        assert_eq!(
            resolve_folder(None, Some(cwd.clone())).unwrap(),
            canonical(&cwd)
        );
        let remote = vec!["https://example.com/x".to_string()];
        assert_eq!(
            resolve_folder(Some(&remote), Some(cwd.clone())).unwrap(),
            canonical(&cwd)
        );
        assert_eq!(
            resolve_folder(Some(&[]), Some(cwd.clone())).unwrap(),
            canonical(&cwd)
        );
        // The result is canonical: a symlink or `..` spelling resolves to the real directory.
        let dotted = cwd.join("..").join("a");
        assert_eq!(resolve_folder(None, Some(dotted)).unwrap(), canonical(&a));
        // It must exist and be a directory; a missing root is an error, not a silent fallback.
        let gone = vec![uri(&d.path().join("gone"))];
        assert!(resolve_folder(Some(&gone), Some(cwd.clone())).is_err());
        std::fs::write(d.path().join("file"), "x").unwrap();
        assert!(resolve_folder(None, Some(d.path().join("file"))).is_err());
        assert!(resolve_folder(None, None).is_err());
    }
    #[test]
    fn candidates_are_the_users_services_that_exist_as_subfolders() {
        let d = tempfile::tempdir().unwrap();
        for dir in ["backend", "web", "unrelated"] {
            std::fs::create_dir(d.path().join(dir)).unwrap();
        }
        std::fs::write(d.path().join("docs"), "a file, not a folder").unwrap();
        let folder = d.path().canonicalize().unwrap();
        let policy =
            json!({"services": ["backend", "docs", "missing", "web", "../etc", "backend", 7]});
        assert_eq!(candidate_services(&folder, &policy), ["backend", "web"]);
        assert!(candidate_services(&folder, &json!({})).is_empty());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.path().join("web"), d.path().join("link")).unwrap();
            assert!(candidate_services(&folder, &json!({"services": ["link"]})).is_empty());
        }
    }
}
