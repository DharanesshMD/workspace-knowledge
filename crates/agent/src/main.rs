mod chat;
mod compliance;
mod cursor_capture;
mod cursor_source;
mod daemon;
mod network;
mod process;
mod state;
mod team;
use clap::{Parser, Subcommand};
use rand::RngCore;
use serde_json::{json, Value};
use std::{io::Read, path::PathBuf};
#[derive(Parser)]
#[command(
    name = "wk-agent",
    version,
    about = "Private workspace collector; native credentials required"
)]
struct Cli {
    #[arg(long)]
    state_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Init {
        #[arg(long)]
        binding: PathBuf,
        #[arg(long)]
        service: String,
        #[arg(long)]
        server: String,
        #[arg(long, default_value = "http://127.0.0.1:11434/v1/chat/completions")]
        inference: String,
        #[arg(long, default_value = "local-model")]
        model: String,
        #[arg(long, default_value_t = 4319)]
        port: u16,
        #[arg(long)]
        yes: bool,
    },
    Serve,
    /// Opt into version-pinned native capture in the enrolled service.
    NativeEnroll {
        #[arg(long)]
        client: String,
        #[arg(long)]
        yes: bool,
    },
    /// Disable native capture before removing hook configuration.
    NativeDisable {
        #[arg(long)]
        client: String,
    },
    /// Opt in to private Cursor 3.23.23 local session recovery (24 hours, reviewed sharing).
    CursorEnroll {
        #[arg(long)]
        yes: bool,
    },
    Hook {
        #[arg(long)]
        client: String,
        #[arg(long)]
        event: String,
    },
    Process,
    Drafts {
        #[arg(long, default_value = "")]
        after: String,
    },
    Submit {
        #[arg(long)]
        id: String,
        #[arg(long)]
        reviewed: bool,
    },
    Doctor,
    Auth,
    /// Pause, resume or show sharing of your live activity ("who is working on what").
    Activity {
        /// pause | resume | status
        action: String,
    },
    ResetCapture {
        #[arg(long)]
        yes: bool,
    },
    Config {
        #[arg(long)]
        client: String,
    },
    Install {
        #[arg(long)]
        client: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    Uninstall {
        #[arg(long)]
        client: String,
        #[arg(long)]
        file: PathBuf,
    },
    Chat {
        #[arg(long)]
        message_file: PathBuf,
        #[arg(long)]
        session: Option<String>,
    },
    Import {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        session: String,
        #[arg(long)]
        approve_transcript: bool,
    },
    Compliance {
        #[arg(long)]
        binding: PathBuf,
    },
}
fn stdin_limited() -> Result<String, String> {
    let mut s = String::new();
    std::io::stdin()
        .take(256 * 1024 + 1)
        .read_to_string(&mut s)
        .map_err(|_| "invalid input")?;
    if s.len() > 256 * 1024 {
        return Err("input too large".into());
    }
    Ok(s)
}
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let fallback = match &cli.command {
        Command::Hook { client, event } => Some(wk_adapters::success(client, event)),
        _ => None,
    };
    let directory = cli.state_dir.clone();
    match run(cli).await {
        Ok(v) => println!("{v}"),
        Err(e) => {
            if let Some(output) = fallback {
                if let Ok(dir) = state::private_dir(&directory, false) {
                    let _ = state::save(&dir.join("capture-gap.json"), &json!({"gap":true}));
                }
                println!("{output}");
                eprintln!("workspace knowledge capture unavailable; coverage is partial");
            } else {
                eprintln!("{}", json!({"error":e}));
                std::process::exit(2);
            }
        }
    }
}
async fn run(cli: Cli) -> Result<Value, String> {
    let create = matches!(&cli.command, Command::Init { .. });
    let state = state::private_dir(&cli.state_dir, create)?;
    match cli.command {
        Command::Init {
            binding,
            service,
            server,
            inference,
            model,
            port,
            yes,
        } => {
            if !yes {
                return Err(
                    "pass --yes after reviewing product, service and inference scope".into(),
                );
            }
            if state.join("agent.json").exists() {
                return Err("agent already configured; refusing credential rotation".into());
            }
            let b: wk_scope::enrollment::Binding =
                serde_json::from_str(&state::read(&binding, 65536)?)
                    .map_err(|_| "invalid local approval")?;
            if state.starts_with(&b.root)
                || !b.services.contains(&service)
                || !b.eligible(&b.root.join(&service))
            {
                return Err("invalid approval or state location".into());
            }
            network::server_url(&server)?;
            wk_summarize::local_url(&inference)?;
            let token = zeroize::Zeroizing::new(
                std::env::var("WK_ACCESS_TOKEN")
                    .map_err(|_| "WK_ACCESS_TOKEN required for enrollment")?,
            );
            let c = state::Config {
                root: b.root.clone(),
                binding_file: binding.canonicalize().map_err(|_| "approval unavailable")?,
                service,
                server,
                model,
                inference,
                port,
                account: "workspace".into(),
                device_id: uuid::Uuid::new_v4().to_string(),
            };
            let policy = network::policy(&c, &b, &token).await?;
            network::post(&c,&token,"/v1/workspace-bindings",&json!({"product":b.product,"device_id":c.device_id,"approval_hash":b.policy_hash,"services":[c.service],"policy_revision":policy["policy_revision"]})).await?;
            for purpose in ["capture-key", "ipc-key"] {
                let e = state::entry(&state, purpose)?;
                match e.get_secret() {
                    Err(keyring::Error::NoEntry) => {
                        let mut key = [0u8; 32];
                        rand::rngs::OsRng.fill_bytes(&mut key);
                        e.set_secret(&key)
                            .map_err(|_| "native key storage failed")?;
                        zeroize::Zeroize::zeroize(&mut key);
                    }
                    Ok(_) => {}
                    Err(_) => return Err("native credential store unavailable".into()),
                }
            }
            state::entry(&state, "server-token")?
                .set_secret(token.as_bytes())
                .map_err(|_| "native key storage failed")?;
            state::save(&state.join("agent.json"), &c)?;
            Ok(
                json!({"enrolled":true,"capture_enabled":false,"next":"wk-agent serve","automatic_publication":false}),
            )
        }
        Command::CursorEnroll { yes } => {
            if !yes {
                return Err("pass --yes to allow private recovery of callback-bound Cursor 3.23.23 local sessions for 24 hours; sharing requires review".into());
            }
            let c = state::load(&state)?;
            let b = state::binding(&c)?;
            let t = state::token(&state, "server-token")?;
            network::policy(&c, &b, &t).await?;
            cursor_source::enroll(&state)?;
            Ok(
                json!({"native_cursor_enrolled":true,"version":cursor_source::VERSION,"surface":"agents-local","raw_ttl_hours":24,"automatic_publication":false,"missed_initial_sessions":"undetectable"}),
            )
        }
        Command::NativeEnroll { client, yes } => {
            if !yes {
                return Err("review native capture boundaries, then pass --yes".into());
            }
            let client = native_enrollment::parse_client(&client)?;
            let c = state::load(&state)?;
            let b = state::binding(&c)?;
            let token = state::token(&state, "server-token")?;
            network::policy(&c, &b, &token).await?;
            native_enrollment::enroll(&state, client)?;
            Ok(json!({"enrolled":true,"automatic_publication":false,"raw_ttl_hours":24}))
        }
        Command::NativeDisable { client } => {
            let client = native_enrollment::parse_client(&client)?;
            let _guard = native_capture::lock(&state).await?;
            native_enrollment::disable(&state, client)?;
            native_capture::clear(&state, Some(client))?;
            Ok(json!({"disabled":true}))
        }
        Command::Serve => daemon::serve(state).await,
        Command::Hook { client, event } => {
            let mut input: Value =
                serde_json::from_str(&stdin_limited()?).map_err(|_| "invalid hook")?;
            if client == "antigravity" && input.get("hook_event_name").is_none() {
                input
                    .as_object_mut()
                    .ok_or("invalid hook")?
                    .insert("hook_event_name".into(), json!(event));
            }
            if let Ok(native) = native_enrollment::parse_client(&client) {
                if native_enrollment::disabled(&state, native)? {
                    return Ok(wk_adapters::success(&client, &event));
                }
                native_enrollment::verify_emitter(&state, native)?;
                input
                    .as_object_mut()
                    .ok_or("invalid hook")?
                    .insert("_wk_emitter_verified".into(), json!(true));
            }
            if input["hook_event_name"] != event {
                return Err("hook event mismatch".into());
            }
            if daemon::send(&state, json!({"client":client,"payload":input}))
                .await
                .is_err()
            {
                state::save(&state.join("capture-gap.json"), &json!({"gap":true}))?;
                return Err("capture delivery failed".into());
            }
            // Team notices are best effort and bounded so the agent is never held up.
            let context = tokio::time::timeout(
                std::time::Duration::from_millis(1500),
                team::on_hook(&state, &client, &event, &input),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
            Ok(wk_adapters::team_context::output(
                &client,
                &event,
                context.as_deref(),
            ))
        }
        Command::Process => process::run(&state).await,
        Command::Drafts { after } => process::list(&state, &after),
        Command::Submit { id, reviewed } => {
            if !reviewed {
                return Err("review the private summary first and pass --reviewed".into());
            }
            process::submit(&state, &id).await
        }
        Command::Auth => {
            let c = state::load(&state)?;
            let b = state::binding(&c)?;
            let t = zeroize::Zeroizing::new(
                std::env::var("WK_ACCESS_TOKEN").map_err(|_| "WK_ACCESS_TOKEN required")?,
            );
            network::policy(&c, &b, &t).await?;
            state::entry(&state, "server-token")?
                .set_secret(t.as_bytes())
                .map_err(|_| "credential update failed")?;
            Ok(json!({"credential_updated":true}))
        }
        Command::Activity { action } => team::activity(&state, &action).await,
        Command::ResetCapture { yes } => {
            if !yes {
                return Err("--yes closes all pending capture sessions and discards their private drafts; start a genuinely new client session afterward".into());
            }
            let _guard = native_capture::lock(&state).await?;
            let mut store = state::store(&state)?;
            loop {
                let rows = store.pending(daemon::now()?)?;
                if rows.is_empty() {
                    break;
                }
                for (id, _) in rows {
                    store.ack(&id)?;
                }
            }
            let gap = state.join("capture-gap.json");
            if gap.exists() {
                std::fs::remove_file(gap).map_err(|_| "cannot clear gap marker")?;
            }
            Ok(json!({"closed_pending_sessions":true,"new_client_session_required":true}))
        }
        Command::Doctor => {
            let c = state::load(&state)?;
            let b = state::binding(&c);
            let secure = state::secret(&state, "capture-key").is_ok();
            let member = match &b {
                Ok(b) => match state::token(&state, "server-token") {
                    Ok(t) => network::policy(&c, b, &t).await.is_ok(),
                    Err(_) => false,
                },
                Err(_) => false,
            };
            let daemon = daemon::health(&state)
                .await
                .unwrap_or(json!({"available":false}));
            Ok(
                json!({"native_clients":native_capture::status(&state).unwrap_or(json!({"unavailable":true})),"native_cursor":cursor_capture::status(&state).unwrap_or(json!({"unavailable":true})),"scope_approved":b.is_ok(),"native_key_available":secure,"membership_current":member,"collector":daemon,"clients":wk_adapters::capabilities(),"mcp_handshake":"not_checked","publication":"review_required"}),
            )
        }
        Command::Config { client } => wk_adapters::config(
            &client,
            &std::env::current_exe().map_err(|_| "executable unavailable")?,
            &state,
        ),
        Command::Install { client, file, yes } => {
            if !yes {
                return Err(
                    "review wk-agent config output first, then pass --yes for the explicit file"
                        .into(),
                );
            }
            state::binding(&state::load(&state)?)?;
            if !file.is_absolute() {
                return Err("explicit absolute config path required".into());
            }
            let fragment = wk_adapters::config(
                &client,
                &std::env::current_exe().map_err(|_| "executable unavailable")?,
                &state,
            )?;
            let old: Value = if file.exists() {
                serde_json::from_str(&state::read(&file, 1024 * 1024)?)
                    .map_err(|_| "config must be strict JSON; JSONC left unchanged")?
            } else {
                json!({})
            };
            let result = wk_adapters::install::merge(&old, &fragment)?;
            let receipt = state.join(format!("install-{client}.json"));
            if receipt.exists() {
                let saved: Value = serde_json::from_str(&state::read(&receipt, 1024 * 1024)?)
                    .map_err(|_| "invalid install receipt")?;
                if saved["file"] != file.to_string_lossy().as_ref() {
                    return Err("client already installed in another file".into());
                }
            } else {
                state::save(&state.join(format!("backup-{client}.json")), &old)?;
            }
            state::save(&file, &result)?;
            state::save(&receipt, &json!({"file":file,"fragment":fragment}))?;
            Ok(json!({"installed":true,"live_verified":false}))
        }
        Command::Uninstall { client, file } => {
            if let Ok(native) = native_enrollment::parse_client(&client) {
                let _guard = native_capture::lock(&state).await?;
                native_enrollment::disable(&state, native)?;
                native_capture::clear(&state, Some(native))?;
            }
            if !wk_contracts::valid_id(&client) {
                return Err("invalid client".into());
            }
            let receipt = state.join(format!("install-{client}.json"));
            let saved: Value = serde_json::from_str(&state::read(&receipt, 1024 * 1024)?)
                .map_err(|_| "invalid install receipt")?;
            if saved["file"] != file.to_string_lossy().as_ref() {
                return Err("config does not match owned installation".into());
            }
            let existing: Value = serde_json::from_str(&state::read(&file, 1024 * 1024)?)
                .map_err(|_| "config must be strict JSON; left unchanged")?;
            state::save(
                &file,
                &wk_adapters::install::remove(&existing, &saved["fragment"])?,
            )?;
            std::fs::remove_file(receipt).map_err(|_| "cannot clear receipt")?;
            Ok(json!({"owned_entries_removed":true,"modified_entries_preserved":true}))
        }
        Command::Chat {
            message_file,
            session,
        } => chat::run(&state, &message_file, session).await,
        Command::Import {
            file,
            session,
            approve_transcript,
        } => chat::import(&state, &file, &session, approve_transcript).await,
        Command::Compliance { binding } => compliance::poll(&state, &binding).await,
    }
}

mod native_enrollment;

mod antigravity_source;

mod native_capture;

mod native_process;
