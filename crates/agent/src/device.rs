//! Device binding and typed controls; all credential material stays in the OS keyring.
use crate::{daemon, network, state};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use wk_contracts::device::*;
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub state_dir: PathBuf,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    config_hash: String,
    approval_hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Remote {
    pub revision: i64,
    pub settings: DeviceSettings,
}
pub struct Effective {
    pub paused: bool,
    pub owner_sync: bool,
    pub activity: bool,
}
pub fn effective(
    local_pause: bool,
    local_owner: bool,
    local_activity: bool,
    r: &DeviceSettings,
) -> Effective {
    Effective {
        paused: local_pause || r.paused,
        owner_sync: local_owner && r.owner_sync && !local_pause && !r.paused,
        activity: local_activity && r.activity && !local_pause && !r.paused,
    }
}
fn directory(c: &state::Config) -> Result<&std::path::Path, String> {
    Ok(&c.device.as_ref().ok_or("device not connected")?.state_dir)
}
fn config_hash(c: &state::Config) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(c).map_err(|_| "invalid device configuration")?)
    ))
}
pub fn save_receipt(c: &state::Config, b: &wk_scope::enrollment::Binding) -> Result<(), String> {
    state::save(
        &directory(c)?.join("device-receipt.json"),
        &Receipt {
            config_hash: config_hash(c)?,
            approval_hash: b.policy_hash.clone(),
        },
    )
}
pub fn validate(c: &state::Config, b: &wk_scope::enrollment::Binding) -> Result<(), String> {
    let dir = directory(c)?;
    let receipt: Receipt =
        serde_json::from_str(&state::read(&dir.join("device-receipt.json"), 1024)?)
            .map_err(|_| "invalid device approval receipt")?;
    let current = state::load(dir)?;
    let binding = state::base_binding(&current)?;
    if current != *c
        || receipt.config_hash != config_hash(c)?
        || receipt.approval_hash != b.policy_hash
        || binding.policy_hash != b.policy_hash
    {
        return Err("device configuration or local approval changed; reconnect locally".into());
    }
    Ok(())
}
pub fn remote(c: &state::Config) -> Result<Remote, String> {
    let path = directory(c)?.join("device-controls.json");
    if !path.exists() {
        return Ok(Remote {
            revision: 0,
            settings: DeviceSettings {
                paused: true,
                ..Default::default()
            },
        });
    }
    serde_json::from_str(&state::read(&path, 1024)?).map_err(|_| "invalid device controls".into())
}
pub fn require_device_for_remote(server: &str, connected: bool) -> Result<(), String> {
    let origin = network::server_url(server)?;
    let u = reqwest::Url::parse(&origin).map_err(|_| "invalid server")?;
    if !connected && u.scheme() == "https" {
        return Err("hosted collectors require device connect; unsigned legacy enrollment is loopback development only".into());
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub fn proof_headers(
    key: &SigningKey,
    origin: &str,
    method: &str,
    path: &str,
    time: i64,
    nonce: uuid::Uuid,
    id: uuid::Uuid,
    token: &str,
    body: &[u8],
) -> Result<reqwest::header::HeaderMap, String> {
    let mut h = reqwest::header::HeaderMap::new();
    let signature = key.sign(&signing_bytes(
        origin, method, path, time, nonce, id, token, body,
    ));
    for (name, value) in [
        ("x-wk-device", id.to_string()),
        ("x-wk-time", time.to_string()),
        ("x-wk-nonce", nonce.to_string()),
        ("x-wk-signature", hex::encode(signature.to_bytes())),
    ] {
        h.insert(
            name,
            reqwest::header::HeaderValue::from_str(&value).map_err(|_| "invalid device proof")?,
        );
    }
    Ok(h)
}
pub async fn signed(
    c: &state::Config,
    token: &str,
    operation: &str,
    request: &Value,
    seconds: u64,
) -> Result<Value, String> {
    let b = state::base_binding(c)?;
    validate(c, &b)?;
    let remote = remote(c)?;
    let local_owner = crate::personal_sync::declared_consent(directory(c)?, c, &b)?;
    let flags = effective(
        !b.enabled,
        local_owner,
        !directory(c)?.join("activity-paused").exists(),
        &remote.settings,
    );
    if matches!(operation, "cards" | "personal" | "activity")
        && (flags.paused
            || operation == "personal" && !flags.owner_sync
            || operation == "activity" && !flags.activity)
    {
        return Err("device privacy setting blocks upload".into());
    }
    let secret = state::secret(directory(c)?, "device-key")?;
    let raw: &[u8; 32] = secret
        .as_slice()
        .try_into()
        .map_err(|_| "invalid device credential")?;
    let key = SigningKey::from_bytes(raw);
    let value = json!({"product":b.product,"approval_hash":b.policy_hash,"request":request});
    send_with_key(c, token, operation, &key, &value, seconds).await
}
pub async fn send_with_key(
    c: &state::Config,
    token: &str,
    operation: &str,
    key: &SigningKey,
    value: &Value,
    seconds: u64,
) -> Result<Value, String> {
    if ![
        "register",
        "policy",
        "cards",
        "personal",
        "activity",
        "activity-clear",
        "overlaps",
        "report",
        "change-context",
        "preflight-consume",
        "repository-enroll",
    ]
    .contains(&operation)
    {
        return Err("unsupported collector operation".into());
    }
    let origin = network::server_url(&c.server)?;
    let path = format!("/collector/{operation}");
    let bytes = serde_json::to_vec(value).map_err(|_| "invalid device request")?;
    if bytes.len() > 65536 {
        return Err("device request too large".into());
    }
    let id = c.device_id.parse().map_err(|_| "invalid device identity")?;
    let headers = proof_headers(
        key,
        &origin,
        "POST",
        &path,
        daemon::now()?,
        uuid::Uuid::new_v4(),
        id,
        token,
        &bytes,
    )?;
    let response = network::client(seconds)?
        .post(format!("{origin}{path}"))
        .bearer_auth(token)
        .headers(headers)
        .header("content-type", "application/json")
        .body(bytes)
        .send()
        .await
        .map_err(|_| "collector service unavailable")?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(network::AUTH_DENIED.into());
    }
    wk_summarize::bounded_json(response, 1024 * 1024).await
}
pub(crate) async fn apply_remote(
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    r: Remote,
) -> Result<Remote, String> {
    let dir = directory(c)?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = options
        .open(dir.join("device-controls.lock"))
        .map_err(|_| "device controls unavailable")?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(_) => return Err("device controls unavailable".into()),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("device controls busy".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    validate(c, b)?;
    let current = remote(c)?;
    if r.revision < 1
        || r.revision < current.revision
        || r.revision == current.revision && r.settings != current.settings
    {
        return Err("stale or conflicting device controls".into());
    }
    state::save(&dir.join("device-controls.json"), &r)?;
    Ok(r)
}
pub async fn poll(dir: &std::path::Path) -> Result<(), String> {
    let c = state::load(dir)?;
    if c.device.is_none() {
        return Ok(());
    }
    let b = state::base_binding(&c)?;
    let t = state::token(dir, "server-token")?;
    let response = signed(&c, &t, "policy", &json!({}), 3).await?;
    let r:Remote=serde_json::from_value(json!({"revision":response["device"]["revision"],"settings":response["device"]["settings"]})).map_err(|_|"invalid device policy")?;
    if r.revision < 1 {
        return Err("invalid device revision".into());
    }
    let r = apply_remote(&c, &b, r).await?;
    let report = health_report(&c, &b, &r)?;
    signed(
        &c,
        &t,
        "report",
        &serde_json::to_value(report).map_err(|_| "invalid report")?,
        3,
    )
    .await?;
    Ok(())
}
fn health_report(
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    r: &Remote,
) -> Result<DeviceReport, String> {
    let dir = directory(c)?;
    let queue = state::store(dir)
        .and_then(|s| s.pending_ids("draft:", "", daemon::now()?, 50))
        .map(|ids| match ids.len() {
            0 => Queue::Empty,
            1..=49 => Queue::Small,
            _ => Queue::Large,
        })
        .unwrap_or(Queue::Unknown);
    report_content(c, b, r, queue)
}
fn report_content(
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    r: &Remote,
    queue: Queue,
) -> Result<DeviceReport, String> {
    let dir = directory(c)?;
    let report = DeviceReport {
        collector_version: env!("CARGO_PKG_VERSION").into(),
        config_revision: r.revision,
        config_status: ConfigStatus::Applied,
        local_pause: !b.enabled,
        local_owner_sync: crate::personal_sync::declared_consent(dir, c, b)?,
        local_activity: !dir.join("activity-paused").exists(),
        queue,
        reason: if r.settings.paused {
            Reason::RemotePause
        } else if dir.join("capture-gap.json").exists() {
            Reason::CaptureGap
        } else {
            Reason::None
        },
        capabilities: vec![
            Capability {
                client: Client::Codex,
                surface: Surface::Cli,
                version: None,
                status: CapabilityStatus::Unknown,
            },
            Capability {
                client: Client::Antigravity,
                surface: Surface::Desktop,
                version: None,
                status: CapabilityStatus::Unknown,
            },
            Capability {
                client: Client::Cursor,
                surface: Surface::Editor,
                version: None,
                status: CapabilityStatus::Unknown,
            },
            Capability {
                client: Client::Claude,
                surface: Surface::Cli,
                version: None,
                status: CapabilityStatus::Unknown,
            },
            Capability {
                client: Client::Gemini,
                surface: Surface::Cli,
                version: None,
                status: CapabilityStatus::Unknown,
            },
        ],
    };
    report.validate()?;
    Ok(report)
}
#[derive(clap::Args)]
pub struct ConnectArgs {
    #[arg(long)]
    pub binding: PathBuf,
    #[arg(long)]
    pub service: String,
    #[arg(long)]
    pub server: String,
    /// Loopback model endpoint; omit it to build drafts from rules instead.
    #[arg(long)]
    pub inference: Option<String>,
    #[arg(long, value_enum)]
    pub summarizer: Option<state::Summarizer>,
    #[arg(long, default_value = "local-model")]
    pub model: String,
    #[arg(long, default_value_t = 4319)]
    pub port: u16,
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub no_browser: bool,
    /// Explicit synthetic loopback token fixture; unavailable for HTTPS deployments.
    #[arg(long)]
    pub test_token: bool,
}
#[derive(clap::Subcommand)]
pub enum Action {
    Connect(ConnectArgs),
    Login {
        #[arg(long)]
        no_browser: bool,
    },
    Status,
    /// Diagnose native capture policy authorization without claiming capture coverage.
    CheckNativePolicy,
    Poll,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthState {
    pub expires_at: i64,
}
async fn authenticate(
    server: &str,
    no_browser: bool,
    test_token: bool,
) -> Result<crate::native_login::Credential, String> {
    if test_token {
        let origin = network::server_url(server)?;
        if !origin.starts_with("http://") {
            return Err("synthetic token is loopback only".into());
        }
        return Ok(crate::native_login::Credential {
            token: zeroize::Zeroizing::new(
                std::env::var("WK_ACCESS_TOKEN")
                    .map_err(|_| "synthetic WK_ACCESS_TOKEN required")?,
            ),
            expires_at: daemon::now()? + 300,
        });
    }
    let attempt = crate::native_login::begin(server).await?;
    eprintln!(
        "{}",
        json!({"authorization_url":attempt.url,"expires_in_seconds":300})
    );
    if !no_browser {
        attempt.open_browser()?;
    }
    attempt.complete().await
}
fn persist_auth(
    dir: &std::path::Path,
    credential: &crate::native_login::Credential,
) -> Result<(), String> {
    state::entry(dir, "server-token")?
        .set_secret(credential.token.as_bytes())
        .map_err(|_| "native credential storage failed")?;
    state::save(
        &dir.join("device-auth.json"),
        &AuthState {
            expires_at: credential.expires_at,
        },
    )
}
pub fn credential_current(dir: &std::path::Path) -> Result<bool, String> {
    let auth: AuthState = serde_json::from_str(&state::read(&dir.join("device-auth.json"), 1024)?)
        .map_err(|_| "invalid device authentication state")?;
    Ok(auth.expires_at > daemon::now()? + 5)
}
pub async fn connect(dir: &std::path::Path, args: ConnectArgs) -> Result<Value, String> {
    if !args.yes {
        return Err(
            "review exact product, service, server and summarizer settings, then pass --yes".into(),
        );
    }
    if dir.join("agent.json").exists() {
        return Err("collector already configured; use device login for authentication or a new private state directory for a new local approval".into());
    }
    let b: wk_scope::enrollment::Binding =
        serde_json::from_str(&state::read(&args.binding, 65536)?)
            .map_err(|_| "invalid workspace approval")?;
    if dir.starts_with(&b.root)
        || !b.services.contains(&args.service)
        || !b.eligible(&b.root.join(&args.service))
    {
        return Err("invalid local approval or state location".into());
    }
    network::server_url(&args.server)?;
    let (summarizer, inference) = state::resolve_summarizer(args.summarizer, args.inference)?;
    let model = if summarizer == state::Summarizer::Local {
        args.model
    } else {
        String::new()
    };
    let c = state::Config {
        root: b.root.clone(),
        binding_file: args
            .binding
            .canonicalize()
            .map_err(|_| "approval unavailable")?,
        service: args.service,
        server: args.server,
        model,
        inference,
        summarizer,
        port: args.port,
        account: "workspace".into(),
        device_id: uuid::Uuid::new_v4().to_string(),
        repository: None,
        device: Some(Connection {
            state_dir: dir.to_path_buf(),
        }),
    };
    let credential = authenticate(&c.server, args.no_browser, args.test_token).await?;
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut *seed);
    let key = SigningKey::from_bytes(&seed);
    let registration = DeviceRegistration {
        id: c.device_id.parse().map_err(|_| "invalid installation")?,
        public_key: hex::encode(key.verifying_key().to_bytes()),
        product: b.product.clone(),
        services: vec![c.service.clone()],
        approval_hash: b.policy_hash.clone(),
    };
    let result = send_with_key(
        &c,
        &credential.token,
        "register",
        &key,
        &serde_json::to_value(registration).map_err(|_| "invalid registration")?,
        10,
    )
    .await?;
    let remote: Remote = serde_json::from_value(
        json!({"revision":result["device"]["revision"],"settings":result["device"]["settings"]}),
    )
    .map_err(|_| "invalid enrollment acknowledgment")?;
    for purpose in ["capture-key", "ipc-key"] {
        let entry = state::entry(dir, purpose)?;
        match entry.get_secret() {
            Err(keyring::Error::NoEntry) => {
                let mut value = zeroize::Zeroizing::new([0u8; 32]);
                rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut *value);
                entry
                    .set_secret(value.as_slice())
                    .map_err(|_| "native credential storage failed")?;
            }
            Ok(_) => {}
            Err(_) => return Err("native credential store unavailable".into()),
        }
    }
    state::entry(dir, "device-key")?
        .set_secret(seed.as_slice())
        .map_err(|_| "native device key storage failed")?;
    persist_auth(dir, &credential)?;
    state::save(&dir.join("activity-paused"), &json!({"paused":true}))?;
    state::save(&dir.join("agent.json"), &c)?;
    save_receipt(&c, &b)?;
    state::save(&dir.join("device-controls.json"), &remote)?;
    Ok(
        json!({"connected":true,"device_id":c.device_id,"owner_sync":false,"activity_permission":false,"capture_coverage":"unknown","automatic_publication":false,"next":"Install and verify hooks locally; enable sharing only after reviewing local and hosted settings."}),
    )
}
pub async fn run(dir: &std::path::Path, action: Action) -> Result<Value, String> {
    match action {
        Action::Connect(args) => connect(dir, args).await,
        Action::Poll => {
            poll(dir).await?;
            status(dir)
        }
        Action::Status => status(dir),
        Action::CheckNativePolicy => {
            crate::native_capture::policy(dir).await?;
            Ok(json!({"native_policy_authorized":true,"capture_coverage":"unknown"}))
        }
        Action::Login { no_browser } => {
            let c = state::load(dir)?;
            let b = state::base_binding(&c)?;
            validate(&c, &b)?;
            let credential = authenticate(&c.server, no_browser, false).await?;
            signed(&c, &credential.token, "policy", &json!({}), 3).await?;
            persist_auth(dir, &credential)?;
            Ok(json!({"authenticated":true,"device_id":c.device_id}))
        }
    }
}
pub fn status(dir: &std::path::Path) -> Result<Value, String> {
    let c = state::load(dir)?;
    if c.device.is_none() {
        return Ok(json!({"registered_device":false,"mode":"legacy_loopback"}));
    }
    let b = state::base_binding(&c)?;
    validate(&c, &b)?;
    Ok(
        json!({"registered_device":true,"device_id":c.device_id,"requested":remote(&c)?,"upstream_auth_current":credential_current(dir).unwrap_or(false),"capture_coverage":"unknown"}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state;
    use ed25519_dalek::SigningKey;
    fn fixture() -> (
        tempfile::TempDir,
        state::Config,
        wk_scope::enrollment::Binding,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        std::fs::create_dir_all(root.join("backend")).unwrap();
        let local = dir.path().join("state");
        std::fs::create_dir(&local).unwrap();
        let b =
            wk_scope::enrollment::Binding::approve(&root, "demo", vec!["backend".into()]).unwrap();
        let c = state::Config {
            root: b.root.clone(),
            binding_file: local.join("binding.json"),
            service: "backend".into(),
            server: "https://team.example".into(),
            model: "local-model".into(),
            inference: "http://127.0.0.1:11434/v1/chat/completions".into(),
            summarizer: crate::state::Summarizer::Local,
            port: 4319,
            account: "workspace".into(),
            device_id: uuid::Uuid::new_v4().to_string(),
            repository: None,
            device: Some(Connection {
                state_dir: local.clone(),
            }),
        };
        state::save(&c.binding_file, &b).unwrap();
        state::save(&local.join("agent.json"), &c).unwrap();
        (dir, c, b)
    }
    #[tokio::test]
    async fn connect_rejects_inconsistent_summarizer_flags_before_signing_in() {
        let (dir, c, _b) = fixture();
        let fresh = dir.path().join("fresh");
        std::fs::create_dir(&fresh).unwrap();
        let args = |summarizer, inference: Option<&str>| ConnectArgs {
            binding: c.binding_file.clone(),
            service: "backend".into(),
            server: "https://team.example".into(),
            inference: inference.map(Into::into),
            summarizer,
            model: "local-model".into(),
            port: 4319,
            yes: true,
            no_browser: true,
            test_token: false,
        };
        let e = connect(&fresh, args(Some(state::Summarizer::Local), None))
            .await
            .unwrap_err();
        assert!(e.contains("needs --inference"), "{e}");
        let e = connect(
            &fresh,
            args(
                Some(state::Summarizer::Rules),
                Some("http://127.0.0.1:11434/v1/chat/completions"),
            ),
        )
        .await
        .unwrap_err();
        assert!(e.contains("cannot be combined"), "{e}");
        assert!(!fresh.join("agent.json").exists());
    }
    #[tokio::test]
    async fn delayed_controls_cannot_roll_back_a_newer_pause_revision() {
        let (_dir, c, b) = fixture();
        save_receipt(&c, &b).unwrap();
        apply_remote(
            &c,
            &b,
            Remote {
                revision: 3,
                settings: DeviceSettings {
                    paused: true,
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
        assert!(apply_remote(
            &c,
            &b,
            Remote {
                revision: 2,
                settings: DeviceSettings::default()
            }
        )
        .await
        .is_err());
        assert_eq!(remote(&c).unwrap().revision, 3);
        assert!(remote(&c).unwrap().settings.paused);
    }
    #[test]
    fn health_content_omits_local_configuration_and_acknowledges_pause() {
        let (_dir, mut c, b) = fixture();
        c.model = "PRIVATE_RAW_CANARY".into();
        let r = Remote {
            revision: 2,
            settings: DeviceSettings {
                paused: true,
                ..Default::default()
            },
        };
        let report = report_content(&c, &b, &r, Queue::Unknown).unwrap();
        let text = serde_json::to_string(&report).unwrap();
        for private in [
            "PRIVATE_RAW_CANARY",
            "binding.json",
            "state_dir",
            "root",
            "inference",
            "session_id",
        ] {
            assert!(!text.contains(private));
        }
        assert_eq!(report.config_revision, 2);
        assert_eq!(report.reason, Reason::RemotePause);
        assert!(report
            .capabilities
            .iter()
            .all(|c| c.status == CapabilityStatus::Unknown));
    }
    #[test]
    fn remote_pause_blocks_capture_but_preserves_control_and_retention_access() {
        let (_dir, c, mut b) = fixture();
        save_receipt(&c, &b).unwrap();
        let dir = c.device.as_ref().unwrap().state_dir.clone();
        state::save(
            &dir.join("device-controls.json"),
            &Remote {
                revision: 2,
                settings: DeviceSettings {
                    paused: true,
                    ..Default::default()
                },
            },
        )
        .unwrap();
        assert!(state::binding(&c).is_err());
        assert!(state::base_binding(&c).is_ok());
        assert!(validate(&c, &b).is_ok());
        b.enabled = false;
        state::save(&c.binding_file, &b).unwrap();
        state::save(
            &dir.join("device-controls.json"),
            &Remote {
                revision: 3,
                settings: DeviceSettings::default(),
            },
        )
        .unwrap();
        assert!(state::binding(&c).is_err());
        assert!(validate(&c, &b).is_ok());
        state::save(&dir.join("device-auth.json"), &AuthState { expires_at: 0 }).unwrap();
        assert!(!credential_current(&dir).unwrap());
        assert!(state::token(&dir, "server-token").is_err());
    }
    #[test]
    fn device_consent_binds_exact_configuration_and_local_approval() {
        let (_dir, c, b) = fixture();
        save_receipt(&c, &b).unwrap();
        assert!(validate(&c, &b).is_ok());
        let mut changed = c.clone();
        changed.server = "https://other.example".into();
        assert!(validate(&changed, &b).is_err());
        let mut different = b;
        different.policy_hash = "aa".repeat(32);
        assert!(validate(&c, &different).is_err());
    }
    #[test]
    fn remote_settings_cannot_override_local_privacy() {
        let remote = DeviceSettings {
            paused: false,
            owner_sync: true,
            activity: true,
        };
        let flags = effective(true, false, false, &remote);
        assert!(flags.paused);
        assert!(!flags.owner_sync);
        assert!(!flags.activity);
        let remote = DeviceSettings {
            paused: true,
            ..Default::default()
        };
        assert!(effective(false, true, true, &remote).paused);
    }
    #[test]
    fn request_signature_binds_origin_route_and_exact_body() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let id = uuid::Uuid::new_v4();
        let nonce = uuid::Uuid::new_v4();
        let proof = proof_headers(
            &key,
            "https://team.example",
            "POST",
            "/collector/policy",
            10,
            nonce,
            id,
            "upstream-token",
            b"{}",
        )
        .unwrap();
        let sig = ed25519_dalek::Signature::from_slice(
            &hex::decode(proof.get("x-wk-signature").unwrap()).unwrap(),
        )
        .unwrap();
        for (origin, path, body, expected) in [
            (
                "https://team.example",
                "/collector/policy",
                b"{}".as_slice(),
                true,
            ),
            (
                "https://other.example",
                "/collector/policy",
                b"{}".as_slice(),
                false,
            ),
            (
                "https://team.example",
                "/collector/cards",
                b"{}".as_slice(),
                false,
            ),
            (
                "https://team.example",
                "/collector/policy",
                b"{} ".as_slice(),
                false,
            ),
        ] {
            assert_eq!(
                key.verifying_key()
                    .verify_strict(
                        &signing_bytes(origin, "POST", path, 10, nonce, id, "upstream-token", body),
                        &sig
                    )
                    .is_ok(),
                expected
            )
        }
    }
    #[test]
    fn unsigned_collectors_are_only_allowed_on_literal_loopback() {
        assert!(require_device_for_remote("http://127.0.0.1:4141", false).is_ok());
        assert!(require_device_for_remote("https://team.example", false).is_err());
        assert!(require_device_for_remote("https://team.example", true).is_ok());
    }
}
