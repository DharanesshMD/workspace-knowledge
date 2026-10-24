//! Conservative metadata-only path classification. This does not sandbox an agent.
use std::path::{Component, Path};

pub fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}
fn sensitive(path: &Path) -> bool {
    path.components().any(|c| {
        let name = c.as_os_str().to_string_lossy().to_ascii_lowercase();
        name == ".env"
            || name.starts_with(".env.")
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || matches!(
                name.as_str(),
                ".git" | "node_modules" | "id_rsa" | "id_ed25519" | ".wk"
            )
    })
}
pub fn approved(root: &Path, candidate: &Path, services: &[String]) -> bool {
    if sensitive(candidate) {
        return false;
    }
    if candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return false;
    }
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let Ok(resolved) = candidate.canonicalize() else {
        return false;
    };
    if !resolved.starts_with(&root) {
        return false;
    }
    let mut reached_root = false;
    for walked in candidate.ancestors() {
        let Ok(actual) = walked.canonicalize() else {
            return false;
        };
        if actual == root {
            reached_root = true;
            break;
        }
        let Ok(meta) = std::fs::symlink_metadata(walked) else {
            return false;
        };
        if meta.file_type().is_symlink() {
            return false;
        }
        if walked.join(".git").exists() && !services.iter().any(|s| root.join(s) == actual) {
            return false;
        }
    }
    if !reached_root {
        return false;
    }
    let candidate = resolved;
    let Ok(relative) = candidate.strip_prefix(&root) else {
        return false;
    };
    if relative.as_os_str().is_empty() || sensitive(relative) {
        return false;
    }
    services.iter().any(|service| {
        let rel = Path::new(service);
        if !safe_relative(rel) || sensitive(rel) {
            return false;
        }
        // Validate the enrolled path itself, not just the requested candidate.
        let mut service_path = root.clone();
        for part in rel.components() {
            service_path.push(part.as_os_str());
            let Ok(metadata) = std::fs::symlink_metadata(&service_path) else {
                return false;
            };
            if metadata.file_type().is_symlink() {
                return false;
            }
        }
        let Ok(service_root) = service_path.canonicalize() else {
            return false;
        };
        service_root.starts_with(&root)
            && service_root != root
            && candidate.starts_with(service_root)
    })
}

pub mod consent;
pub mod enrollment;
