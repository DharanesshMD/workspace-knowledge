//! Offline synthetic event inspection. Never outputs conversation text.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use wk_scope::enrollment::Binding;
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    UserPrompt,
    AssistantResponse,
    ToolResult,
    TurnEnd,
    CaptureGap,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Event {
    schema_version: u32,
    event_id: String,
    session_id: String,
    agent_id: String,
    turn_id: String,
    sequence: u32,
    kind: Kind,
    path: PathBuf,
    text: String,
}
pub fn inspect(binding: &Binding, input: &str) -> Result<Value, String> {
    if input.len() > 8 * 1024 * 1024 {
        return Err("replay exceeds size limit".into());
    }
    let mut seen: BTreeMap<(String, String, String), Event> = BTreeMap::new();
    for line in input.lines().filter(|s| !s.trim().is_empty()) {
        if line.len() > 1024 * 1024 {
            return Err("event exceeds size limit".into());
        }
        let e: Event = serde_json::from_str(line).map_err(|_| "invalid event schema")?;
        if e.schema_version != 1
            || [&e.event_id, &e.session_id, &e.agent_id, &e.turn_id]
                .iter()
                .any(|s| s.is_empty() || s.len() > 256)
        {
            return Err("invalid event identity or version".into());
        }
        let key = (e.session_id.clone(), e.agent_id.clone(), e.event_id.clone());
        if let Some(previous) = seen.get(&key) {
            if previous != &e {
                return Err("conflicting duplicate event".into());
            }
        } else {
            seen.insert(key, e);
        }
    }
    let mut mixed = BTreeSet::new();
    let mut turns: BTreeMap<(&str, &str, &str), Vec<&Event>> = BTreeMap::new();
    for e in seen.values() {
        if !binding.eligible(&e.path) {
            mixed.insert(e.session_id.as_str());
        }
        turns
            .entry((&e.session_id, &e.agent_id, &e.turn_id))
            .or_default()
            .push(e);
    }
    let mut results = vec![];
    for ((session, agent, turn), mut events) in turns {
        events.sort_by_key(|e| e.sequence);
        let ordered = events.windows(2).all(|w| w[0].sequence < w[1].sequence);
        let complete = ordered
            && events.first().is_some_and(|e| e.kind == Kind::UserPrompt)
            && events.last().is_some_and(|e| e.kind == Kind::TurnEnd)
            && events.iter().filter(|e| e.kind == Kind::UserPrompt).count() == 1
            && events.iter().filter(|e| e.kind == Kind::TurnEnd).count() == 1
            && events.iter().any(|e| e.kind == Kind::AssistantResponse)
            && !events.iter().any(|e| e.kind == Kind::CaptureGap);
        let scoped = !mixed.contains(session);
        results.push(json!({"session_id":session,"agent_id":agent,"turn_id":turn,"event_count":events.len(),"complete":complete,"scope":if scoped{"approved"}else{"mixed"},"eligible":scoped&&complete}));
    }
    Ok(json!({"turns":results,"mode":"offline_inspection","published":false}))
}
