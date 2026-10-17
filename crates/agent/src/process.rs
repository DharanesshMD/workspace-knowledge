use crate::{daemon, network, state};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
pub async fn run(dir: &Path) -> Result<Value, String> {
    state::store(dir)?.purge_expired(daemon::now()?)?;
    crate::cursor_capture::reconcile(dir).await?;
    crate::native_capture::reconcile(dir).await?;
    if dir.join("capture-gap.json").exists() {
        return Err("capture gap requires reset into a clean session".into());
    }
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let token = state::token(dir, "server-token")?;
    let policy = network::policy(&c, &b, &token).await?;
    let mut store = state::store(dir)?;
    let ids = store.next_session_ids(daemon::now()?)?;
    let mut drafts =
        native_run(dir, &c, &b, &policy).await? + crate::native_process::run(dir).await?;
    let mut held = 0;
    for id in ids {
        let Some(bytes) = store.read_record(&id, daemon::now()?)? else {
            continue;
        };
        let session: Value =
            serde_json::from_slice(&bytes).map_err(|_| "invalid private session")?;
        if session["held"] == true || session["policy_hash"] != b.policy_hash {
            held += 1;
            continue;
        }
        let Some(events) = session["events"].as_array() else {
            continue;
        };
        if !events.iter().any(|e| e["kind"] == "assistant_response") {
            continue;
        }
        // Hook coverage remains partial. A managed chat writes a turn_end under
        // a fresh explicit turn identity and supplies both visible messages.
        let (made, failed) = journal_turns(dir, &c, &b, &policy, &mut store, &id, &session).await?;
        drafts += made;
        held += failed;
    }
    // A memory import problem never blocks session drafts or owner sync.
    let memory = crate::memory_import::draft_all(dir, &c, &b, &policy, &mut store).await;
    if let Ok((made, failed)) = memory {
        drafts += made;
        held += failed;
    }

    let owner_sync = crate::personal_sync::run(dir)
        .await
        .unwrap_or_else(|_| json!({"enabled":false,"uploaded":0,"held":0,"status":"unavailable"}));
    Ok(
        json!({"private_drafts_created":drafts,"held":held,"memory_drafting":if memory.is_ok() {"ok"} else {"failed"},"published":false,"owner_sync":owner_sync}),
    )
}
// Re-read local permission and destination around every slow inference attempt.
// An acknowledged pause stops subsequent requests and discards an in-flight result.
pub(crate) fn processing_current(
    dir: &Path,
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
) -> Result<(), String> {
    if state::load(dir)? != *c || state::binding(c)?.policy_hash != b.policy_hash {
        return Err("processing approval or configuration changed".into());
    }
    Ok(())
}
pub(crate) async fn journal_turns(
    dir: &Path,
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    policy: &Value,
    store: &mut wk_local::LocalStore,
    id: &str,
    session: &Value,
) -> Result<(usize, usize), String> {
    let events = session["events"]
        .as_array()
        .ok_or("invalid private events")?;
    let mut drafts = 0;
    let mut held = 0;
    let mut turns = std::collections::BTreeMap::<String, Vec<&Value>>::new();
    for event in events {
        let key = format!("{}:{}", event["agent_id"], event["turn_id"]);
        turns.entry(key).or_default().push(event);
    }
    for (turn_key, events) in &turns {
        if processing_current(dir, c, b).is_err() {
            break;
        }
        if !events.iter().any(|e| e["kind"] == "assistant_response") {
            continue;
        }
        let source = events
            .iter()
            .filter(|e| e["kind"] == "user_prompt" || e["kind"] == "assistant_response")
            .map(|e| {
                format!(
                    "{}: {}",
                    e["kind"].as_str().unwrap_or("message"),
                    e["text"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let hash = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    id,
                    events,
                    &b.policy_hash,
                    &policy["policy_revision"],
                    c.model.as_str(),
                    1
                ))
                .map_err(|_| "invalid private events")?
            )
        );
        let key = format!("draft:{hash}");
        if store.seen(&key)? {
            continue;
        }
        match wk_summarize::extract(&c.inference, &c.model, &source).await {
            Ok(extract) => {
                if processing_current(dir, c, b).is_err() {
                    break;
                }
                let complete = events.iter().any(|e| e["kind"] == "turn_end")
                    && events.iter().all(|e| e["agent_id"] == "managed-chat");
                let card = wk_summarize::card(
                    extract,
                    hash,
                    policy["policy_revision"].as_u64().ok_or("invalid policy")?,
                    vec![c.service.clone()],
                    complete,
                )?;
                let draft = json!({"card":card,"source_id":id,"source_digest":format!("{:x}",Sha256::digest(serde_json::to_vec(events).map_err(|_|"invalid private events")?)),"turn_key":turn_key,"policy_hash":b.policy_hash,"model":c.model,"prompt_version":1,"privacy_policy_version":1});
                let encoded = serde_json::to_vec(&draft).map_err(|_| "invalid draft")?;
                store.update(&key, daemon::now()?, 3600, |old| {
                    Ok(old.map_or(encoded.clone(), |bytes| bytes.to_vec()))
                })?;
                drafts += 1;
            }
            Err(_) => {
                held += 1;
            }
        }
    }
    Ok((drafts, held))
}

