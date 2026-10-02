//! Explicit bound-chat pull. Run only on an isolated broker account with a read-only provider key.
use crate::{chat, daemon, network, state};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    chat_id: String,
    provider_org: String,
    provider_user: String,
    root: PathBuf,
    service: String,
    approval_hash: String,
    expires_at: i64,
}
pub async fn poll(dir: &Path, file: &Path) -> Result<Value, String> {
    if std::env::var("WK_COMPLIANCE_BROKER").as_deref() != Ok("1") {
        return Err("Compliance pulls require isolated broker mode".into());
    }
    let g: Grant =
        serde_json::from_str(&state::read(file, 65536)?).map_err(|_| "invalid chat grant")?;
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    if !wk_contracts::valid_id(&g.chat_id)
        || g.root != b.root
        || g.service != c.service
        || g.approval_hash != b.policy_hash
        || g.expires_at <= daemon::now()?
        || g.provider_org.is_empty()
        || g.provider_user.is_empty()
    {
        return Err("chat grant expired or binding mismatch".into());
    }
    let token = state::token(dir, "server-token")?;
    network::policy(&c, &b, &token).await?;
    let provider = zeroize::Zeroizing::new(
        std::env::var("WK_COMPLIANCE_READ_KEY")
            .map_err(|_| "read-only Compliance key required on broker")?,
    );
    let url = format!(
        "https://api.anthropic.com/v1/compliance/apps/chats/{}/messages",
        g.chat_id
    );
    let client = network::client(15)?;
    let mut after = None;
    let mut count = 0;
    let mut pages = 0;
    let mut store = state::store(dir)?;
    let session = format!("compliance:{}", g.chat_id);
    loop {
        if g.expires_at <= daemon::now()? {
            return Err("chat grant expired".into());
        }
        state::binding(&c)?;
        network::policy(&c, &b, &token).await?;
        let mut req = client
            .get(&url)
            .header("x-api-key", provider.as_str())
            .header("anthropic-version", "2023-06-01")
            .query(&[("limit", "100"), ("order", "asc")]);
        if let Some(ref id) = after {
            req = req.query(&[("after_id", id)]);
        }
        let response = req
            .send()
            .await
            .map_err(|_| "Compliance service unavailable")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND
            || response.status() == reqwest::StatusCode::GONE
        {
            let _ = chat::record(
                &mut store,
                &b,
                &c.service,
                chat::Record {
                    session: &session,
                    turn: "deleted",
                    id: "deleted",
                    kind: "capture_gap",
                    text: "",
                    seq: 0,
                    agent: "compliance",
                },
            );
            return Err("bound source unavailable; session held".into());
        }
        let v = wk_summarize::bounded_json(response, 1024 * 1024).await?;
        if v["id"] != g.chat_id
            || v["organization_uuid"] != g.provider_org
            || v["user"]["id"] != g.provider_user
            || !v["deleted_at"].is_null()
        {
            return Err("provider ownership mismatch or source deleted".into());
        }
        let messages = v["chat_messages"]
            .as_array()
            .ok_or("invalid Compliance schema")?;
        for m in messages {
            let id = m["id"]
                .as_str()
                .filter(|s| wk_contracts::valid_id(s))
                .ok_or("invalid source identity")?;
            let kind = match m["role"].as_str() {
                Some("user") => "user_prompt",
                Some("assistant") => "assistant_response",
                _ => continue,
            };
            let mut text = String::new();
            for block in m["content"].as_array().ok_or("invalid message content")? {
                if block["type"] == "text" {
                    text.push_str(block["text"].as_str().ok_or("invalid text block")?);
                    text.push('\n');
                }
            }
            // No attachment download and no reasoning extraction. Coverage remains partial.
            chat::record(
                &mut store,
                &b,
                &c.service,
                chat::Record {
                    session: &session,
                    turn: id,
                    id,
                    kind,
                    text: &text,
                    seq: 1,
                    agent: "compliance",
                },
            )?;
            count += 1;
        }
        pages += 1;
        if messages.len() < 100 {
            break;
        }
        if pages >= 20 {
            return Err("Compliance pagination limit reached; coverage incomplete".into());
        }
        let last = messages
            .last()
            .and_then(|m| m["id"].as_str())
            .ok_or("missing cursor")?
            .to_string();
        if after.as_ref() == Some(&last) {
            return Err("repeating source cursor".into());
        }
        after = Some(last);
    }
    Ok(
        json!({"captured_messages":count,"pages":pages,"coverage":"partial_text_only","published":false,"next_poll_after_seconds":60}),
    )
}
