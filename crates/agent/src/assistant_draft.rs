//! `wk-agent draft`: a card written by the developer's own assistant, kept as a private local draft.
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use wk_contracts::{CardSubmission, Completeness, Lifecycle};

const MAX_INPUT: usize = 64 * 1024;
/// The local store's maximum retention.
pub(crate) const TTL: i64 = 86_400;
/// How many times the same card can be drafted again after earlier copies were submitted or expired.
const MAX_COPIES: u32 = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    title: String,
    intent: String,
    rationale: String,
    decisions: Vec<String>,
    constraints: Vec<String>,
    open_questions: Vec<String>,
}

/// The label of the assistant that wrote the card: `[a-z0-9-]{1,32}`.
pub(crate) fn valid_client(client: &str) -> Result<(), String> {
    if (1..=32).contains(&client.len())
        && client
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        Ok(())
    } else {
        Err("client must be 1-32 characters of a-z, 0-9 or -".into())
    }
}

/// Validate the assistant's JSON and build the private draft record: `(draft key, record)`.
/// The card's `idempotency_key` is the content digest, so the server dedupes re-submissions.
pub(crate) fn build(
    text: &str,
    service: &str,
    revision: u64,
    policy_hash: &str,
    client: &str,
) -> Result<(String, Value), String> {
    valid_client(client)?;
    if text.len() > MAX_INPUT {
        return Err("card input too large".into());
    }
    let i: Input = serde_json::from_str(text).map_err(|_| {
        "invalid card JSON; expected title, intent, rationale, decisions, constraints, open_questions"
    })?;
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                &i.title,
                &i.intent,
                &i.rationale,
                &i.decisions,
                &i.constraints,
                &i.open_questions,
                service,
                policy_hash,
                revision,
                "assistant-v1"
            ))
            .map_err(|_| "invalid card")?
        )
    );
    let card = CardSubmission {
        schema_version: 1,
        idempotency_key: digest.clone(),
        policy_revision: revision,
        title: i.title,
        intent: i.intent,
        rationale: i.rationale,
        decisions: wk_summarize::tidy(i.decisions),
        constraints: wk_summarize::tidy(i.constraints),
        open_questions: wk_summarize::tidy(i.open_questions),
        service_ids: vec![service.to_string()],
        evidence: vec![],
        lifecycle: Lifecycle::Proposed,
        capture_completeness: Completeness::Partial,
        base_revision: None,
        observed_at_ms: None,
    };
    card.validate()?;
    if wk_summarize::sensitive(&serde_json::to_string(&card).map_err(|_| "invalid card")?) {
        return Err("card requires private review (possible credential)".into());
    }
    let record = json!({"card": card, "assistant": {"client": client}, "policy_hash": policy_hash, "model": "assistant"});
    Ok((format!("draft:{digest}"), record))
}

/// The nth local id for one card. The first is the content digest; later ones
/// hash in a counter, so they stay deterministic and the same length as `draft:` + 64 hex.
fn copy_key(base: &str, n: u32) -> String {
    if n == 0 {
        return base.to_string();
    }
    format!("draft:{:x}", Sha256::digest(format!("{base}:{n}")))
}

/// Write the draft and return its id. Drafting a card that is still pending
/// keeps that copy. The store never lets a submitted or expired id be created
/// again, so a card drafted again after submission gets the next counter id and
/// can be reviewed and submitted anew; the server dedupes by `idempotency_key`.
pub(crate) fn store_draft(
    store: &mut wk_local::LocalStore,
    key: &str,
    record: &Value,
    now: i64,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(record).map_err(|_| "invalid draft")?;
    for n in 0..MAX_COPIES {
        let id = copy_key(key, n);
        if store.contains(&id, now)? {
            return Ok(id);
        }
        if !store.seen(&id)? {
            store.update(&id, now, TTL, |_| Ok(bytes.clone()))?;
            return Ok(id);
        }
    }
    Err("this card was drafted too many times; edit it or submit a pending copy".into())
}

/// What `submit` does with an assistant draft before posting it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    /// Drafted under another approval or policy revision: it can never be valid, so it is consumed.
    Consume,
    /// Needs review for sensitive content: an error, and the draft stays.
    Hold,
    Ready,
}