pub fn list(dir: &Path, after: &str) -> Result<Value, String> {
    if !after.is_empty() && (!after.starts_with("draft:") || after.len() != 70) {
        return Err("invalid draft cursor".into());
    }
    let s = state::store(dir)?;
    let ids = s.pending_ids("draft:", after, daemon::now()?, 100)?;
    let next_cursor = if ids.len() == 100 {
        ids.last().cloned()
    } else {
        None
    };
    let mut result = Vec::new();
    for id in ids {
        let Some(bytes) = s.read_record(&id, daemon::now()?)? else {
            continue;
        };
        let v: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid private draft")?;
        result.push(crate::memory_import::draft_item(&id, &v));
    }
    Ok(json!({"private_drafts":result,"next_cursor":next_cursor}))
}
pub async fn submit(dir: &Path, id: &str) -> Result<Value, String> {
    deliver(dir, id, false).await
}
pub(crate) async fn sync_owner(dir: &Path, id: &str) -> Result<Value, String> {
    deliver(dir, id, true).await
}
pub(crate) async fn post_draft(
    dir: &Path,
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    t: &str,
    v: &Value,
    owner_only: bool,
) -> Result<Value, String> {
    if owner_only {
        let revision = v["card"]["policy_revision"]
            .as_u64()
            .ok_or("invalid policy")?;
        if !crate::personal_sync::permits(dir, c, b, revision)? {
            return Err("owner synchronization disabled or approval changed".into());
        }
        return crate::personal_sync::send(c, &b.product, t, &v["card"]).await;
    }
    network::post(
        c,
        t,
        "/v1/cards",
        &json!({"product":b.product,"card":v["card"]}),
    )
    .await
}
async fn deliver(dir: &Path, id: &str, owner_only: bool) -> Result<Value, String> {
    if dir.join("capture-gap.json").exists() {
        return Err("capture gap requires clean-session reset".into());
    }
    if !id.starts_with("draft:") || id.len() != 70 {
        return Err("invalid draft ID".into());
    }
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let t = state::token(dir, "server-token")?;
    let p = network::policy(&c, &b, &t).await?;
    let mut s = state::store(dir)?;
    let bytes = s
        .read_record(id, daemon::now()?)?
        .ok_or("draft expired or unavailable")?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid private draft")?;
    if owner_only {
        crate::personal_sync::payload(&v["card"])?;
    }
    if v.get("memory").is_some() {
        crate::memory_import::check_submit(&mut s, id, &v, &b.policy_hash, &p["policy_revision"])?;
        let response = post_draft(dir, &c, &b, &t, &v, owner_only).await?;
        return crate::memory_import::finish_submit(dir, &mut s, id, &v, response, owner_only);
    }
    if v.get("native_adapter").is_some() {
        let _guard = crate::native_process::validate_draft(dir, &v).await?;
        if wk_summarize::sensitive(&v["card"].to_string()) {
            return Err("sensitive_summary".into());
        }
        let result = post_draft(dir, &c, &b, &t, &v, owner_only).await?;
        if !owner_only {
            s.ack(id)?;
        }
        return Ok(result);
    }
    if v.get("native_turn_index").is_some() {
        let key = v["source_id"].as_str().ok_or("missing_source")?;
        crate::cursor_capture::reconcile_one(dir, key).await?;
        let source =
            crate::cursor_capture::read(&s, key, daemon::now()?)?.ok_or("source_expired")?;
        let latest = state::binding(&c)?;
        let consent = crate::cursor_source::load(dir)?;
        let policy = network::policy(&c, &latest, &t).await?;
        if consent.policy_hash != latest.policy_hash
            || !native_matches(
                &v,
                &source.turns,
                source.held.is_some(),
                &latest.policy_hash,
                policy["policy_revision"].as_u64().ok_or("invalid_policy")?,
                source.expires_at,
                daemon::now()?,
            )
        {
            return Err("source_or_policy_changed_review_required".into());
        }
        if wk_summarize::sensitive(&v["card"].to_string()) {
            return Err("sensitive_summary".into());
        }
        let response = post_draft(dir, &c, &latest, &t, &v, owner_only).await?;
        if !owner_only {
            s.ack(id)?;
        }
        return Ok(response);
    }
    let source = s
        .read_record(
            v["source_id"].as_str().ok_or("missing source")?,
            daemon::now()?,
        )?
        .ok_or("source expired")?;
    let source: Value = serde_json::from_slice(&source).map_err(|_| "invalid source")?;
    if source["held"] == true {
        return Err("source session held".into());
    }
    let events: Vec<&Value> = source["events"]
        .as_array()
        .ok_or("invalid source")?
        .iter()
        .filter(|e| {
            format!("{}:{}", e["agent_id"], e["turn_id"]) == v["turn_key"].as_str().unwrap_or("")
        })
        .collect();
    if v["source_digest"]
        != format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&events).map_err(|_| "invalid source")?)
        )
        || v["policy_hash"] != b.policy_hash
        || v["card"]["policy_revision"] != p["policy_revision"]
    {
        return Err("source or policy changed; regenerate and review draft".into());
    }
    if wk_summarize::sensitive(&v["card"].to_string()) {
        return Err("draft requires sensitive-content review".into());
    }
    let response = post_draft(dir, &c, &b, &t, &v, owner_only).await?;
    if !owner_only {
        s.ack(id)?;
    }
    Ok(response)
}

