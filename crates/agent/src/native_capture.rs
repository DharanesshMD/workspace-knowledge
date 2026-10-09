use crate::{
    antigravity_source,
    cursor_source::SourceBinding,
    daemon,
    native_enrollment::{self, Enrollment},
    network, state,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use wk_adapters::{
    antigravity_session::{self, Snapshot},
    codex_session,
    native_evidence::*,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub enrollment: Enrollment,
    pub expires_at: i64,
    pub last_checked: i64,
    pub source: Option<SourceBinding>,
    pub snapshot: Option<Snapshot>,
    pub turns: Vec<Evidence>,
    pub hold_reason: Option<String>,
}

pub fn read(store: &wk_local::LocalStore, key: &str, now: i64) -> Result<Option<Record>, String> {
    store
        .read_record(key, now)?
        .map(|v| serde_json::from_slice(&v).map_err(|_| "invalid_native_record".into()))
        .transpose()
}
fn new_record(e: &Enrollment, now: i64) -> Result<Record, String> {
    Ok(Record {
        enrollment: e.clone(),
        expires_at: now.checked_add(86400).ok_or("invalid_expiry")?,
        last_checked: now,
        source: None,
        snapshot: None,
        turns: vec![],
        hold_reason: None,
    })
}
fn hold(r: &mut Record, reason: &str) {
    r.hold_reason.get_or_insert_with(|| reason.into());
    r.turns.clear();
    r.source = None;
    r.snapshot = None;
}
fn encode(r: &Record) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(r).map_err(|_| "invalid_native_record")?;
    if bytes.len() > 900 * 1024 {
        return Err("native_record_quota".into());
    }
    Ok(bytes)
}
fn ingest_codex(
    store: &mut wk_local::LocalStore,
    key: &str,
    e: &Enrollment,
    p: &Value,
    now: i64,
) -> Result<(), String> {
    store.update(key, now, 86400, |old| {
        let mut r: Record = match old {
            Some(v) => serde_json::from_slice(v).map_err(|_| "invalid_native_record")?,
            None => new_record(e, now)?,
        };
        if r.enrollment.policy_hash != e.policy_hash || now >= r.expires_at {
            hold(&mut r, "source_or_policy_changed");
        }
        if r.hold_reason.is_none() {
            let index = r.turns.iter().position(|t| p["turn_id"] == t.turn);
            match codex_session::apply(index.map(|i| &r.turns[i]), p, &e.canonical_workspace) {
                Ok(turn) => {
                    if turn.hold_reason.is_some() && turn.terminal != Terminal::Cancelled {
                        hold(&mut r, "unsupported_session_context");
                    } else if let Some(i) = index {
                        r.turns[i] = turn;
                    } else {
                        r.turns.push(turn);
                    }
                }
                Err(_) => hold(&mut r, "invalid_native_observation"),
            }
        }
        r.last_checked = now;
        encode(&r)
    })
}
pub async fn policy(dir: &Path) -> Result<Value, String> {
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let token = state::token(dir, "server-token")?;
    let response = network::client(1)?
        .get(format!("{}/v1/policy", network::server_url(&c.server)?))
        .query(&[("product", &b.product)])
        .bearer_auth(&*token)
        .send()
        .await
        .map_err(|_| "membership_service_unavailable")?;
    if matches!(response.status().as_u16(), 401 | 403) {
        let _guard = lock(dir).await?;
        clear(dir, None)?;
        return Err("native_membership_revoked".into());
    }
    let v = wk_summarize::bounded_json(response, 65536).await?;
    if !v["policy"]["services"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s == &c.service))
    {
        let _guard = lock(dir).await?;
        clear(dir, None)?;
        return Err("native_membership_revoked".into());
    }
    Ok(v["policy"].clone())
}
pub async fn lock(dir: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(dir.join("native-capture.lock"))
        .map_err(|_| "native_lock_unavailable")?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(_) => return Err("native_lock_unavailable".into()),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("native_capture_busy".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
pub fn clear(dir: &Path, client: Option<Client>) -> Result<(), String> {
    clear_store(&mut state::store(dir)?, daemon::now()?, client)
}
fn clear_store(
    store: &mut wk_local::LocalStore,
    now: i64,
    client: Option<Client>,
) -> Result<(), String> {
    for prefix in ["native2:", "draft:"] {
        let mut after = String::new();
        loop {
            let ids = store.pending_ids(prefix, &after, now, 100)?;
            if ids.is_empty() {
                break;
            }
            for key in ids {
                after = key.clone();
                let bytes = store.read_record(&key, now)?.ok_or("source_expired")?;
                let owned = if prefix == "native2:" {
                    let r: Record =
                        serde_json::from_slice(&bytes).map_err(|_| "invalid_native_record")?;
                    client.is_none_or(|c| c == r.enrollment.client)
                } else {
                    let v: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_draft")?;
                    v.get("native_adapter")
                        .is_some_and(|a| client.is_none_or(|c| a == native_enrollment::name(c)))
                };
                if owned {
                    store.ack(&key)?;
                }
            }
        }
    }
    Ok(())
}
pub async fn receive(dir: &Path, client: Client, p: &Value) -> Result<Value, String> {
    if serde_json::to_vec(p).map_err(|_| "invalid_hook")?.len() > 256 * 1024 {
        return Err("hook_exceeds_limit".into());
    }
    if native_enrollment::disabled(dir, client)? {
        return Ok(json!({"disabled":true}));
    }
    native_enrollment::load(dir, client)?;
    policy(dir).await?;
    let _guard = lock(dir).await?;
    if native_enrollment::disabled(dir, client)? {
        return Ok(json!({"disabled":true}));
    }
    let e = native_enrollment::load(dir, client)?;
    if p["_wk_emitter_verified"] != true {
        return Err("native_emitter_unverified".into());
    }
    let mut payload = p.clone();
    payload
        .as_object_mut()
        .ok_or("invalid_hook")?
        .remove("_wk_emitter_verified");
    let p = &payload;
    let now = daemon::now()?;
    let session = match client {
        Client::Codex => p["session_id"].as_str(),
        Client::Antigravity => p["conversationId"].as_str(),
    }
    .ok_or("session_missing")?;
    if session.is_empty()
        || session.len() > 128
        || !session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err("session_invalid".into());
    }
    let identity = match client {
        Client::Codex => {
            let check = codex_session::apply(None, p, &e.canonical_workspace)?;
            check.agent
        }
        Client::Antigravity => "conversation".into(),
    };
    let key = format!(
        "native2:{:x}",
        Sha256::digest(
            serde_json::to_vec(&(client, &e.device_id, session, identity))
                .map_err(|_| "invalid_identity")?
        )
    );
    let mut store = state::store(dir)?;
    match client {
        Client::Codex => ingest_codex(&mut store, &key, &e, p, now)?,
        Client::Antigravity => {
            let source = antigravity_source::bind(&e, p, now)?;
            store.update(&key, now, 86400, |old| {
                let mut r: Record = match old {
                    Some(v) => serde_json::from_slice(v).map_err(|_| "invalid_native_record")?,
                    None => new_record(&e, now)?,
                };
                if r.enrollment.policy_hash != e.policy_hash {
                    hold(&mut r, "policy_changed");
                }
                if r.hold_reason.is_none() {
                    if let Some(bound) = &r.source {
                        if bound.path != source.path
                            || bound.device != source.device
                            || bound.inode != source.inode
                        {
                            hold(&mut r, "source_replaced");
                        }
                    }
                    if r.hold_reason.is_none() && r.source.is_none() {
                        r.source = Some(source);
                    }
                    if !matches!(
                        p["hook_event_name"].as_str(),
                        Some("PreInvocation" | "PostInvocation" | "Stop")
                    ) {
                        hold(&mut r, "unsupported_event");
                    }
                }
                r.last_checked = now;
                encode(&r)
            })?;
        }
    }
    Ok(json!({"stored":true,"automatic_publication":false}))
}
pub async fn revalidate(dir: &Path, key: &str) -> Result<Record, String> {
    let now = daemon::now()?;
    let mut store = state::store(dir)?;
    let before = read(&store, key, now)?.ok_or("source_expired")?;
    let current = native_enrollment::load(dir, before.enrollment.client);
    if !current.is_ok_and(|e| {
        e.policy_hash == before.enrollment.policy_hash
            && e.version == before.enrollment.version
            && e.device_id == before.enrollment.device_id
            && e.canonical_workspace == before.enrollment.canonical_workspace
            && e.service == before.enrollment.service
    }) {
        let _guard = lock(dir).await?;
        clear(dir, Some(before.enrollment.client))?;
        return Err("native_enrollment_changed".into());
    }
    policy(dir).await?;
    let _guard = lock(dir).await?;
    let latest = native_enrollment::load(dir, before.enrollment.client)?;
    if latest.policy_hash != before.enrollment.policy_hash
        || latest.canonical_workspace != before.enrollment.canonical_workspace
        || latest.device_id != before.enrollment.device_id
        || latest.version != before.enrollment.version
    {
        clear(dir, Some(before.enrollment.client))?;
        return Err("native_enrollment_changed".into());
    }
    store.update(key, now, (before.expires_at - now).min(86400), |old| {
        let mut r: Record = serde_json::from_slice(old.ok_or("source_expired")?)
            .map_err(|_| "invalid_native_record")?;
        if r.hold_reason.is_none() {
            if r.enrollment.client == Client::Antigravity {
                let source = r.source.as_ref().ok_or("source_missing")?;
                let result = antigravity_source::read_bound(source, now).and_then(|bytes| {
                    antigravity_session::reconcile(r.snapshot.as_ref(), &bytes, None)
                });
                match result {
                    Ok(mut snapshot) => {
                        for t in &mut snapshot.turns {
                            t.session = source.conversation_id.clone();
                        }
                        r.turns = snapshot.turns.clone();
                        r.snapshot = Some(snapshot);
                    }
                    Err(_) => hold(&mut r, "source_reconciliation_failed"),
                }
            } else if now - r.last_checked >= 900
                && r.turns.iter().any(|t| t.terminal == Terminal::Open)
            {
                hold(&mut r, "stale_open_turn");
            }
        }
        encode(&r)
    })?;
    read(&store, key, now)?.ok_or("source_expired".into())
}
pub async fn reconcile(dir: &Path) -> Result<Value, String> {
    let mut store = state::store(dir)?;
    let ids = store.next_ids("native2-reconcile", "native2:", daemon::now()?, 10)?;
    let mut checked = 0;
    for id in ids {
        if revalidate(dir, &id).await.is_ok() {
            checked += 1;
        }
    }
    Ok(json!({"checked":checked}))
}
pub fn status(dir: &Path) -> Result<Value, String> {
    let store = state::store(dir)?;
    let now = daemon::now()?;
    let ids = store.pending_ids("native2:", "", now, 100)?;
    let mut reasons = std::collections::BTreeMap::<String, usize>::new();
    let mut complete = 0;
    let mut held = 0;
    let mut partial = 0;
    let mut cancelled = 0;
    let mut last = 0;
    for key in &ids {
        if let Some(r) = read(&store, key, now)? {
            last = last.max(r.last_checked);
            if let Some(reason) = &r.hold_reason {
                held += 1;
                *reasons.entry(reason.clone()).or_default() += 1;
            }
            for t in r.turns {
                if eligible(&t) {
                    complete += 1;
                } else if t.terminal == Terminal::Cancelled {
                    cancelled += 1;
                } else if let Some(reason) = &t.hold_reason {
                    held += 1;
                    *reasons.entry(reason.clone()).or_default() += 1;
                } else {
                    partial += 1;
                }
            }
        }
    }
    Ok(
        json!({"capture_gap":dir.join("capture-gap.json").exists(),"hold_reasons":reasons,"message_complete":complete,"held":held,"partial":partial,"cancelled":cancelled,"last_callback":last,"counts_capped":ids.len()==100,"missed_initial_sessions":"undetectable","automatic_publication":false,"antigravity_terminal_attribution":"unverified"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn enrollment(root: &Path) -> Enrollment {
        Enrollment {
            client: Client::Codex,
            version: "0.162.0-alpha.2".into(),
            surface: "cli".into(),
            device_id: "d".into(),
            canonical_workspace: root.into(),
            service: "app".into(),
            policy_hash: "p".into(),
            enabled: true,
        }
    }
    #[tokio::test]
    async fn state_lock_serializes_independent_handles() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let first = lock(&root).await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(40), lock(&root))
                .await
                .is_err()
        );
        drop(first);
        assert!(lock(&root).await.is_ok());
    }
    #[test]
    fn clear_one_client_preserves_other_clients_and_cursor_drafts() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let e = enrollment(&root);
        let mut store = wk_local::LocalStore::open(&root.join("db"), [4; 32], 1000000).unwrap();
        let c = new_record(&e, 10).unwrap();
        let mut a = c.clone();
        a.enrollment.client = Client::Antigravity;
        for (id, r) in [("native2:c", c), ("native2:a", a)] {
            store.update(id, 10, 100, |_| encode(&r)).unwrap();
        }
        for (id, v) in [
            ("draft:c", json!({"native_adapter":"codex"})),
            ("draft:a", json!({"native_adapter":"antigravity"})),
            ("draft:cursor", json!({"native_turn_index":0})),
        ] {
            store
                .update(id, 10, 100, |_| Ok(v.to_string().into_bytes()))
                .unwrap();
        }
        clear_store(&mut store, 11, Some(Client::Codex)).unwrap();
        assert!(store.read_record("native2:c", 11).unwrap().is_none());
        assert!(store.read_record("draft:c", 11).unwrap().is_none());
        for key in ["native2:a", "draft:a", "draft:cursor"] {
            assert!(store.read_record(key, 11).unwrap().is_some(), "{key}");
        }
    }
    #[test]
    fn unsupported_context_holds_later_turns_in_the_same_session() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let e = enrollment(&root);
        let mut store = wk_local::LocalStore::open(&root.join("db"), [9; 32], 1000000).unwrap();
        let p = json!({"session_id":"s","turn_id":"one","cwd":root,"hook_event_name":"UserPromptSubmit","prompt":"hi"});
        ingest_codex(&mut store, "native2:session", &e, &p, 10).unwrap();
        let mut tool = p.clone();
        tool["hook_event_name"] = json!("PreToolUse");
        tool["tool_name"] = json!("Bash");
        ingest_codex(&mut store, "native2:session", &e, &tool, 11).unwrap();
        let held = read(&store, "native2:session", 11).unwrap().unwrap();
        assert!(
            held.hold_reason.is_some(),
            "unsupported context must hold the session, not just the turn"
        );
        let mut next = p;
        next["turn_id"] = json!("two");
        ingest_codex(&mut store, "native2:session", &e, &next, 12).unwrap();
        assert!(read(&store, "native2:session", 12)
            .unwrap()
            .unwrap()
            .turns
            .iter()
            .all(|t| !eligible(t)));
    }
    #[test]
    fn sequential_safe_turns_remain_separate_in_one_record() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let e = enrollment(&root);
        let mut store = wk_local::LocalStore::open(&root.join("db"), [9; 32], 1000000).unwrap();
        for turn in ["one", "two"] {
            let p = json!({"session_id":"s","turn_id":turn,"cwd":root,"hook_event_name":"UserPromptSubmit","prompt":"hi"});
            ingest_codex(&mut store, "native2:session", &e, &p, 10).unwrap();
            let stop = json!({"session_id":"s","turn_id":turn,"cwd":root,"hook_event_name":"Stop","last_assistant_message":"ok","stop_hook_active":false});
            ingest_codex(&mut store, "native2:session", &e, &stop, 11).unwrap();
        }
        let r = read(&store, "native2:session", 11).unwrap().unwrap();
        assert_eq!(r.turns.len(), 2);
        assert!(r.turns.iter().all(eligible));
    }
    #[test]
    fn restart_replay_cannot_resurrect_cancelled_turn_or_extend_expiry() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let path = root.join("db");
        let e = enrollment(&root);
        let p = json!({"session_id":"s","turn_id":"t","cwd":root,"hook_event_name":"UserPromptSubmit","prompt":"hi"});
        let mut store = wk_local::LocalStore::open(&path, [7; 32], 1000000).unwrap();
        ingest_codex(&mut store, "native2:test", &e, &p, 10).unwrap();
        let mut interrupt = p.clone();
        interrupt["hook_event_name"] = json!("Interrupt");
        ingest_codex(&mut store, "native2:test", &e, &interrupt, 11).unwrap();
        drop(store);
        let mut store = wk_local::LocalStore::open(&path, [7; 32], 1000000).unwrap();
        let stop = json!({"session_id":"s","turn_id":"t","cwd":root,"hook_event_name":"Stop","last_assistant_message":"ok","stop_hook_active":false});
        ingest_codex(&mut store, "native2:test", &e, &stop, 12).unwrap();
        let r = read(&store, "native2:test", 12).unwrap().unwrap();
        assert_eq!(r.expires_at, 86410);
        assert!(!eligible(&r.turns[0]));
        assert!(read(&store, "native2:test", 86410).unwrap().is_none());
    }
    #[test]
    fn conflict_discards_retained_content_instead_of_leaving_previous_complete_turn() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let e = enrollment(&root);
        let mut store = wk_local::LocalStore::open(&root.join("db"), [8; 32], 1000000).unwrap();
        let p = json!({"session_id":"s","turn_id":"t","cwd":root,"hook_event_name":"UserPromptSubmit","prompt":"CANARY"});
        ingest_codex(&mut store, "native2:test", &e, &p, 10).unwrap();
        let mut other = e;
        other.policy_hash = "new".into();
        ingest_codex(&mut store, "native2:test", &other, &p, 11).unwrap();
        let r = read(&store, "native2:test", 11).unwrap().unwrap();
        assert!(r.hold_reason.is_some());
        assert!(r.turns.is_empty());
    }
}
