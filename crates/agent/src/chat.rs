use crate::{daemon, network, state};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub id: String,
    pub role: String,
    pub text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Export {
    schema_version: u32,
    workspace: String,
    product: String,
    session_id: String,
    messages: Vec<Message>,
}
pub fn session_key(
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    session: &str,
) -> Result<String, String> {
    Ok(format!(
        "session:{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&c.root, &b.product, session)).map_err(|_| "invalid session")?
        )
    ))
}
pub struct Record<'a> {
    pub session: &'a str,
    pub turn: &'a str,
    pub id: &'a str,
    pub kind: &'a str,
    pub text: &'a str,
    pub seq: u32,
    pub agent: &'a str,
}
pub fn record(
    store: &mut wk_local::LocalStore,
    b: &wk_scope::enrollment::Binding,
    service: &str,
    r: Record<'_>,
) -> Result<(), String> {
    let Record {
        session,
        turn,
        id,
        kind,
        text,
        seq,
        agent,
    } = r;
    let event = json!({"schema_version":1,"event_id":id,"session_id":session,"agent_id":agent,"turn_id":turn,"sequence":seq,"kind":kind,"path":b.root.join(service),"text":text});
    let r = wk_capture::journal::ingest(store, b, &event.to_string(), daemon::now()?)?;
    if r["held"] == true {
        return Err("session held; start a clean conversation".into());
    }
    Ok(())
}
pub async fn run(dir: &Path, file: &Path, session: Option<String>) -> Result<Value, String> {
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let token = state::token(dir, "server-token")?;
    network::policy(&c, &b, &token).await?;
    let prompt = state::read(file, 64 * 1024)?;
    // An explicit file is a user-supplied message, not an implicit product scan.
    if wk_summarize::sensitive(&prompt) {
        return Err("message contains potentially private or secret content".into());
    }
    let session = match session {
        Some(s) if s.starts_with("managed:") && s.len() < 128 => s,
        Some(_) => return Err("invalid managed session".into()),
        None => format!("managed:{}", uuid::Uuid::new_v4()),
    };
    let mut store = state::store(dir)?;
    let mut messages = vec![
        json!({"role":"system","content":"You are a product development assistant in an explicitly approved workspace. Discuss the requested work. No file access or tools are available. Do not claim to have implemented code or run tests. Only visible messages are captured."}),
    ];
    if let Some(bytes) = store.read_record(&session_key(&c, &b, &session)?, daemon::now()?)? {
        let old: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid session")?;
        if old["held"] == true || old["policy_hash"] != b.policy_hash {
            return Err("session held or approval changed".into());
        }
        for e in old["events"].as_array().ok_or("invalid session")? {
            let role = match e["kind"].as_str() {
                Some("user_prompt") => "user",
                Some("assistant_response") => "assistant",
                _ => continue,
            };
            messages.push(json!({"role":role,"content":e["text"]}));
        }
    }
    messages.push(json!({"role":"user","content":prompt}));
    if serde_json::to_vec(&messages)
        .map_err(|_| "invalid chat")?
        .len()
        > 128 * 1024
    {
        return Err("chat context limit reached; start a new session".into());
    }
    let turn = uuid::Uuid::new_v4().to_string();
    record(
        &mut store,
        &b,
        &c.service,
        Record {
            session: &session,
            turn: &turn,
            id: &format!("{turn}:prompt"),
            kind: "user_prompt",
            text: &prompt,
            seq: 1,
            agent: "managed-chat",
        },
    )?;
    let url = wk_summarize::local_url(&c.inference)?;
    let response = network::client(60)?
        .post(url)
        .json(&json!({"model":c.model,"messages":messages,"stream":false,"max_tokens":4096}))
        .send()
        .await
        .map_err(|_| "local chat provider unavailable")?;
    let v = wk_summarize::bounded_json(response, 128 * 1024).await?;
    if v["choices"][0]["finish_reason"] != "stop" {
        record(
            &mut store,
            &b,
            &c.service,
            Record {
                session: &session,
                turn: &turn,
                id: &format!("{turn}:gap"),
                kind: "capture_gap",
                text: "",
                seq: 3,
                agent: "managed-chat",
            },
        )?;
        return Err("chat response interrupted or truncated".into());
    }
    let text = v["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("missing visible response")?;
    // Recheck local consent and server membership after a long generation.
    let b = state::binding(&c)?;
    network::policy(&c, &b, &token).await?;
    record(
        &mut store,
        &b,
        &c.service,
        Record {
            session: &session,
            turn: &turn,
            id: &format!("{turn}:response"),
            kind: "assistant_response",
            text,
            seq: 2,
            agent: "managed-chat",
        },
    )?;
    record(
        &mut store,
        &b,
        &c.service,
        Record {
            session: &session,
            turn: &turn,
            id: &format!("{turn}:end"),
            kind: "turn_end",
            text: "",
            seq: 3,
            agent: "managed-chat",
        },
    )?;
    Ok(
        json!({"session":session,"response":text,"coverage":"complete_visible_messages_within_managed_chat","published":false}),
    )
}
pub async fn import(
    dir: &Path,
    file: &Path,
    session: &str,
    approved: bool,
) -> Result<Value, String> {
    if !approved {
        return Err("explicit --approve-transcript required for this exact file".into());
    }
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    let token = state::token(dir, "server-token")?;
    network::policy(&c, &b, &token).await?;
    let e: Export = serde_json::from_str(&state::read(file, 768 * 1024)?)
        .map_err(|_| "invalid scoped export")?;
    if e.schema_version != 1
        || e.session_id != session
        || e.product != b.product
        || Path::new(&e.workspace)
            .canonicalize()
            .map_err(|_| "export workspace unavailable")?
            != b.root
        || e.messages.len() > 500
    {
        return Err("export binding mismatch".into());
    }
    let mut store = state::store(dir)?;
    let mut count = 0;
    for m in e.messages {
        let kind = match m.role.as_str() {
            "user" => "user_prompt",
            "assistant" => "assistant_response",
            _ => return Err("unsupported export role".into()),
        };
        record(
            &mut store,
            &b,
            &c.service,
            Record {
                session: &format!("export:{session}"),
                turn: &m.id,
                id: &m.id,
                kind,
                text: &m.text,
                seq: 1,
                agent: "explicit-export",
            },
        )?;
        count += 1;
    }
    Ok(json!({"captured_messages":count,"coverage":"partial_export","published":false}))
}
