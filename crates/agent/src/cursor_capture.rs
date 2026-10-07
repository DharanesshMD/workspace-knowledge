//! One encrypted record owns source binding, checkpoint and visible turns.
use crate::{
    cursor_source::{self, SourceBinding},
    daemon, network, state,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use wk_adapters::cursor_session::{parse_transcript, CursorTurn, MessageState, ScopeState};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSession {
    pub policy_hash: String,
    pub expires_at: i64,
    pub source: Option<SourceBinding>,
    pub consumed: usize,
    pub prefix_digest: String,
    pub turns: Vec<CursorTurn>,
    pub held: Option<String>,
    pub last_checked: i64,
    pub observations: Vec<String>,
}
impl NativeSession {
    fn new(policy_hash: String, now: i64) -> Self {
        Self {
            policy_hash,
            expires_at: now + 86400,
            source: None,
            consumed: 0,
            prefix_digest: String::new(),
            turns: vec![],
            held: None,
            last_checked: 0,
            observations: vec![],
        }
    }
    fn hold(&mut self, reason: &str) {
        if self.held.is_none() {
            self.held = Some(reason.into());
        }
        self.source = None;
        self.turns.clear();
        self.observations.clear();
    }
    fn encode(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|_| "invalid_native_session".into())
    }
    fn ingest(
        &mut self,
        bytes: &[u8],
        b: &wk_scope::enrollment::Binding,
        service: &str,
        now: i64,
    ) -> Result<(), String> {
        if self.held.is_some() {
            return Ok(());
        }
        self.last_checked = now;
        if now >= self.expires_at {
            self.hold("source_expired");
            return Ok(());
        }
        if self.policy_hash != b.policy_hash {
            self.hold("policy_changed");
            return Ok(());
        }
        if bytes.len() < self.consumed
            || (self.consumed > 0
                && format!("{:x}", Sha256::digest(&bytes[..self.consumed])) != self.prefix_digest)
        {
            self.hold("source_prefix_changed");
            return Ok(());
        }
        let snapshot = match parse_transcript(bytes, b, service) {
            Ok(v) => v,
            Err(_) => {
                self.hold("source_parse_failed");
                return Ok(());
            }
        };
        if let Some(reason) = snapshot.held_reason {
            self.hold(&reason);
            return Ok(());
        }
        if snapshot
            .turns
            .iter()
            .any(|t| t.scope != ScopeState::Verified)
        {
            self.hold("scope_unverified");
            return Ok(());
        }
        if snapshot
            .turns
            .iter()
            .any(|t| wk_summarize::sensitive(&t.prompt) || wk_summarize::sensitive(&t.response))
        {
            self.hold("sensitive_source");
            return Ok(());
        }
        self.consumed = snapshot.consumed;
        self.prefix_digest = snapshot.prefix_digest;
        self.turns = snapshot.turns;
        if self.encode()?.len() > 768 * 1024 {
            self.hold("retained_limit");
        }
        Ok(())
    }
}
fn id(c: &state::Config, conversation: &str) -> String {
    format!(
        "native:{:x}",
        Sha256::digest(format!("{}\0{}\0{}", c.root.display(), c.service, conversation).as_bytes())
    )
}
pub fn read(
    store: &wk_local::LocalStore,
    id: &str,
    now: i64,
) -> Result<Option<NativeSession>, String> {
    store
        .read_record(id, now)?
        .map(|v| serde_json::from_slice(&v).map_err(|_| "invalid_native_session".into()))
        .transpose()
}
pub async fn receive(dir: &Path, p: &Value) -> Result<Value, String> {
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let consent = cursor_source::load(dir)?;
    let conversation = cursor_source::validate_hook(&c, &consent, p)?;
    let key = id(&c, &conversation);
    let now = daemon::now()?;
    let mut store = state::store(dir)?;
    let token = state::token(dir, "server-token")?;
    if network::policy(&c, &b, &token).await.is_err() {
        store.update(&key, now, 86400, |old| {
            let mut s = old
                .map(serde_json::from_slice::<NativeSession>)
                .transpose()
                .map_err(|_| "invalid_native_session")?
                .unwrap_or_else(|| NativeSession::new(b.policy_hash.clone(), now));
            s.hold("membership_unavailable");
            s.encode()
        })?;
        return Err("membership_unavailable".into());
    }
    let old = read(&store, &key, now)?;
    if old.as_ref().is_some_and(|s| s.held.is_some()) {
        return Ok(json!({"held":true}));
    }
    // Binding is metadata-only and only follows successful scope/permission checks.
    let source = cursor_source::bind(&c, &consent, p, now);
    let event = p["hook_event_name"].as_str().unwrap_or("");
    let observation = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                event,
                &p["generation_id"],
                &p["status"],
                &p["transcript_path"]
            ))
            .map_err(|_| "invalid_hook")?
        )
    );
    store.update(&key, now, 86400, |old| {
        let mut s = old
            .map(serde_json::from_slice::<NativeSession>)
            .transpose()
            .map_err(|_| "invalid_native_session")?
            .unwrap_or_else(|| NativeSession::new(b.policy_hash.clone(), now));
        if s.held.is_some() {
            return s.encode();
        }
        if s.policy_hash != b.policy_hash {
            s.hold("policy_changed");
            return s.encode();
        }
        match source {
            Ok(Some(mut source)) => {
                source.expires_at = s.expires_at;
                if let Some(previous) = &s.source {
                    if previous.path != source.path
                        || previous.device != source.device
                        || previous.inode != source.inode
                    {
                        s.hold("source_replaced");
                    }
                } else {
                    s.source = Some(source);
                }
            }
            Ok(None) => {}
            Err(_) => s.hold("source_binding_failed"),
        }
        if !matches!(
            event,
            "sessionStart"
                | "beforeSubmitPrompt"
                | "afterAgentResponse"
                | "stop"
                | "beforeReadFile"
                | "afterFileEdit"
                | "postToolUse"
        ) {
            s.hold("unknown_hook");
        }
        if s.held.is_none() && !s.observations.contains(&observation) {
            s.observations.push(observation);
        }
        if s.observations.len() > 100 {
            s.hold("hook_limit");
        }
        s.encode()
    })?;
    Ok(json!({"accepted":true,"published":false}))
}
pub async fn reconcile_one(dir: &Path, key: &str) -> Result<(), String> {
    let mut store = state::store(dir)?;
    let now = daemon::now()?;
    let Some(mut session) = read(&store, key, now)? else {
        return Err("source_expired".into());
    };
    if session.held.is_some() {
        return Err("source_held".into());
    }
    let result = async {
        if dir.join("capture-gap.json").exists() {
            return Err("capture_gap".to_string());
        }
        let c = state::load(dir)?;
        let b = state::binding(&c)?;
        let consent = cursor_source::load(dir)?;
        if consent.policy_hash != session.policy_hash || b.policy_hash != session.policy_hash {
            return Err("policy_changed".into());
        }
        let token = state::token(dir, "server-token")?;
        network::policy(&c, &b, &token)
            .await
            .map_err(|_| "membership_unavailable")?;
        let Some(source) = &session.source else {
            return Ok(());
        };
        let bytes = cursor_source::read_bound(source, daemon::now()?)?;
        session.ingest(&bytes, &b, &c.service, daemon::now()?)?;
        if session.held.is_some() {
            return Ok(());
        }
        let source = session.source.as_ref().ok_or("source_missing")?;
        let check = cursor_source::read_bound(source, daemon::now()?)?;
        if check.len() < session.consumed
            || format!("{:x}", Sha256::digest(&check[..session.consumed])) != session.prefix_digest
        {
            return Err("source_changed_during_read".into());
        }
        // Do not commit if scope/consent/remote authorization changed while reading.
        let latest = state::binding(&c)?;
        let consent = cursor_source::load(dir)?;
        if latest.policy_hash != session.policy_hash || consent.policy_hash != session.policy_hash {
            return Err("policy_changed".into());
        }
        network::policy(&c, &latest, &token)
            .await
            .map_err(|_| "membership_unavailable")?;
        Ok::<(), String>(())
    }
    .await;
    if let Err(reason) = &result {
        session.hold(match reason.as_str() {
            "membership_unavailable" => "membership_unavailable",
            "source_expired" => "source_expired",
            _ => "source_unavailable_or_changed",
        });
    }
    session.last_checked = daemon::now()?;
    store.update(key, daemon::now()?, 86400, |old| {
        let mut latest: NativeSession = serde_json::from_slice(old.ok_or("source_expired")?)
            .map_err(|_| "invalid_native_session")?;
        if latest.held.is_some() {
            return latest.encode();
        }
        if latest.source != session.source && session.held.is_none() {
            latest.hold("concurrent_source_change");
            return latest.encode();
        }
        // Concurrent receipts may add observations; preserve them with the checkpoint.
        session.observations = latest.observations;
        if session.held.is_some() {
            session.observations.clear();
        }
        session.encode()
    })?;
    result
}
pub async fn reconcile(dir: &Path) -> Result<Value, String> {
    let mut store = state::store(dir)?;
    store.purge_expired(daemon::now()?)?;
    let ids = store.next_ids("native-recovery", "native:", daemon::now()?, 10)?;
    let mut checked = 0;
    let mut held = 0;
    for id in ids {
        if reconcile_one(dir, &id).await.is_ok() {
            checked += 1;
        } else {
            held += 1;
        }
    }
    Ok(json!({"checked":checked,"held":held}))
}
pub fn status(dir: &Path) -> Result<Value, String> {
    let mut store = state::store(dir)?;
    let now = daemon::now()?;
    store.purge_expired(now)?;
    let ids = store.pending_ids("native:", "", now, 100)?;
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let mut reasons = std::collections::BTreeMap::<String, usize>::new();
    let mut turns = 0;
    let mut last = 0;
    for key in &ids {
        if let Some(s) = read(&store, key, now)? {
            if let Some(reason) = &s.held {
                *reasons.entry(reason.clone()).or_default() += 1;
            }
            let label = if s.held.is_some() {
                "held"
            } else if s.source.is_none() {
                "waiting_for_binding"
            } else if now - s.last_checked > 45 {
                "stale"
            } else {
                "watching"
            };
            *counts.entry(label.into()).or_default() += 1;
            turns += s
                .turns
                .iter()
                .filter(|t| t.messages == MessageState::Complete)
                .count();
            last = last.max(s.last_checked);
        }
    }
    Ok(
        json!({"enrolled":dir.join("cursor-consent.json").exists(),"version":cursor_source::VERSION,"surface":"agents-local","sessions":counts,"hold_reasons":reasons,"counts_capped":ids.len()==100,"recovered_complete_turns":turns,"last_checked":last,"missed_initial_sessions":"undetectable","automatic_publication":false}),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    fn setup() -> (
        tempfile::TempDir,
        wk_scope::enrollment::Binding,
        NativeSession,
    ) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("frontend")).unwrap();
        let b = wk_scope::enrollment::Binding::approve(d.path(), "demo", vec!["frontend".into()])
            .unwrap();
        let s = NativeSession::new(b.policy_hash.clone(), 100);
        (d, b, s)
    }
    fn transcript() -> Vec<u8> {
        [json!({"role":"user","message":{"content":[{"type":"text","text":"Bound retries"}]}}),json!({"role":"assistant","message":{"content":[{"type":"text","text":"Use idempotency"}]}}),json!({"type":"turn_ended","status":"success"})].iter().map(|v|v.to_string()+"\n").collect::<String>().into_bytes()
    }
    #[test]
    fn duplicate_reconciliation_and_restart_are_atomic() {
        let (d, b, mut s) = setup();
        let bytes = transcript();
        s.ingest(&bytes, &b, "frontend", 101).unwrap();
        let db = d.path().join("capture.db");
        let mut store = wk_local::LocalStore::open(&db, [1; 32], 1_000_000).unwrap();
        store
            .update("native:test", 100, 86400, |_| s.encode())
            .unwrap();
        drop(store);
        let mut store = wk_local::LocalStore::open(&db, [1; 32], 1_000_000).unwrap();
        let mut restored: NativeSession =
            serde_json::from_slice(&store.read_record("native:test", 102).unwrap().unwrap())
                .unwrap();
        restored.ingest(&bytes, &b, "frontend", 102).unwrap();
        store
            .update("native:test", 102, 86400, |_| restored.encode())
            .unwrap();
        assert_eq!(restored.turns.len(), 1);
        assert_eq!(restored.turns[0].digest, s.turns[0].digest);
        assert_eq!(restored.expires_at, s.expires_at);
        assert!(!std::fs::read(&db)
            .unwrap()
            .windows(13)
            .any(|w| w == b"Bound retries"));
    }
    #[test]
    fn prefix_mutation_and_scope_loss_hold_and_erase() {
        let (_d, b, mut s) = setup();
        let bytes = transcript();
        s.ingest(&bytes, &b, "frontend", 101).unwrap();
        let changed = String::from_utf8(bytes).unwrap().replace("Bound", "Round");
        s.ingest(changed.as_bytes(), &b, "frontend", 102).unwrap();
        assert!(s.held.is_some());
        assert!(s.turns.is_empty());
        s.ingest(&transcript(), &b, "frontend", 103).unwrap();
        assert!(s.turns.is_empty());
        let mut fresh = NativeSession::new(b.policy_hash.clone(), 100);
        let data = [transcript(), b"{\"type\":\"compaction\"}\n".to_vec()].concat();
        fresh.ingest(&data, &b, "frontend", 101).unwrap();
        assert!(fresh.held.is_some());
        assert!(fresh.turns.is_empty());
    }
    #[test]
    fn partial_growth_expiry_policy_loss_and_sensitive_text_do_not_escape() {
        let (_d, b, mut s) = setup();
        let bytes = transcript();
        let cut = bytes.len() - 4;
        s.ingest(&bytes[..cut], &b, "frontend", 101).unwrap();
        assert_eq!(s.turns[0].messages, MessageState::Partial);
        s.ingest(&bytes, &b, "frontend", 102).unwrap();
        assert_eq!(s.turns[0].messages, MessageState::Complete);
        s.ingest(&bytes, &b, "frontend", 86500).unwrap();
        assert_eq!(s.held.as_deref(), Some("source_expired"));
        let mut s = NativeSession::new("wrong".into(), 100);
        s.ingest(&bytes, &b, "frontend", 101).unwrap();
        assert_eq!(s.held.as_deref(), Some("policy_changed"));
        let mut s = NativeSession::new(b.policy_hash.clone(), 100);
        s.ingest(
            String::from_utf8(bytes)
                .unwrap()
                .replace("Bound retries", "password = CANARY")
                .as_bytes(),
            &b,
            "frontend",
            101,
        )
        .unwrap();
        assert!(s.turns.is_empty());
        assert!(!String::from_utf8(s.encode().unwrap())
            .unwrap()
            .contains("CANARY"));
    }
}
