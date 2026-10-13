//! Bounded visible-message parsing. Terminal correlation is not yet verified.
use crate::native_evidence::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub consumed: usize,
    pub prefix_digest: String,
    pub turns: Vec<Evidence>,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn prompt(text: &str) -> Result<String, String> {
    let text = text
        .strip_prefix("<USER_REQUEST>\n")
        .ok_or("unsupported_user_wrapper")?;
    let (message, tail) = text
        .split_once("\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nThe current local time is: ")
        .ok_or("unsupported_user_wrapper")?;
    if message.is_empty() || message.contains(['<', '>']) {
        return Err("unsupported_user_context".into());
    }
    let (time, tail) = tail
        .split_once(".\n</ADDITIONAL_METADATA>")
        .ok_or("unsupported_user_wrapper")?;
    if time.len() != 25
        || !time
            .bytes()
            .all(|b| b.is_ascii_digit() || b"-T:+".contains(&b))
    {
        return Err("unsupported_metadata".into());
    }
    const SETTINGS:&str="\n<USER_SETTINGS_CHANGE>\nThe user changed setting `Model Selection` from None to Gemini 3.8 Flash (Medium). No need to comment on this change if the user doesn't ask about it. If reporting what model you are, please use a human readable name instead of the exact string.\n</USER_SETTINGS_CHANGE>";
    if !tail.is_empty() && tail != SETTINGS {
        return Err("unsupported_metadata".into());
    }
    Ok(message.into())
}
pub fn parse(bytes: &[u8]) -> Result<Snapshot, String> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("source_too_large".into());
    }
    let consumed = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
    if bytes.len() - consumed > 256 * 1024 {
        return Err("record_too_large".into());
    }
    let mut turns: Vec<Evidence> = vec![];
    let mut last = None;
    for line in bytes[..consumed]
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
    {
        if line.len() > 256 * 1024 {
            return Err("record_too_large".into());
        }
        let v: Value = serde_json::from_slice(line).map_err(|_| "invalid_transcript_record")?;
        let allowed = [
            "step_index",
            "type",
            "status",
            "content",
            "source",
            "created_at",
        ];
        if v.as_object()
            .ok_or("invalid_transcript_record")?
            .keys()
            .any(|k| !allowed.contains(&k.as_str()))
        {
            return Err("unsupported_record_fields".into());
        }
        let index = v["step_index"].as_u64().ok_or("invalid_step")?;
        if last.is_some_and(|n| index <= n) {
            return Err("step_order_changed".into());
        }
        last = Some(index);
        if v["status"] != "DONE" {
            return Err("record_not_done".into());
        }
        let text = v["content"].as_str().ok_or("unsupported_content")?;
        match v["type"].as_str() {
            Some("USER_INPUT") => turns.push(Evidence {
                client: Client::Antigravity,
                client_version: "2.19.1".into(),
                parser_version: 1,
                session: String::new(),
                agent: "main".into(),
                turn: index.to_string(),
                prompt: prompt(text)?,
                response: String::new(),
                messages_complete: false,
                scope: Scope::Unknown,
                terminal: Terminal::Held,
                digest: String::new(),
                files_read: vec![],
                files_changed: vec![],
                hold_reason: Some("terminal_attribution_unverified".into()),
            }),
            Some("PLANNER_RESPONSE") => {
                let turn = turns.last_mut().ok_or("orphan_response")?;
                if text.trim().is_empty() {
                    return Err("empty_response".into());
                }
                if !turn.response.is_empty() {
                    turn.response.push('\n');
                }
                turn.response.push_str(text);
            }
            _ => return Err("unsupported_record_type".into()),
        }
    }
    for turn in &mut turns {
        turn.digest = digest(&serde_json::to_vec(turn).map_err(|_| "invalid_evidence")?);
    }
    Ok(Snapshot {
        consumed,
        prefix_digest: digest(&bytes[..consumed]),
        turns,
    })
}
pub fn reconcile(
    previous: Option<&Snapshot>,
    bytes: &[u8],
    _stop: Option<&Value>,
) -> Result<Snapshot, String> {
    if let Some(old) = previous {
        if bytes.len() < old.consumed || digest(&bytes[..old.consumed]) != old.prefix_digest {
            return Err("source_prefix_changed".into());
        }
    }
    // Repeated invocation/execution counters cannot associate even a normal Stop with a turn.
    parse(bytes)
}
