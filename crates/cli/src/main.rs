use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use wk_scope::enrollment::Binding;
#[derive(Parser)]
#[command(
    name = "wk",
    version,
    about = "Workspace Knowledge — local foundation (no live collection or sharing)"
)]
struct Cli {
    /// Private local consent directory, outside the approved product.
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Manage explicit local folder consent. Does not grant team membership.
    Workspace {
        #[command(subcommand)]
        action: Workspace,
    },
    /// Validate a sanitized card JSON file without publishing it.
    ValidateCard { file: PathBuf },
    /// Inspect synthetic JSONL events; never prints raw prompts or responses.
    Replay { root: PathBuf, file: PathBuf },
}
#[derive(Subcommand)]
enum Workspace {
    Approve {
        root: PathBuf,
        #[arg(long)]
        product: String,
        #[arg(long = "service", required = true)]
        services: Vec<String>,
        #[arg(long)]
        yes: bool,
    },
    Status {
        root: PathBuf,
    },
    Check {
        root: PathBuf,
        path: PathBuf,
    },
    Pause {
        root: PathBuf,
    },
    Revoke {
        root: PathBuf,
    },
}
fn limited(path: &Path, max: u64) -> Result<String, String> {
    let f = fs::File::open(path).map_err(|_| "input file unavailable")?;
    let mut s = String::new();
    f.take(max + 1)
        .read_to_string(&mut s)
        .map_err(|_| "invalid input encoding")?;
    if s.len() as u64 > max {
        return Err("input exceeds limit".into());
    }
    Ok(s)
}
fn binding_file(state: &Path, root: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(|_| "workspace unavailable")?;
    Ok(state.join(format!(
        "{:x}.json",
        Sha256::digest(root.as_os_str().to_string_lossy().as_bytes())
    )))
}
fn state_dir(state: Option<&Path>, root: &Path, create: bool) -> Result<PathBuf, String> {
    let state = state.ok_or("--state-dir is required for local consent")?;
    if !state.is_absolute()
        || state
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("state directory must be an absolute path without parent traversal".into());
    }
    let root = root.canonicalize().map_err(|_| "workspace unavailable")?;
    // Resolve the existing prefix before making any directories.
    let mut ancestor = state;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(ancestor.file_name().ok_or("invalid state directory")?);
        ancestor = ancestor.parent().ok_or("invalid state directory")?;
    }
    let mut resolved = ancestor
        .canonicalize()
        .map_err(|_| "state directory unavailable")?;
    for part in suffix.iter().rev() {
        resolved.push(part);
    }
    if resolved.starts_with(&root) {
        return Err("state directory must be outside product folder".into());
    }
    if !state.exists() {
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
fn read_binding(state: &Path, root: &Path) -> Result<Binding, String> {
    serde_json::from_str(&limited(&binding_file(state, root)?, 65536)?)
        .map_err(|_| "invalid workspace binding".into())
}
fn save_binding(state: &Path, binding: &Binding) -> Result<(), String> {
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
fn run(cli: Cli) -> Result<Value, String> {
    match cli.command {
        Command::ValidateCard { file } => {
            let value: Value =
                serde_json::from_str(&limited(&file, 65536)?).map_err(|_| "invalid JSON")?;
            wk_contracts::validate_card(&value)?;
            Ok(json!({"valid":true,"published":false}))
        }
        Command::Replay { root, file } => {
            let state = state_dir(cli.state_dir.as_deref(), &root, false)?;
            let b = read_binding(&state, &root)?;
            wk_capture::inspect(&b, &limited(&file, 8 * 1024 * 1024)?)
        }
        Command::Workspace { action } => match action {
            Workspace::Approve {
                root,
                product,
                services,
                yes,
            } => {
                if !yes {
                    return Err(
                        "review folder and service scope, then pass --yes to approve locally"
                            .into(),
                    );
                }
                let b = Binding::approve(&root, &product, services)?;
                let state = state_dir(cli.state_dir.as_deref(), &root, true)?;
                save_binding(&state, &b)?;
                Ok(
                    json!({"local_approved":true,"root":b.root,"services":b.services,"team_authorized":false,"live_capture_enabled":false}),
                )
            }
            Workspace::Status { root } => {
                let state = state_dir(cli.state_dir.as_deref(), &root, false)?;
                let b = read_binding(&state, &root)?;
                Ok(
                    json!({"root":b.root,"product":b.product,"services":b.services,"enabled":b.enabled,"team_authorized":false,"live_capture_enabled":false}),
                )
            }
            Workspace::Check { root, path } => {
                let state = state_dir(cli.state_dir.as_deref(), &root, false)?;
                let b = read_binding(&state, &root)?;
                Ok(json!({"eligible":b.eligible(&path),"team_authorized":false}))
            }
            Workspace::Pause { root } => {
                let state = state_dir(cli.state_dir.as_deref(), &root, false)?;
                let mut b = read_binding(&state, &root)?;
                b.pause();
                save_binding(&state, &b)?;
                Ok(json!({"enabled":false}))
            }
            Workspace::Revoke { root } => {
                let state = state_dir(cli.state_dir.as_deref(), &root, false)?;
                fs::remove_file(binding_file(&state, &root)?)
                    .map_err(|_| "cannot revoke local binding")?;
                Ok(json!({"revoked":true}))
            }
        },
    }
}
fn main() {
    match run(Cli::parse()) {
        Ok(v) => println!("{v}"),
        Err(message) => {
            eprintln!(
                "{}",
                json!({"error":{"code":"validation","message":message,"retryable":false}})
            );
            std::process::exit(2);
        }
    }
}