pub(crate) fn assistant_ready(draft: &Value, policy_hash: &str, revision: u64) -> Readiness {
    if draft["policy_hash"] != policy_hash || draft["card"]["policy_revision"] != revision {
        Readiness::Consume
    } else if wk_summarize::sensitive(&draft["card"].to_string()) {
        Readiness::Hold
    } else {
        Readiness::Ready
    }
}

/// `wk-agent draft`: check approval and policy, then store a private draft.
pub(crate) async fn run(dir: &std::path::Path, client: &str, input: &str) -> Result<Value, String> {
    valid_client(client)?;
    let c = crate::state::load(dir)?;
    let b = crate::state::binding(&c)?;
    let token = crate::state::token(dir, "server-token")?;
    let policy = crate::network::policy(&c, &b, &token).await?;
    let revision = policy["policy_revision"].as_u64().ok_or("invalid policy")?;
    let (key, record) = build(input, &c.service, revision, &b.policy_hash, client)?;
    let mut store = crate::state::store(dir)?;
    let id = store_draft(&mut store, &key, &record, crate::daemon::now()?)?;
    Ok(
        json!({"draft_id": id, "private": true, "published": false, "next": "review with `wk-agent drafts`, then `wk-agent submit --id ID --reviewed`"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const OK: &str = r#"{"title":"Cap retries at 3","intent":"Avoid repeated charges","rationale":"The gateway charges again on duplicate calls; an idempotency key plus a cap keeps retries safe.","decisions":["Retries capped at 3"],"constraints":["Gateway repeats charges"],"open_questions":[]}"#;
    fn store() -> (tempfile::TempDir, wk_local::LocalStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = wk_local::LocalStore::open(&dir.path().join("t.db"), [7; 32], 1_000_000).unwrap();
        (dir, s)
    }
    #[test]
    fn builds_a_partial_proposed_card_with_a_stable_key() {
        let (key, draft) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        assert!(key.starts_with("draft:") && key.len() == 70);
        assert_eq!(draft["card"]["capture_completeness"], "partial");
        assert_eq!(draft["card"]["lifecycle"], "proposed");
        assert_eq!(draft["card"]["service_ids"], json!(["backend"]));
        assert_eq!(draft["card"]["policy_revision"], 7);
        assert_eq!(draft["card"]["idempotency_key"], key["draft:".len()..]);
        assert_eq!(draft["assistant"]["client"], "claude-code");
        assert_eq!(draft["model"], "assistant");
        assert_eq!(
            build(OK, "backend", 7, "hash1", "claude-code").unwrap().0,
            key,
            "same card, same id"
        );
        assert_ne!(
            build(OK, "backend", 8, "hash1", "claude-code").unwrap().0,
            key
        );
    }
    #[test]
    fn strict_fields_and_size_limits() {
        assert!(build(r#"{"title":"t","intent":"i","rationale":"r","decisions":[],"constraints":[],"open_questions":[],"extra":1}"#, "b", 1, "h", "x").is_err());
        assert!(build("{}", "b", 1, "h", "x").is_err());
        assert!(build(&"x".repeat(65 * 1024), "b", 1, "h", "x").is_err());
    }
    #[test]
    fn errors_never_echo_the_input() {
        let err = build(r#"{"title":"leaky-marker-1"#, "b", 1, "h", "x").unwrap_err();
        assert!(!err.contains("leaky-marker"));
        let blank = OK.replace("Cap retries at 3", "  ");
        assert!(build(&blank, "backend", 1, "h", "x").is_err());
    }
    #[test]
    fn sensitive_cards_are_refused_without_echo() {
        let credential = ["sk-", "proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"].concat();
        let bad = OK.replace("Avoid repeated charges", &format!("key {credential}"));
        let err = build(&bad, "backend", 1, "h", "x").unwrap_err();
        assert_eq!(err, "card requires private review (possible credential)");
        assert!(!err.contains("proj-"));
    }
    #[test]
    fn client_labels_are_restricted() {
        for ok in ["claude-code", "codex", "a", &"a".repeat(32)] {
            assert!(valid_client(ok).is_ok(), "{ok}");
        }
        for bad in ["", "Claude", "a b", "a_b", "../x", &"a".repeat(33), "é"] {
            assert!(valid_client(bad).is_err(), "{bad}");
        }
        assert!(build(OK, "backend", 1, "h", "Bad Client").is_err());
    }
    #[test]
    fn store_draft_writes_once_with_the_store_maximum_retention() {
        let (_dir, mut s) = store();
        let (key, draft) = build(OK, "backend", 7, "hash1", "codex").unwrap();
        let now = 1_000;
        assert_eq!(store_draft(&mut s, &key, &draft, now).unwrap(), key);
        let read = |s: &wk_local::LocalStore, id: &str, at: i64| {
            s.read_record(id, at)
                .unwrap()
                .map(|b| serde_json::from_slice::<Value>(&b).unwrap())
        };
        assert_eq!(read(&s, &key, now), Some(draft.clone()));
        assert!(read(&s, &key, now + TTL - 1).is_some());
        assert!(read(&s, &key, now + TTL).is_none(), "24-hour retention");
        // A second identical write keeps the first copy and its deadline.
        let mut other = draft.clone();
        other["assistant"]["client"] = json!("copilot");
        assert_eq!(store_draft(&mut s, &key, &other, now + 60).unwrap(), key);
        assert_eq!(read(&s, &key, now + 60), Some(draft));
        assert_eq!(s.pending_ids("draft:", "", now, 100).unwrap(), [key]);
    }
    #[test]
    fn a_card_drafted_again_after_submission_gets_a_new_draft() {
        let (_dir, mut s) = store();
        let (key, draft) = build(OK, "backend", 7, "hash1", "codex").unwrap();
        let now = 1_000;
        assert_eq!(store_draft(&mut s, &key, &draft, now).unwrap(), key);
        s.ack(&key).unwrap(); // submitted
        let second = store_draft(&mut s, &key, &draft, now + 10).unwrap();
        assert_ne!(second, key);
        assert!(second.starts_with("draft:") && second.len() == 70);
        assert!(s.contains(&second, now + 10).unwrap());
        // While that copy is pending, drafting again keeps it.
        assert_eq!(store_draft(&mut s, &key, &draft, now + 20).unwrap(), second);
        // Submitted and expired copies are skipped in turn; the card keeps its idempotency key.
        s.ack(&second).unwrap();
        let third = store_draft(&mut s, &key, &draft, now + 30).unwrap();
        assert!(third != key && third != second);
        let bytes = s.read_record(&third, now + 30).unwrap().unwrap();
        let stored: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored["card"]["idempotency_key"], key["draft:".len()..]);
        // After expiry of the first copy's window a new draft can still be made.
        assert!(store_draft(&mut s, &key, &draft, now + TTL + 100).is_ok());
    }
    #[test]
    fn redrafting_is_bounded() {
        let (_dir, mut s) = store();
        let (key, draft) = build(OK, "backend", 7, "hash1", "codex").unwrap();
        for _ in 0..MAX_COPIES {
            let id = store_draft(&mut s, &key, &draft, 1_000).unwrap();
            s.ack(&id).unwrap();
        }
        assert!(store_draft(&mut s, &key, &draft, 1_000).is_err());
    }
    #[test]
    fn readiness_consumes_on_mismatch_holds_sensitive_and_otherwise_passes() {
        let (_, draft) = build(OK, "backend", 7, "hash1", "codex").unwrap();
        assert_eq!(assistant_ready(&draft, "hash1", 7), Readiness::Ready);
        assert_eq!(assistant_ready(&draft, "other", 7), Readiness::Consume);
        assert_eq!(assistant_ready(&draft, "hash1", 8), Readiness::Consume);
        let mut sensitive = draft.clone();
        sensitive["card"]["rationale"] = json!(["pass", "word"].concat());
        assert_eq!(assistant_ready(&sensitive, "hash1", 7), Readiness::Hold);
        // A mismatch outranks sensitivity: such a draft can never be valid.
        assert_eq!(assistant_ready(&sensitive, "hash1", 9), Readiness::Consume);
    }
}
