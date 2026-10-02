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
    about = "Workspace Knowledge — workspace approval, knowledge retrieval and agent tools"
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
    /// Run the native collector/setup CLI.
    Agent {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<std::ffi::OsString>,
    },
    /// Run the separately authorized environment broker CLI.
    Env {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<std::ffi::OsString>,
    },
    Search {
        #[arg(long)]
        product: String,
        #[arg(long)]
        query: String,
        #[arg(long, default_value_t = 8)]
        limit: u32,
    },
    Context {
        #[arg(long)]
        product: String,
        #[arg(long)]
        query: String,
    },

    /// Manage explicit local folder consent. Does not grant team membership.
    Workspace {
        #[command(subcommand)]
        action: Workspace,
    },
    /// Validate a sanitized card JSON file without publishing it.
    ValidateCard { file: PathBuf },
    /// Inspect synthetic JSONL events; never prints raw prompts or responses.
    Replay { root: PathBuf, file: PathBuf },
    /// Read your local Cursor chat history (read-only snapshot) for review and export.
    CursorHistory {
        /// Cursor user-data directory; defaults to the standard location for this OS.
        #[arg(long, global = true)]
        cursor_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: CursorHistory,
    },
}
#[derive(Subcommand)]
enum CursorHistory {
    /// Summarise conversations per workspace folder (titles and counts only).
    List,
    /// Print conversations for one workspace as Markdown, with likely secrets masked.
    Show {
        #[arg(long)]
        workspace: PathBuf,
        /// Truncate each message to this many characters (0 = no limit).
        #[arg(long, default_value_t = 1500)]
        max_chars: usize,
    },
    /// Write confirmed conversations as `wk-agent import` files (secrets masked).
    Export {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        product: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long = "session", required = true)]
        sessions: Vec<String>,
        #[arg(long)]
        yes: bool,
    },
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
        Command::CursorHistory { cursor_dir, action } => cursor_history(cursor_dir, action),
        Command::Agent { args } => forward("wk-agent", cli.state_dir.as_deref(), args),
        Command::Env { args } => forward("wk-env", None, args),
        Command::Search {
            product,
            query,
            limit,
        } => search(&product, &query, limit),
        Command::Context { product, query } => {
            let result = search(&product, &query, 8)?;
            let mut cards = Vec::new();
            for card in result["cards"].as_array().ok_or("invalid response")? {
                let mut next = cards.clone();
                next.push(json!({"id":card["id"],"revision":card["revision"],"title":card["body"]["title"],"intent":card["body"]["intent"]}));
                if serde_json::to_vec(&next)
                    .map_err(|_| "invalid context")?
                    .len()
                    <= 1900
                {
                    cards = next;
                }
            }
            Ok(json!({"cards":cards,"budget_tokens":2000}))
        }

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
fn forward(
    binary: &str,
    state: Option<&Path>,
    args: Vec<std::ffi::OsString>,
) -> Result<Value, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "executable unavailable")?
        .with_file_name(format!("{binary}{}", std::env::consts::EXE_SUFFIX));
    let mut command = std::process::Command::new(executable);
    if let Some(state) = state {
        command.arg("--state-dir").arg(state);
    }
    let status = command
        .args(args)
        .status()
        .map_err(|_| "companion binary unavailable; build workspace")?;
    if !status.success() {
        return Err("companion command failed".into());
    }
    Ok(json!({"completed":true}))
}
fn search(product: &str, query: &str, limit: u32) -> Result<Value, String> {
    if !wk_contracts::valid_id(product) || wk_summarize::sensitive(query) {
        return Err("invalid or sensitive search".into());
    }
    let base = std::env::var("WK_SERVER_URL").map_err(|_| "WK_SERVER_URL required")?;
    let u = reqwest::Url::parse(&base).map_err(|_| "invalid server URL")?;
    if !(u.scheme() == "https"
        || (u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "[::1]"))))
        || !u.username().is_empty()
        || u.password().is_some()
        || u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err("invalid server origin".into());
    }
    let token = std::env::var("WK_ACCESS_TOKEN")
        .or_else(|_| std::env::var("WK_DEV_TOKEN"))
        .map_err(|_| "knowledge access token required")?;
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "network unavailable")?;
    let response = client
        .post(format!("{}/v1/search", base.trim_end_matches('/')))
        .bearer_auth(token)
        .json(&json!({"product":product,"query":query,"limit":limit}))
        .send()
        .map_err(|_| "knowledge unavailable")?;
    if !response.status().is_success() {
        return Err("knowledge request denied or unavailable".into());
    }
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "invalid response")?;
    if bytes.len() > 1024 * 1024 {
        return Err("response exceeds limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid response".into())
}
fn cursor_history(dir: Option<PathBuf>, action: CursorHistory) -> Result<Value, String> {
    use wk_adapters::cursor_history as ch;
    let dir = dir
        .or_else(ch::default_dir)
        .ok_or("Cursor directory not found; pass --cursor-dir")?;
    if !dir.is_dir() {
        return Err("Cursor directory not found; pass --cursor-dir".into());
    }
    let history = ch::read(&dir)?;
    match action {
        CursorHistory::List => {
            let mut groups: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
            let mut unattributed = 0;
            for c in &history.conversations {
                match &c.workspace {
                    Some(w) => groups
                        .entry(w.display().to_string())
                        .or_default()
                        .push(json!({"id":c.id,"title":c.title,"messages":c.messages.len()})),
                    None => unattributed += 1,
                }
            }
            let workspaces: Vec<Value> = groups
                .into_iter()
                .map(|(w, cs)| json!({"workspace":w,"conversations":cs.len(),"items":cs}))
                .collect();
            Ok(
                json!({"workspaces":workspaces,"unattributed_conversations":unattributed,"unreadable_records":history.unreadable_records}),
            )
        }
        CursorHistory::Show {
            workspace,
            max_chars,
        } => {
            let selected = ch::within(&history, &workspace);
            println!("# Cursor conversations in {}\n\n{} conversation(s). Likely secrets are masked; review before exporting.\n", workspace.display(), selected.len());
            for c in selected {
                println!(
                    "## {}\n\n`{}` · {} message(s) · {}\n",
                    c.title,
                    c.id,
                    c.messages.len(),
                    c.source
                );
                for m in &c.messages {
                    let mut text = ch::redact(&m.text);
                    if max_chars > 0 && text.chars().count() > max_chars {
                        text = text.chars().take(max_chars).collect::<String>() + " …[truncated]";
                    }
                    println!("**{}:** {}\n", m.role, text);
                }
            }
            Ok(Value::Null)
        }
        CursorHistory::Export {
            workspace,
            product,
            out,
            sessions,
            yes,
        } => {
            if !yes {
                return Err("review `cursor-history show` first, then pass --yes to export the listed sessions".into());
            }
            if !wk_contracts::valid_id(&product) {
                return Err("invalid product ID".into());
            }
            let root = workspace
                .canonicalize()
                .map_err(|_| "workspace unavailable")?;
            let selected = ch::within(&history, &root);
            let mut chosen = Vec::new();
            for id in &sessions {
                chosen.push(
                    selected
                        .iter()
                        .find(|c| &c.id == id)
                        .ok_or("a requested session is not in this workspace")?,
                );
            }
            if !out.is_absolute() {
                return Err("--out must be an absolute path".into());
            }
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .create(&out)
                .map_err(|_| "cannot create output directory")?;
            let out = out
                .canonicalize()
                .map_err(|_| "output directory unavailable")?;
            if out.starts_with(&root) {
                return Err("output directory must be outside the workspace".into());
            }
            let mut written = Vec::new();
            for c in chosen {
                let body = json!({"schema_version":1,"workspace":root,"product":product,"session_id":c.id,
                    "messages":c.messages.iter().map(|m| json!({"id":m.id,"role":m.role,"text":ch::redact(&m.text)})).collect::<Vec<_>>()});
                let path = out.join(format!("{}.json", c.id.replace(':', "_")));
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options
                    .open(&path)
                    .map_err(|_| "cannot create export file (already exists?)")?;
                file.write_all(&serde_json::to_vec_pretty(&body).map_err(|_| "invalid export")?)
                    .map_err(|_| "cannot write export")?;
                written.push(path);
            }
            Ok(
                json!({"exported":written,"published":false,"next":"wk-agent import --file FILE --session SESSION_ID --approve-transcript"}),
            )
        }
    }
}
fn main() {
    match run(Cli::parse()) {
        Ok(Value::Null) => {}
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
