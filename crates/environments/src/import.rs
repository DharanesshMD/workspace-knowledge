use crate::{openbao::Values, profiles::key, Error, Result};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};
/// Reject symlink traversal, even for paths whose ultimate target lies inside the root.
pub fn confined(root: &Path, path: &Path, must_exist: bool) -> Result<PathBuf> {
    if !root.is_absolute()
        || !path.is_absolute()
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(Error::Invalid);
    }
    let root = fs::canonicalize(root).map_err(|_| Error::Local)?;
    let relative = path.strip_prefix(&root).map_err(|_| Error::Denied)?;
    if relative.as_os_str().is_empty() {
        return Err(Error::Denied);
    }
    let mut cursor = root.clone();
    for part in relative.components() {
        if !matches!(part, Component::Normal(_)) {
            return Err(Error::Denied);
        }
        cursor.push(part);
        match fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(Error::Denied),
            Ok(_) => {
                if !fs::canonicalize(&cursor)
                    .map_err(|_| Error::Local)?
                    .starts_with(&root)
                {
                    return Err(Error::Denied);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && cursor == path && !must_exist => {
            }
            Err(_) => return Err(Error::Local),
        }
    }
    Ok(cursor)
}
pub fn parse(input: &str) -> Result<Values> {
    if input.len() > 1_048_576 {
        return Err(Error::Invalid);
    }
    let mut values = Values::new();
    for line in input.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once('=').ok_or(Error::Invalid)?;
        // Literal format, no quoting/expansion/source semantics.
        if !key(name) || value.contains('\0') || values.insert(name.into(), value.into()).is_some()
        {
            return Err(Error::Invalid);
        }
    }
    Ok(values)
}
pub fn read(root: &Path, file: &Path, approved: bool) -> Result<Values> {
    if !approved {
        return Err(Error::Denied);
    }
    let path = confined(root, file, true)?;
    if fs::metadata(&path).map_err(|_| Error::Local)?.len() > 1_048_576 {
        return Err(Error::Invalid);
    }
    parse(&fs::read_to_string(path).map_err(|_| Error::Local)?)
}
pub fn export(root: &Path, destination: &Path, values: &Values) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (root, destination, values);
        Err(Error::Unsupported)
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let path = confined(root, destination, false)?;
        let relative = path.strip_prefix(root).map_err(|_| Error::Denied)?;
        let tracked = Command::new("git")
            .current_dir(root)
            .args(["ls-files", "--error-unmatch", "--"])
            .arg(relative)
            .output()
            .map_err(|_| Error::Local)?;
        let ignored = Command::new("git")
            .current_dir(root)
            .args(["check-ignore", "--quiet", "--"])
            .arg(relative)
            .status()
            .map_err(|_| Error::Local)?;
        if tracked.status.success() || tracked.status.code() != Some(1) || !ignored.success() {
            return Err(Error::Denied);
        }
        if values
            .iter()
            .any(|(k, v)| !key(k) || v.contains(['\n', '\r', '\0']))
        {
            return Err(Error::Invalid);
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|_| Error::Local)?;
        for (key, value) in values {
            writeln!(file, "{key}={value}").map_err(|_| Error::Local)?;
        }
        file.sync_all().map_err(|_| Error::Local)?;
        Ok(())
    }
}
