use crate::{daemon, network, state};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
pub async fn run(dir: &Path) -> Result<Value, String> {
    if dir.join("capture-gap.json").exists() {
        return Err("capture gap requires reset into a clean session".into());
    }
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let token = state::token(dir, "server-token")?;
    let policy = network::policy(&c, &b, &token).await?;
    let mut store = state::store(dir)?;
    let ids = store.next_session_ids(daemon::now()?)?;
    let mut drafts = 0;
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
        let mut turns = std::collections::BTreeMap::<String, Vec<&Value>>::new();
        for event in events {
            let key = format!("{}:{}", event["agent_id"], event["turn_id"]);
            turns.entry(key).or_default().push(event);
        }
        for (turn_key, events) in &turns {
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
                        id.as_str(),
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
    }
    Ok(json!({"private_drafts_created":drafts,"held":held,"published":false}))
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
        result.push(json!({"id":id,"card":v["card"],"model":v["model"]}));
    }
    Ok(json!({"private_drafts":result,"next_cursor":next_cursor}))
}
pub async fn submit(dir: &Path, id: &str) -> Result<Value, String> {
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
    let response = network::post(
        &c,
        &t,
        "/v1/cards",
        &json!({"product":b.product,"card":v["card"]}),
    )
    .await?;
    s.ack(id)?;
    Ok(response)
}