fn native_eligible(turn: &wk_adapters::cursor_session::CursorTurn, held: bool) -> bool {
    !held
        && turn.messages == wk_adapters::cursor_session::MessageState::Complete
        && turn.scope == wk_adapters::cursor_session::ScopeState::Verified
}
#[allow(clippy::too_many_arguments)]
fn native_matches(
    draft: &Value,
    turns: &[wk_adapters::cursor_session::CursorTurn],
    held: bool,
    policy_hash: &str,
    revision: u64,
    expires: i64,
    now: i64,
) -> bool {
    expires > now
        && draft["policy_hash"] == policy_hash
        && draft["card"]["policy_revision"] == revision
        && turns.iter().any(|t| {
            native_eligible(t, held)
                && draft["native_turn_index"] == t.index
                && draft["source_digest"] == t.digest
        })
}
pub(crate) struct InferenceBudget {
    remaining: usize,
    deadline: tokio::time::Instant,
}
impl InferenceBudget {
    pub(crate) fn new(attempts: usize, duration: std::time::Duration) -> Self {
        Self {
            remaining: attempts,
            deadline: tokio::time::Instant::now() + duration,
        }
    }
    pub(crate) async fn attempt<F: std::future::Future>(&mut self, work: F) -> Option<F::Output> {
        if self.remaining == 0 || tokio::time::Instant::now() >= self.deadline {
            return None;
        }
        self.remaining -= 1;
        tokio::time::timeout_at(self.deadline, work).await.ok()
    }
}
async fn native_run(
    dir: &Path,
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    policy: &Value,
) -> Result<usize, String> {
    let mut store = state::store(dir)?;
    let ids = store.next_ids("native-inference", "native:", daemon::now()?, 10)?;
    let mut created = 0;
    let mut budget = InferenceBudget::new(10, std::time::Duration::from_secs(120));
    for id in ids {
        let Some(session) = crate::cursor_capture::read(&store, &id, daemon::now()?)? else {
            continue;
        };
        if session.policy_hash != b.policy_hash || session.source.is_none() {
            continue;
        }
        for turn in &session.turns {
            if processing_current(dir, c, b).is_err() {
                return Ok(created);
            }
            if !native_eligible(turn, session.held.is_some()) {
                continue;
            }
            let hash = format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&(
                        &id,
                        turn.index,
                        &turn.digest,
                        &b.policy_hash,
                        &policy["policy_revision"],
                        &c.model,
                        "native-v1"
                    ))
                    .map_err(|_| "invalid_native_turn")?
                )
            );
            let key = format!("draft:{hash}");
            if store.seen(&key)? {
                continue;
            }
            // Revalidate before EVERY attempt, including after failed inference.
            if crate::cursor_capture::reconcile_one(dir, &id)
                .await
                .is_err()
            {
                break;
            }
            let Some(current) = crate::cursor_capture::read(&store, &id, daemon::now()?)? else {
                break;
            };
            if current.held.is_some()
                || current.policy_hash != b.policy_hash
                || current.source.is_none()
                || !current.turns.iter().any(|t| {
                    native_eligible(t, false) && t.index == turn.index && t.digest == turn.digest
                })
            {
                break;
            }
            let source = format!("user: {}\nassistant: {}", turn.prompt, turn.response);
            let Some(result) = budget
                .attempt(wk_summarize::extract(&c.inference, &c.model, &source))
                .await
            else {
                return Ok(created);
            };
            let Ok(extract) = result else {
                continue;
            };
            if processing_current(dir, c, b).is_err() {
                return Ok(created);
            }
            let mut card = wk_summarize::card(
                extract,
                hash,
                policy["policy_revision"].as_u64().ok_or("invalid_policy")?,
                vec![c.service.clone()],
                true,
            )?;
            let files = turn
                .files_read
                .iter()
                .chain(&turn.files_changed)
                .collect::<std::collections::BTreeSet<_>>();
            for file in files {
                card.evidence.push(wk_contracts::Evidence {
                    kind: wk_contracts::EvidenceKind::FileObservation,
                    reference: file.clone(),
                });
            }
            card.validate()?;
            if wk_summarize::sensitive(&serde_json::to_string(&card).map_err(|_| "invalid_card")?) {
                continue;
            }
            let draft = json!({"card":card,"source_id":id,"source_digest":turn.digest,"native_turn_index":turn.index,"source_boundary":"Cursor 3.23.23 local visible text","policy_hash":b.policy_hash,"model":c.model,"prompt_version":1,"privacy_policy_version":1});
            // Inference is slow. Reconcile under current permission before retaining its result.
            if crate::cursor_capture::reconcile_one(dir, &id)
                .await
                .is_err()
            {
                break;
            }
            let current = crate::cursor_capture::read(&store, &id, daemon::now()?)?
                .ok_or("source_expired")?;
            let latest = state::binding(c)?;
            let token = state::token(dir, "server-token")?;
            let p = network::policy(c, &latest, &token).await?;
            if !native_matches(
                &draft,
                &current.turns,
                current.held.is_some(),
                &latest.policy_hash,
                p["policy_revision"].as_u64().ok_or("invalid_policy")?,
                current.expires_at,
                daemon::now()?,
            ) {
                continue;
            }
            let ttl = (current.expires_at - daemon::now()?).min(3600);
            if ttl <= 0 {
                continue;
            }
            let encoded = serde_json::to_vec(&draft).map_err(|_| "invalid_draft")?;
            store.update(&key, daemon::now()?, ttl, |old| {
                Ok(old.map_or(encoded.clone(), |b| b.to_vec()))
            })?;
            created += 1;
            if created >= 10 {
                return Ok(created);
            }
        }
    }
    Ok(created)
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use wk_adapters::cursor_session::{CursorTurn, MessageState, ScopeState};
    #[tokio::test]
    async fn failed_inference_exhausts_attempts_and_stalled_work_times_out() {
        let mut budget = InferenceBudget::new(2, std::time::Duration::from_millis(20));
        assert!(budget
            .attempt(async { Err::<(), _>("failed") })
            .await
            .unwrap()
            .is_err());
        assert!(budget
            .attempt(async { Err::<(), _>("failed") })
            .await
            .unwrap()
            .is_err());
        assert!(budget.attempt(async { Ok::<_, &str>(()) }).await.is_none());
        let mut budget = InferenceBudget::new(10, std::time::Duration::from_millis(20));
        assert!(budget.attempt(std::future::pending::<()>()).await.is_none());
        assert!(budget.attempt(async {}).await.is_none());
    }
    fn turn() -> CursorTurn {
        CursorTurn {
            index: 0,
            prompt: "Retries".into(),
            response: "Use idempotency".into(),
            files_read: vec![],
            files_changed: vec![],
            messages: MessageState::Complete,
            scope: ScopeState::Verified,
            digest: "digest".into(),
        }
    }
    #[test]
    fn native_completeness_requires_verified_scope_and_no_hold() {
        let mut t = turn();
        assert!(native_eligible(&t, false));
        assert!(!native_eligible(&t, true));
        for scope in [ScopeState::Unknown, ScopeState::Violated] {
            t.scope = scope;
            assert!(!native_eligible(&t, false));
        }
        t.scope = ScopeState::Verified;
        t.messages = MessageState::Partial;
        assert!(!native_eligible(&t, false));
    }
    #[test]
    fn native_review_rejects_mutation_expiry_revocation_and_accepts_later_safe_turns() {
        let t = turn();
        let draft = json!({"native_turn_index":0,"source_digest":"digest","policy_hash":"p","card":{"policy_revision":1}});
        assert!(native_matches(
            &draft,
            std::slice::from_ref(&t),
            false,
            "p",
            1,
            200,
            100
        ));
        let mut later = t.clone();
        later.index = 1;
        assert!(native_matches(
            &draft,
            &[t.clone(), later],
            false,
            "p",
            1,
            200,
            100
        ));
        let mut changed = t.clone();
        changed.digest = "edited".into();
        assert!(!native_matches(&draft, &[changed], false, "p", 1, 200, 100));
        assert!(!native_matches(
            &draft,
            std::slice::from_ref(&t),
            false,
            "p",
            1,
            100,
            100
        ));
        assert!(!native_matches(
            &draft,
            std::slice::from_ref(&t),
            false,
            "p",
            2,
            200,
            100
        ));
        assert!(!native_matches(&draft, &[t], true, "p", 1, 200, 100));
    }
}

