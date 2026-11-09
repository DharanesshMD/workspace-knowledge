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
#[cfg(target_os = "linux")]
fn identity(root: &Path) -> Result<String, String> {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    // Linux birth times may share a coarse clock tick with a replacement inode.
    // Export handles include the filesystem generation rather than a timestamp.
    #[repr(C)]
    struct Handle {
        header: libc::file_handle,
        bytes: [u8; 128],
    }
    let meta = std::fs::metadata(root).map_err(|_| "workspace unavailable")?;
    let mut handle = Handle {
        header: libc::file_handle {
            handle_bytes: 128,
            handle_type: 0,
            f_handle: [],
        },
        bytes: [0; 128],
    };
    let path = std::ffi::CString::new(root.as_os_str().as_bytes())
        .map_err(|_| "invalid workspace path")?;
    let mut mount = 0;
    // SAFETY: the initialized C header is followed by its advertised 128 writable
    // bytes, and the path and output pointers remain valid throughout the call.
    let result = unsafe {
        libc::name_to_handle_at(
            libc::AT_FDCWD,
            path.as_ptr(),
            (&raw mut handle).cast::<libc::file_handle>(),
            &mut mount,
            0,
        )
    };
    let len = handle.header.handle_bytes as usize;
    if result != 0 || len == 0 || len > handle.bytes.len() {
        return Err(
            "filesystem cannot provide a stable workspace handle; local consent unavailable".into(),
        );
    }
    Ok(format!(
        "{}:{}:{:?}",
        meta.dev(),
        handle.header.handle_type,
        &handle.bytes[..len]
    ))
}
#[cfg(not(target_os = "linux"))]
fn identity(root: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(root).map_err(|_| "workspace unavailable")?;
    let created = meta.created().map_err(|_| {
        "workspace creation time unavailable; cannot bind consent to this filesystem"
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Inodes can be recycled immediately after deletion. Birth time remains
        // stable during ordinary edits and distinguishes the replacement folder.
        Ok(format!("{}:{}:{created:?}", meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        Ok(format!("{created:?}"))
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
