//! Private, bounded session journal for normalized adapter events.
//! This is not a live adapter, a publication decision, or a completeness proof.
use crate::Event;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use wk_local::LocalStore;
use wk_scope::enrollment::Binding;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    version: u32,
    policy_hash: String,
    held: bool,
    events: Vec<Event>,
}
/// Accept one already-normalized event from a trusted adapter boundary.
/// Session identity is namespaced by product/root; agents share contamination
/// state. The caller must authenticate adapters, observe gaps, and supply trusted
/// wall time. Neither client-supplied paths nor this receipt authorize publication.
pub fn ingest(
    store: &mut LocalStore,
    binding: &Binding,
    input: &str,
    now: i64,
) -> Result<Value, String> {
    if input.len() > 256 * 1024 {
        return Err("event exceeds size limit".into());
    }
    let event: Event = serde_json::from_str(input).map_err(|_| "invalid event schema")?;
    if event.schema_version != 1
        || [
            &event.event_id,
            &event.session_id,
            &event.agent_id,
            &event.turn_id,
        ]
        .iter()
        .any(|s| s.is_empty() || s.len() > 256)
    {
        return Err("invalid event identity or version".into());
    }
    let identity = serde_json::to_vec(&(&binding.root, &binding.product, &event.session_id))
        .map_err(|_| "invalid session identity")?;
    let key = format!("session:{:x}", Sha256::digest(identity));
    let approved = binding.eligible(&event.path);
    let mut receipt = json!({"stored":false,"held":true,"event_count":0,"published":false});
    store.update(&key, now, 86400, |previous| {
        let mut state: Session = match previous {
            Some(bytes) => serde_json::from_slice(bytes).map_err(|_| "invalid private session")?,
            None => Session { version:1, policy_hash:binding.policy_hash.clone(), held:false, events:vec![] },
        };
        if state.version != 1 { return Err("unsupported private session version".into()); }
        let duplicate = state.events.iter().find(|e| e.agent_id == event.agent_id && e.event_id == event.event_id);
        let conflict = duplicate.is_some_and(|e| e != &event);
        let exists = duplicate.is_some();
        // An excluded path is inspected only as metadata. Its text and path never
        // enter persisted state. Remove earlier payloads once contamination is known.
        if !approved || conflict || state.policy_hash != binding.policy_hash || event.kind == crate::Kind::CaptureGap {
            state.held = true;
        }
        if state.held {
            state.events.clear();
        } else if !exists {
            state.events.push(event);
        }
        // Fail closed with a content-free hold on oversized sessions. There is no
        // truncated payload that a later consumer could mistake for a complete one.
        let mut bytes = serde_json::to_vec(&state).map_err(|_| "invalid private session")?;
        if bytes.len() > 768 * 1024 || state.events.len() > 1000 {
            state.held = true;
            state.events.clear();
            bytes = serde_json::to_vec(&state).map_err(|_| "invalid private session")?;
        }
        receipt = json!({"stored":true,"held":state.held,"event_count":state.events.len(),"published":false});
        Ok(bytes)
    })?;
    Ok(receipt)
}