#[cfg(test)]
mod pause_regression {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[tokio::test]
    async fn acknowledged_pause_stops_second_turn_and_discards_inflight_result() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(root.join("backend")).unwrap();
        let dir = state::private_dir(&temp.path().join("state"), true).unwrap();
        let b =
            wk_scope::enrollment::Binding::approve(&root, "demo", vec!["backend".into()]).unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app=axum::Router::new().route("/inference",axum::routing::post({let entered=entered.clone();let release=release.clone();let calls=calls.clone();move ||{let entered=entered.clone();let release=release.clone();let calls=calls.clone();async move {if calls.fetch_add(1,Ordering::SeqCst)==0 {entered.notify_one();release.notified().await;}let content=json!({"title":"Limiter","intent":"Bound request work","rationale":"Middleware","decisions":[],"constraints":[],"open_questions":[],"sensitive":false});axum::Json(json!({"choices":[{"message":{"content":content.to_string()}}]}))}}}));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let c = state::Config {
            root: b.root.clone(),
            binding_file: dir.join("binding.json"),
            service: "backend".into(),
            server: "http://127.0.0.1:4318".into(),
            model: "synthetic-local".into(),
            inference: format!("http://{address}/inference"),
            port: 4319,
            account: "workspace".into(),
            device_id: uuid::Uuid::new_v4().to_string(),
            repository: None,
            device: Some(crate::device::Connection {
                state_dir: dir.clone(),
            }),
        };
        state::save(&c.binding_file, &b).unwrap();
        state::save(&dir.join("agent.json"), &c).unwrap();
        crate::device::save_receipt(&c, &b).unwrap();
        crate::device::apply_remote(
            &c,
            &b,
            crate::device::Remote {
                revision: 1,
                settings: Default::default(),
            },
        )
        .await
        .unwrap();
        let session = json!({"events":[{"kind":"user_prompt","agent_id":"managed-chat","turn_id":"one","text":"Bound request work"},{"kind":"assistant_response","agent_id":"managed-chat","turn_id":"one","text":"Use middleware"},{"kind":"turn_end","agent_id":"managed-chat","turn_id":"one"},{"kind":"user_prompt","agent_id":"managed-chat","turn_id":"two","text":"Bound more work"},{"kind":"assistant_response","agent_id":"managed-chat","turn_id":"two","text":"Review middleware"},{"kind":"turn_end","agent_id":"managed-chat","turn_id":"two"}]});
        let work = tokio::spawn({
            let dir = dir.clone();
            let c = c.clone();
            async move {
                let b = state::base_binding(&c).unwrap();
                let mut store =
                    wk_local::LocalStore::open(&dir.join("journal.db"), [7; 32], 1000000).unwrap();
                journal_turns(
                    &dir,
                    &c,
                    &b,
                    &json!({"policy_revision":1}),
                    &mut store,
                    "managed-test",
                    &session,
                )
                .await
                .unwrap()
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        crate::device::apply_remote(
            &c,
            &b,
            crate::device::Remote {
                revision: 2,
                settings: wk_contracts::device::DeviceSettings {
                    paused: true,
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
        assert!(crate::device::remote(&c).unwrap().settings.paused);
        release.notify_one();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), work)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "no raw source from the second turn reaches inference after pause acknowledgment"
        );
        assert_eq!(result.0, 0, "inflight inference result is not retained");
        let store = wk_local::LocalStore::open(&dir.join("journal.db"), [7; 32], 1000000).unwrap();
        assert!(store
            .pending_ids("draft:", "", daemon::now().unwrap(), 100)
            .unwrap()
            .is_empty());
        server.abort();
    }
}
