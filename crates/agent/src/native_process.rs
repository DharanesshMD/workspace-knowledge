use crate::{
    daemon,
    native_capture::{self, Record},
    native_enrollment, state,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use wk_adapters::native_evidence::eligible;

fn matches(draft: &Value, r: &Record, now: i64, revision: u64) -> bool {
    r.expires_at > now
        && r.hold_reason.is_none()
        && r.enrollment.enabled
        && draft["policy_hash"] == r.enrollment.policy_hash
        && draft["policy_revision"] == revision
        && draft["card"]["policy_revision"] == revision
        && r.turns.iter().any(|t| {
            eligible(t)
                && draft["native_adapter"] == native_enrollment::name(t.client)
                && draft["turn_id"] == t.turn
                && draft["source_digest"] == t.digest
                && draft["parser_version"] == t.parser_version
                && draft["client_version"] == t.client_version
        })
}
fn check_current(
    dir: &Path,
    store: &wk_local::LocalStore,
    draft: &Value,
    now: i64,
    revision: u64,
) -> Result<(), String> {
    if dir.join("capture-gap.json").exists() {
        return Err("capture_gap".into());
    }
    let key = draft["source_id"].as_str().ok_or("source_missing")?;
    let r = native_capture::read(store, key, now)?.ok_or("source_expired")?;
    let current = native_enrollment::load_current(dir, r.enrollment.client)?;
    if current.policy_hash != r.enrollment.policy_hash
        || current.canonical_workspace != r.enrollment.canonical_workspace
        || current.device_id != r.enrollment.device_id
        || !matches(draft, &r, now, revision)
    {
        return Err("source_or_policy_changed_review_required".into());
    }
    Ok(())
}
pub async fn validate_draft(dir: &Path, draft: &Value) -> Result<std::fs::File, String> {
    if dir.join("capture-gap.json").exists() {
        return Err("capture_gap".into());
    }
    let key = draft["source_id"].as_str().ok_or("source_missing")?;
    if !key.starts_with("native2:") {
        return Err("source_invalid".into());
    }
    native_capture::revalidate(dir, key).await?;
    let p = native_capture::policy(dir).await?;
    let guard = native_capture::lock(dir).await?;
    check_current(
        dir,
        &state::store(dir)?,
        draft,
        daemon::now()?,
        p["policy_revision"].as_u64().ok_or("invalid_policy")?,
    )?;
    Ok(guard)
}
pub async fn run(dir: &Path) -> Result<usize, String> {
    if dir.join("capture-gap.json").exists() {
        return Ok(0);
    }
    let c = state::load(dir)?;
    let binding = state::binding(&c)?;
    let mut store = state::store(dir)?;
    let ids = store.next_ids("native2-inference", "native2:", daemon::now()?, 10)?;
    let mut created = 0;
    let mut budget = crate::process::InferenceBudget::new(10, std::time::Duration::from_secs(120));
    for key in ids {
        let Ok(r) = native_capture::revalidate(dir, &key).await else {
            continue;
        };
        for turn in r.turns.iter().filter(|t| eligible(t)) {
            if crate::process::processing_current(dir, &c, &binding).is_err() {
                return Ok(created);
            }
            let policy = native_capture::policy(dir).await?;
            let revision = policy["policy_revision"].as_u64().ok_or("invalid_policy")?;
            let hash = format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&(
                        &key,
                        &turn.turn,
                        &turn.digest,
                        turn.parser_version,
                        &turn.client_version,
                        &c.model,
                        &r.enrollment.policy_hash,
                        revision,
                        1
                    ))
                    .map_err(|_| "invalid_draft")?
                )
            );
            let id = format!("draft:{hash}");
            if store.seen(&id)? {
                continue;
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
            if crate::process::processing_current(dir, &c, &binding).is_err() {
                return Ok(created);
            }
            let card = wk_summarize::card(extract, hash, revision, vec![c.service.clone()], true)?;
            card.validate()?;
            if wk_summarize::sensitive(&serde_json::to_string(&card).map_err(|_| "invalid_card")?) {
                continue;
            }
            let draft = json!({"card":card,"native_adapter":native_enrollment::name(turn.client),"source_id":key,"source_digest":turn.digest,"turn_id":turn.turn,"parser_version":turn.parser_version,"client_version":turn.client_version,"policy_hash":r.enrollment.policy_hash,"policy_revision":revision,"model":c.model,"prompt_version":1});
            let Ok(_guard) = validate_draft(dir, &draft).await else {
                continue;
            };
            let now = daemon::now()?;
            let ttl = (r.expires_at - now).min(3600);
            if ttl <= 0 {
                continue;
            }
            store.update(&id, now, ttl, |old| {
                Ok(old.map_or_else(|| serde_json::to_vec(&draft).unwrap(), |v| v.to_vec()))
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
mod tests {
    use super::*;
    use wk_adapters::native_evidence::*;
    #[tokio::test]
    async fn cancellation_after_policy_wait_is_checked_from_current_storage() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("app")).unwrap();
        let b = wk_scope::enrollment::Binding::approve(&root, "test", vec!["app".into()]).unwrap();
        let c = state::Config {
            root: root.clone(),
            binding_file: root.join("binding.json"),
            service: "app".into(),
            server: "http://127.0.0.1:1".into(),
            model: "test".into(),
            inference: "http://127.0.0.1:1".into(),
            port: 1,
            account: "test".into(),
            device_id: "d".into(),

            device: None,
        };
        state::save(&c.binding_file, &b).unwrap();
        state::save(&root.join("agent.json"), &c).unwrap();
        let e = native_enrollment::Enrollment {
            client: Client::Codex,
            version: "0.162.0-alpha.2".into(),
            surface: "cli".into(),
            device_id: "d".into(),
            canonical_workspace: root.join("app"),
            service: "app".into(),
            policy_hash: b.policy_hash.clone(),
            enabled: true,
        };
        state::save(&root.join("native-codex.json"), &e).unwrap();
        let p = json!({"session_id":"s","turn_id":"t","cwd":e.canonical_workspace,"hook_event_name":"UserPromptSubmit","prompt":"synthetic"});
        let first = wk_adapters::codex_session::apply(None, &p, &e.canonical_workspace).unwrap();
        let stop = json!({"session_id":"s","turn_id":"t","cwd":e.canonical_workspace,"hook_event_name":"Stop","last_assistant_message":"ok","stop_hook_active":false});
        let turn =
            wk_adapters::codex_session::apply(Some(&first), &stop, &e.canonical_workspace).unwrap();
        let mut r = Record {
            enrollment: e.clone(),
            expires_at: 200,
            last_checked: 100,
            source: None,
            snapshot: None,
            turns: vec![turn.clone()],
            hold_reason: None,
        };
        let draft = json!({"source_id":"native2:source","native_adapter":"codex","turn_id":"t","source_digest":turn.digest,"parser_version":1,"client_version":"0.162.0-alpha.2","policy_hash":b.policy_hash,"policy_revision":1,"card":{"policy_revision":1}});
        let mut store = wk_local::LocalStore::open(&root.join("db"), [8; 32], 1000000).unwrap();
        store
            .update("native2:source", 100, 100, |_| {
                Ok(serde_json::to_vec(&r).unwrap())
            })
            .unwrap();
        assert!(check_current(&root, &store, &draft, 100, 1).is_ok());
        // A cancellation arrives while the remote permission response is pending.
        let (tx, rx) = tokio::sync::oneshot::channel();
        r.turns[0].terminal = Terminal::Cancelled;
        store
            .update("native2:source", 101, 99, |_| {
                Ok(serde_json::to_vec(&r).unwrap())
            })
            .unwrap();
        tx.send(()).unwrap();
        rx.await.unwrap();
        assert!(check_current(&root, &store, &draft, 101, 1).is_err());
        r.turns[0].terminal = Terminal::Complete;
        store
            .update("native2:source", 102, 98, |_| {
                Ok(serde_json::to_vec(&r).unwrap())
            })
            .unwrap();
        let mut disabled = e;
        disabled.enabled = false;
        state::save(&root.join("native-codex.json"), &disabled).unwrap();
        assert!(check_current(&root, &store, &draft, 102, 1).is_err());
    }
    #[test]
    fn changed_source_expiry_hold_and_policy_block_draft() {
        let mut r = Record {
            enrollment: native_enrollment::Enrollment {
                client: Client::Codex,
                version: "0.162.0-alpha.2".into(),
                surface: "cli".into(),
                device_id: "d".into(),
                canonical_workspace: "/tmp/synthetic".into(),
                service: "app".into(),
                policy_hash: "p".into(),
                enabled: true,
            },
            expires_at: 200,
            last_checked: 100,
            source: None,
            snapshot: None,
            hold_reason: None,
            turns: vec![Evidence {
                client: Client::Codex,
                client_version: "0.162.0-alpha.2".into(),
                parser_version: 1,
                session: "s".into(),
                agent: "main".into(),
                turn: "t".into(),
                prompt: "hi".into(),
                response: "ok".into(),
                messages_complete: true,
                scope: Scope::Verified,
                terminal: Terminal::Complete,
                digest: "new".into(),
                files_read: vec![],
                files_changed: vec![],
                hold_reason: None,
            }],
        };
        let mut draft = json!({"native_adapter":"codex","turn_id":"t","source_digest":"new","parser_version":1,"client_version":"0.162.0-alpha.2","policy_hash":"p","policy_revision":1,"card":{"policy_revision":1}});
        assert!(matches(&draft, &r, 100, 1));
        assert!(!matches(&draft, &r, 200, 1));
        assert!(!matches(&draft, &r, 100, 2));
        for (field, v) in [
            ("source_digest", json!("old")),
            ("parser_version", json!(2)),
            ("turn_id", json!("other")),
            ("client_version", json!("new")),
            ("policy_hash", json!("other")),
        ] {
            let mut bad = draft.clone();
            bad[field] = v;
            assert!(!matches(&bad, &r, 100, 1), "{field}");
        }
        draft["card"]["policy_revision"] = json!(2);
        assert!(!matches(&draft, &r, 100, 1));
        draft["card"]["policy_revision"] = json!(1);
        r.hold_reason = Some("uncertain".into());
        assert!(!matches(&draft, &r, 100, 1));
        r.hold_reason = None;
        r.turns[0].terminal = Terminal::Cancelled;
        assert!(!matches(&draft, &r, 100, 1));
    }
}
