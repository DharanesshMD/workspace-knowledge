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

/// Where a session sits relative to the approval that applies to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// The folder whose approval applies: the session folder itself, or the product folder above
    /// it when the session was opened inside one of that folder's approved services.
    pub root: PathBuf,
    /// The session folder below `root`, `/`-separated, such as `backend`; empty when the
    /// session is `root` itself.
    pub within: String,
}
/// Whether the consent directory holds an approval record for exactly `folder`, in any state.
fn has_record(consent: &Path, folder: &Path) -> bool {
    state_dir(Some(consent), folder, false)
        .and_then(|state| read_binding(&state, folder))
        .is_ok()
}
/// Find the approval that covers `session`. A folder with an approval record of its own keeps it,
/// whatever its state. Otherwise the nearest folder above with a record applies, but only while
/// that approval is in force for `product` and `session` lies inside one of its approved
/// services. A sibling or personal folder under an approved product stays outside the approval.
pub fn place(consent: &Path, session: &Path, product: &str) -> Place {
    let here = Place {
        root: session.to_path_buf(),
        within: String::new(),
    };
    if has_record(consent, session) {
        return here;
    }
    let Some(parent) = session
        .ancestors()
        .skip(1)
        .find(|parent| has_record(consent, parent))
    else {
        return here;
    };
    let Consent::Approved { services, .. } = self::consent(consent, parent, product) else {
        return here;
    };
    let Ok(rest) = session.strip_prefix(parent) else {
        return here;
    };
    let within: Vec<String> = rest
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let within = within.join("/");
    // The same scope test an approval applies to every file: no symlinks, no nested repository
    // other than an approved service.
    if crate::engage::in_services(&format!("{within}/"), &services)
        && wk_scope::approved(parent, session, &services)
    {
        return Place {
            root: parent.to_path_buf(),
            within,
        };
    }
    here
}
/// The approved service the session was opened in, if it was opened inside one.
pub fn session_service(place: &Place, services: &[String]) -> Option<String> {
    let within = format!("{}/", place.within);
    services
        .iter()
        .find(|s| within.starts_with(&format!("{s}/")))
        .cloned()
}
const AMBIGUOUS: &str = "found in more than one service folder; start it with the service folder";
/// A file path an agent typed, as the path from the approved folder that the hooks and the
/// collector store for the same file, such as `backend/src/routes/users.ts`.
///
/// - An absolute path is made relative to the approved folder.
/// - A relative path is read from the session folder first, so `src/x.ts` typed in a session
///   opened in `backend/` is `backend/src/x.ts`. A path already written from the approved folder
///   is also accepted.
/// - From the approved folder itself, a path that starts with no approved service but names an
///   existing file in exactly one service (as `git -C backend status` prints it) gets that
///   service in front.
///
/// Where both readings are possible, the one that exists on disk wins. The error says why a
/// path was left out.
pub fn product_path(place: &Place, services: &[String], raw: &str) -> Result<String, &'static str> {
    let mut typed = raw.trim();
    while let Some(rest) = typed.strip_prefix("./") {
        typed = rest;
    }
    let directory = typed.len() > 1 && (typed.ends_with('/') || typed.ends_with('\\'));
    let typed = typed.trim_end_matches(['/', '\\']);
    let rooted = Path::new(typed).is_absolute() || raw.trim_start().starts_with(['/', '\\']);
    let rel = wk_adapters::team_context::relative(&place.root, typed).ok_or(if rooted {
        "outside the project folder"
    } else {
        "not a plain relative path"
    })?;
    let mut readings = Vec::new();
    if !rooted && !place.within.is_empty() {
        readings.push(format!("{}/{rel}", place.within));
    }
    readings.push(rel.clone());
    let approved = |p: &String| crate::engage::in_services(p, services);
    let exists = |p: &String| place.root.join(p).exists();
    let readings: Vec<String> = readings.into_iter().filter(approved).collect();
    // Nothing on disk (a deleted file): a path that already starts with a service is taken as
    // written, as the tool descriptions ask; otherwise the session-folder reading.
    let found = readings
        .iter()
        .find(|p| exists(p))
        .or_else(|| readings.iter().find(|p| **p == rel))
        .or(readings.first());
    let chosen = match found {
        Some(found) => found.clone(),
        None if rooted || !place.within.is_empty() => {
            return Err("not inside an approved service folder")
        }
        None => {
            let mut hits = services
                .iter()
                .map(|s| format!("{s}/{rel}"))
                .filter(|p| exists(p));
            match (hits.next(), hits.next()) {
                (Some(only), None) => only,
                (Some(_), Some(_)) => return Err(AMBIGUOUS),
                _ => return Err("not inside an approved service folder"),
            }
        }
    };
    Ok(if directory { format!("{chosen}/") } else { chosen })
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
    /// A product folder with `backend` and `web` service repositories and a personal `notes`
    /// folder, approved for `demo` with both services.
    fn product() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().canonicalize().unwrap();
        let root = base.join("product");
        for dir in ["backend/.git", "backend/src/routes", "web/.git", "web/src", "notes"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["backend/src/routes/users.ts", "web/src/app.ts", "web/src/shared.ts"] {
            std::fs::write(root.join(file), "x").unwrap();
        }
        std::fs::create_dir_all(root.join("backend/src/shared.ts")).unwrap();
        let consent = base.join("consent");
        wk_scope::consent::approve(
            Some(&consent),
            &root,
            "demo",
            vec!["backend".into(), "web".into()],
        )
        .unwrap();
        (d, root, consent)
    }
    fn at(root: &Path, within: &str) -> Place {
        Place {
            root: root.to_path_buf(),
            within: within.into(),
        }
    }
    #[test]
    fn a_session_inside_an_approved_service_uses_the_product_approval() {
        let (_d, root, consent) = product();
        assert_eq!(place(&consent, &root, "demo"), at(&root, ""));
        assert_eq!(place(&consent, &root.join("backend"), "demo"), at(&root, "backend"));
        assert_eq!(
            place(&consent, &root.join("backend/src/routes"), "demo"),
            at(&root, "backend/src/routes")
        );
        // A personal folder beside the services stays outside the approval.
        let notes = root.join("notes");
        assert_eq!(place(&consent, &notes, "demo"), at(&notes, ""));
        // So does everything once the approval is for another product.
        let backend = root.join("backend");
        assert_eq!(place(&consent, &backend, "other"), at(&backend, ""));
        // An unrelated folder with no approval above it is its own root.
        let outside = root.parent().unwrap().to_path_buf();
        assert_eq!(place(&consent, &outside, "demo"), at(&outside, ""));
    }
    #[test]
    fn a_nearer_approval_record_wins_whatever_its_state() {
        let (_d, root, consent) = product();
        let backend = root.join("backend");
        std::fs::create_dir_all(backend.join("api")).unwrap();
        // The service folder has an approval of its own, for another product: it is not
        // overridden by the product approval above.
        wk_scope::consent::approve(Some(&consent), &backend, "other", vec!["api".into()]).unwrap();
        assert_eq!(place(&consent, &backend, "demo"), at(&backend, ""));
        assert_eq!(place(&consent, &backend.join("api"), "other"), at(&backend, "api"));
        // A paused product approval covers nothing below it.
        let (_d, root, consent) = product();
        let state = state_dir(Some(&consent), &root, false).unwrap();
        let mut binding = read_binding(&state, &root).unwrap();
        binding.pause();
        wk_scope::consent::save_binding(&state, &binding).unwrap();
        let web = root.join("web");
        assert_eq!(place(&consent, &web, "demo"), at(&web, ""));
    }
    #[cfg(unix)]
    #[test]
    fn a_symlinked_way_into_a_service_is_not_covered() {
        let (_d, root, consent) = product();
        let link = root.join("notes/backend");
        std::os::unix::fs::symlink(root.join("backend"), &link).unwrap();
        // `resolve_folder` canonicalizes, but a link must not count even if a caller does not.
        assert_eq!(place(&consent, &link, "demo"), at(&link, ""));
    }
    #[test]
    fn the_session_service_is_the_one_the_session_was_opened_in() {
        let services = vec!["backend".to_string(), "web".to_string()];
        let root = Path::new("/p");
        assert_eq!(
            session_service(&at(root, "backend/src"), &services).as_deref(),
            Some("backend")
        );
        assert_eq!(session_service(&at(root, "web"), &services).as_deref(), Some("web"));
        assert_eq!(session_service(&at(root, ""), &services), None);
        assert_eq!(session_service(&at(root, "webapp"), &services), None);
    }
    #[test]
    fn typed_paths_become_paths_from_the_product_folder() {
        let (_d, root, _consent) = product();
        let services = vec!["backend".to_string(), "web".to_string()];
        let from_root = at(&root, "");
        let in_backend = at(&root, "backend");
        let path = |p: &Place, raw: &str| product_path(p, &services, raw);
        let users = "backend/src/routes/users.ts";
        // Already from the product folder: unchanged, from either session.
        assert_eq!(path(&from_root, users), Ok(users.into()));
        assert_eq!(path(&in_backend, users), Ok(users.into()));
        assert_eq!(path(&from_root, "./backend/src/routes/users.ts"), Ok(users.into()));
        // Relative to the session folder.
        assert_eq!(path(&in_backend, "src/routes/users.ts"), Ok(users.into()));
        assert_eq!(path(&at(&root, "backend/src"), "routes/users.ts"), Ok(users.into()));
        // A deleted file keeps the session-folder reading, or the service it names.
        assert_eq!(path(&in_backend, "src/old.ts"), Ok("backend/src/old.ts".into()));
        assert_eq!(path(&in_backend, "web/src/gone.ts"), Ok("web/src/gone.ts".into()));
        // Absolute paths inside the project, from either session.
        let absolute = root.join(users).to_string_lossy().into_owned();
        assert_eq!(path(&from_root, &absolute), Ok(users.into()));
        assert_eq!(path(&in_backend, &absolute), Ok(users.into()));
        // Repository-relative from the product folder: the one service holding the file.
        assert_eq!(path(&from_root, "src/routes/users.ts"), Ok(users.into()));
        assert_eq!(path(&from_root, "src/app.ts"), Ok("web/src/app.ts".into()));
        assert_eq!(path(&from_root, "src/shared.ts"), Err(AMBIGUOUS));
        // Directories keep their trailing slash.
        assert_eq!(path(&in_backend, "src/routes/"), Ok("backend/src/routes/".into()));
        // Left out: outside the services, outside the project, or not a plain path.
        let outside = "not inside an approved service folder";
        assert_eq!(path(&from_root, "notes/todo.md"), Err(outside));
        assert_eq!(path(&from_root, "src/missing.ts"), Err(outside));
        assert_eq!(path(&from_root, &root.join("notes/a.md").to_string_lossy()), Err(outside));
        assert_eq!(path(&from_root, "/etc/passwd"), Err("outside the project folder"));
        assert_eq!(path(&in_backend, "../web/src/app.ts"), Err("not a plain relative path"));
        assert_eq!(path(&from_root, ""), Err("not a plain relative path"));
    }
}
