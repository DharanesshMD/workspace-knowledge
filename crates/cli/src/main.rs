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
        /// Cursor agent data directory (`~/.cursor` by default), which holds agent transcripts.
        #[arg(long, global = true)]
        dot_cursor_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: CursorHistory,
    },
}
#[derive(Subcommand)]
enum CursorHistory {
    /// Summarise conversations per workspace folder (titles and counts only).
    List,
    /// Report storage structure only (names, counts, field types; no values or text).
    Diagnose,
    /// Print conversations for one workspace as Markdown, with likely secrets masked.
    Show {
        /// Only chats attributed to this folder (or inside it). Omit to search all chats.
        #[arg(long, required_unless_present = "latest")]
        workspace: Option<PathBuf>,
        /// Only the N most recently updated chats.
        #[arg(long)]
        latest: Option<usize>,
        /// Truncate each message to this many characters (0 = no limit).
        #[arg(long, default_value_t = 1500)]
        max_chars: usize,
    },
    /// Summarize the latest chats in a workspace into knowledge cards with the local model.
    /// Writes one JSON line per chat (card or failure reason) to a new owner-only file.
    Cards {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long, default_value_t = 10)]
        latest: usize,
        #[arg(long, default_value = "frontend")]
        service: String,
        #[arg(long, default_value = "http://127.0.0.1:11434/api/chat")]
        endpoint: String,
        #[arg(long, default_value = "gemma4:e2b-mlx")]
        model: String,
        #[arg(long)]
        out: PathBuf,
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
        Command::CursorHistory {
            cursor_dir,
            dot_cursor_dir,
            action,
        } => cursor_history(cursor_dir, dot_cursor_dir, action),
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
                    json!({"local_approved":true,"binding_file":binding_file(&state, &b.root)?,"root":b.root,"services":b.services,"team_authorized":false,"live_capture_enabled":false}),
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
fn cursor_history(
    dir: Option<PathBuf>,
    dot: Option<PathBuf>,
    action: CursorHistory,
) -> Result<Value, String> {
    use wk_adapters::cursor_history as ch;
    let dir = dir
        .or_else(ch::default_dir)
        .ok_or("Cursor directory not found; pass --cursor-dir")?;
    if !dir.is_dir() {
        return Err("Cursor directory not found; pass --cursor-dir".into());
    }
    let dot = dot.or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cursor")));
    let history = ch::read_with(&dir, dot.as_deref())?;
    match action {
        CursorHistory::Diagnose => ch::diagnose_with(&dir, dot.as_deref()),
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
                json!({"workspaces":workspaces,"unattributed_conversations":unattributed,"empty_conversations":history.empty_conversations,"unreadable_records":history.unreadable_records}),
            )
        }
        CursorHistory::Show {
            workspace,
            latest,
            max_chars,
        } => {
            let mut selected = match &workspace {
                Some(w) => ch::within(&history, w),
                None => history.conversations.clone(),
            };
            if let Some(n) = latest {
                selected.truncate(n);
            }
            let scope = workspace
                .as_ref()
                .map_or("all projects".to_string(), |w| w.display().to_string());
            println!("# Cursor conversations in {scope}\n\n{} conversation(s). Likely secrets are masked; review before exporting.\n", selected.len());
            for c in selected {
                let place = c
                    .workspace
                    .as_ref()
                    .map_or("unattributed".to_string(), |w| w.display().to_string());
                println!(
                    "## {}\n\nProject: {place}\n\n`{}` · {} message(s) · {}\n",
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
        CursorHistory::Cards {
            workspace,
            latest,
            service,
            endpoint,
            model,
            out,
        } => {
            if !wk_contracts::valid_id(&service) {
                return Err("invalid service ID".into());
            }
            let root = workspace
                .canonicalize()
                .map_err(|_| "workspace unavailable")?;
            let mut chats = ch::within(&history, &root);
            chats.truncate(latest);
            chats.reverse(); // chronological
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&out)
                .map_err(|_| "cannot create output file (already exists?)")?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "runtime unavailable")?;
            let home = std::env::var("HOME").unwrap_or_default();
            let mut summary = json!({"chats":0,"cards":0,"failed":0});
            for c in &chats {
                let source = card_source(c, &root, &home);
                // Nothing changed and almost nothing said (e.g. a greeting): no card.
                let said: usize = c
                    .messages
                    .iter()
                    .filter(|m| m.role == "user")
                    .map(|m| m.text.chars().count())
                    .sum();
                if c.files_changed.is_empty() && said < 40 {
                    writeln!(file, "{}", json!({"id":c.id,"observed_at_ms":c.created_at_ms,"files_changed":[],"files_read":[],"skipped":"nothing to record"})).map_err(|_| "cannot write output")?;
                    continue;
                }
                let started = std::time::Instant::now();
                let result = runtime.block_on(wk_summarize::extract(&endpoint, &model, &source));
                let seconds = started.elapsed().as_secs_f64();
                let first_request = c.messages.iter().find(|m| m.role == "user").map(|m| {
                    relativize(&m.text, &root, &home)
                        .chars()
                        .take(400)
                        .collect::<String>()
                });
                let mut line = json!({"id":c.id,"observed_at_ms":c.created_at_ms,"files_changed":c.files_changed,"files_read":c.files_read.iter().take(100).collect::<Vec<_>>(),"first_request":first_request,"source_chars":source.len(),"seconds":seconds});
                summary["chats"] = json!(summary["chats"].as_u64().unwrap_or(0) + 1);
                match result.and_then(|e| {
                    let key = format!("{:x}", Sha256::digest(c.id.as_bytes()));
                    let mut card =
                        wk_summarize::card(e, key, 1, vec![service.clone()], c.complete)?;
                    card.evidence = c
                        .files_changed
                        .iter()
                        .take(50)
                        .map(|f| {
                            serde_json::from_value(json!({"kind":"file_observation","reference":f}))
                                .map_err(|_| "invalid evidence".to_string())
                        })
                        .collect::<Result<_, _>>()?;
                    card.observed_at_ms = c.created_at_ms;
                    card.validate()?;
                    Ok(card)
                }) {
                    Ok(card) => {
                        line["card"] = json!(card);
                        summary["cards"] = json!(summary["cards"].as_u64().unwrap_or(0) + 1);
                    }
                    Err(reason) => {
                        line["error"] = json!(reason);
                        summary["failed"] = json!(summary["failed"].as_u64().unwrap_or(0) + 1);
                    }
                }
                writeln!(file, "{line}").map_err(|_| "cannot write output")?;
                eprintln!(
                    "{} {:.0}s {}",
                    c.id,
                    seconds,
                    if line.get("card").is_some() {
                        "card"
                    } else {
                        "failed"
                    }
                );
            }
            Ok(summary)
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
/// Replace absolute workspace/home prefixes so summaries carry relative paths only.
fn relativize(text: &str, root: &Path, home: &str) -> String {
    let root = format!("{}/", root.display());
    let mut out = text.replace(&root, "");
    if home.len() > 1 {
        out = out.replace(home, "~");
    }
    out
}

/// Summarizer input: the developer's requests, files changed and the agent's closing report.
fn card_source(c: &wk_adapters::cursor_history::Conversation, root: &Path, home: &str) -> String {
    let clip = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let mut out = String::from("Developer requests:\n");
    // One-word confirmations ("yes", "1") only answer the agent; they add no knowledge.
    for m in c
        .messages
        .iter()
        .filter(|m| m.role == "user" && m.text.split_whitespace().count() > 2)
        .take(8)
    {
        out.push_str(&format!(
            "- {}\n",
            clip(&relativize(&m.text, root, home), 1200)
        ));
    }
    if !c.files_changed.is_empty() {
        let files: Vec<&str> = c
            .files_changed
            .iter()
            .take(30)
            .map(String::as_str)
            .collect();
        out.push_str(&format!("\nFiles changed: {}\n", files.join(", ")));
    }
    if let Some(outcome) = &c.outcome {
        out.push_str(&format!(
            "\nAgent's final report:\n{}\n",
            clip(&relativize(outcome, root, home), 4000)
        ));
    }
    clip(&out, 9000)
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
