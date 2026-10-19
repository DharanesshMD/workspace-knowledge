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

/// Server capability that accepts `provenance` and publishes `assistant_written` cards.
const FEATURE: &str = "card_provenance";

/// Before submit: a server that does not advertise `card_provenance` (older than this change)
/// rejects the unknown field, so send the card as the plain `partial` draft it was before.
/// The body changes, so the idempotency key changes too (deterministically): otherwise, once the
/// server is upgraded, running `/wk-summarize` again would send the same key with a different
/// body and conflict with the row stored without provenance.
/// Returns the note to show the developer when the field was dropped.
pub(crate) fn adapt_to_server(draft: &mut Value, policy: &Value) -> Option<&'static str> {
    let supported = policy["features"]
        .as_array()
        .is_some_and(|f| f.iter().any(|x| x == FEATURE));
    if supported {
        return None;
    }
    let card = draft["card"].as_object_mut()?;
    card.remove("provenance")?;
    if let Some(key) = card.get("idempotency_key").and_then(Value::as_str) {
        let key = format!("{:x}", Sha256::digest(format!("{key}:no-provenance")));
        card.insert("idempotency_key".into(), json!(key));
    }
    Some("this server cannot publish assistant-written cards yet, so the card was sent as a partial draft that cannot be published; after the server is upgraded, withdraw it and run /wk-summarize again")
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
    let given: Vec<String> = [&i.title, &i.intent, &i.rationale]
        .into_iter()
        .chain(&i.decisions)
        .chain(&i.constraints)
        .chain(&i.open_questions)
        .cloned()
        .collect();
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
                "assistant-v2"
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
        provenance: Some(wk_contracts::Provenance::AssistantWritten),
    };
    card.validate()?;
    // `sensitive_reason` returns a fixed label, never the matched text.
    if let Some(reason) =
        wk_summarize::sensitive_reason(&serde_json::to_string(&card).map_err(|_| "invalid card")?)
    {
        return Err(format!("card requires private review ({reason})"));
    }
    // The assistant's words are stored as written, not redacted. But anything the history
    // masker would change looks like a credential `sensitive` can miss (a dotted JWT, `token=`).
    if given
        .iter()
        .any(|f| wk_adapters::cursor_history::redact(f) != *f)
    {
        return Err("card requires private review (possible credential)".into());
    }
    // Assistant drafts are `Partial`, which `personal_sync::payload` refuses, so they are never
    // uploaded to the owner's personal store automatically. A test pins that dependency.
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

/// The error `LocalStore::update` gives for an id that was submitted or expired.
const CLOSED: &str = "session closed; a new session identity is required";

/// Create the draft under `id` unless it already exists (the first copy is kept).
/// `false` means the id is closed: it was submitted or expired, and the store never reopens it.
fn place(
    store: &mut wk_local::LocalStore,
    id: &str,
    bytes: &[u8],
    now: i64,
) -> Result<bool, String> {
    match store.update(id, now, TTL, |old| {
        Ok(old.map_or_else(|| bytes.to_vec(), |b| b.to_vec()))
    }) {
        Ok(()) => Ok(true),
        Err(e) if e == CLOSED => Ok(false),
        Err(e) => Err(e),
    }
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
        // Closed ids are found by the write itself, so another process acknowledging a
        // copy between a check and the write cannot fail the draft.
        if place(store, &id, &bytes, now)? {
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

/// Apply the readiness decision for `submit`. Only a real policy mismatch consumes the draft
/// (never a malformed policy reply, never a sensitive hold), and an owner-only sync consumes
/// nothing. `Consume` and `Hold` are for the caller to report as errors.
pub(crate) fn settle_assistant(
    store: &mut wk_local::LocalStore,
    id: &str,
    draft: &Value,
    policy_hash: &str,
    policy_revision: &Value,
    owner_only: bool,
) -> Result<Readiness, String> {
    let revision = policy_revision.as_u64().ok_or("invalid policy")?;
    let readiness = assistant_ready(draft, policy_hash, revision);
    if readiness == Readiness::Consume && !owner_only {
        store.ack(id)?; // a draft for an older approval can never become valid
    }
    Ok(readiness)
}

const LISTS: [&str; 3] = ["decisions", "constraints", "open_questions"];

/// How many list items `tidy` removed from the assistant's input.
fn dropped(input: &str, record: &Value) -> usize {
    let given: usize = serde_json::from_str::<Value>(input).map_or(0, |v| {
        LISTS
            .iter()
            .map(|k| v[*k].as_array().map_or(0, Vec::len))
            .sum()
    });
    let kept: usize = LISTS
        .iter()
        .map(|k| record["card"][*k].as_array().map_or(0, Vec::len))
        .sum();
    given.saturating_sub(kept)
}

fn quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-:@%+=".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// The result `draft` prints. `note` appears only when list items were dropped.
fn result(id: &str, dir: &std::path::Path, dropped: usize) -> Value {
    let dir = quote(&dir.to_string_lossy());
    let mut out = json!({"draft_id": id, "private": true, "published": false, "dropped": dropped,
        "next": format!("review with `wk-agent --state-dir {dir} drafts`, then `wk-agent --state-dir {dir} submit --id {id} --reviewed`")});
    if dropped > 0 {
        out["note"] = json!(format!(
            "{dropped} short list items were dropped; write items as full short sentences"
        ));
    }
    out
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
    Ok(result(&id, dir, dropped(input, &record)))
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
    fn draft_is_assistant_written_with_a_v2_key() {
        let (key, draft) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        assert_eq!(draft["card"]["provenance"], "assistant_written");
        assert_eq!(draft["card"]["capture_completeness"], "partial");
        // The digest tag moved from "assistant-v1" so a resubmission after an upgrade is a new
        // card, not a body conflict with an older provenance-less row. Pin the tag change.
        assert!(include_str!("assistant_draft.rs").contains("\"assistant-v2\""));
        assert!(key.starts_with("draft:") && key.len() == 70);
    }

    #[test]
    fn provenance_is_dropped_for_a_server_that_does_not_advertise_it() {
        let (_, mut draft) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        let original = draft["card"]["idempotency_key"].clone();
        let mut again = draft.clone();
        let note = adapt_to_server(&mut draft, &json!({"policy_revision":7})).unwrap();
        assert!(note.contains("/wk-summarize"));
        assert!(draft["card"].get("provenance").is_none());
        // A distinct, stable key: the provenance-less body must not collide with the v2 key an
        // upgraded server will see for the same card.
        assert_ne!(draft["card"]["idempotency_key"], original);
        assert!(adapt_to_server(&mut again, &json!({})).is_some());
        assert_eq!(
            draft["card"]["idempotency_key"],
            again["card"]["idempotency_key"]
        );
        let (_, mut draft) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        assert!(adapt_to_server(&mut draft, &json!({"features":["card_provenance"]})).is_none());
        assert_eq!(draft["card"]["provenance"], "assistant_written");
        assert_eq!(draft["card"]["idempotency_key"], original);
        // A draft queued before this change has no provenance: it is sent as it is, key untouched.
        let (_, mut queued) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        queued["card"].as_object_mut().unwrap().remove("provenance");
        assert!(adapt_to_server(&mut queued, &json!({})).is_none());
        assert_eq!(queued["card"]["idempotency_key"], original);
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
        assert_eq!(err, "card requires private review (prefix:sk-)");
        assert!(!err.contains("proj-"));
        // Every label is a fixed rule name, whatever the content was.
        let bad = OK.replace(
            "Avoid repeated charges",
            "see /Users/someone/leaky-marker-7",
        );
        let err = build(&bad, "backend", 1, "h", "x").unwrap_err();
        assert_eq!(err, "card requires private review (home_path)");
        assert!(!err.contains("leaky-marker"));
    }
    #[test]
    fn credentials_the_screen_misses_are_caught_by_the_masker() {
        let jwt = [
            "eyJhbGciOiJIUzI1NiJ9",
            "eyJzdWIiOiIxMjM0NTY3ODkwIn0",
            "c2lnbmF0dXJlMTIzNDU2Nzg5MA",
        ]
        .join(".");
        let assignment = ["to", "ken=abc123xyz"].concat();
        for (field, leak) in [
            ("Avoid repeated charges", format!("sent {jwt} upstream")),
            ("Avoid repeated charges", format!("ran with {assignment}")),
            ("Retries capped at 3", format!("header {jwt}")),
        ] {
            let bad = OK.replace(field, &leak);
            assert!(bad != OK);
            let err = build(&bad, "backend", 1, "h", "x").unwrap_err();
            assert_eq!(err, "card requires private review (possible credential)");
            assert!(!err.contains("abc123") && !err.contains("eyJ"));
        }
        // Ordinary text is stored as written.
        let (_, record) = build(OK, "backend", 1, "h", "x").unwrap();
        assert_eq!(
            record["card"]["rationale"],
            serde_json::from_str::<Value>(OK).unwrap()["rationale"]
        );
    }
    #[test]
    fn assistant_drafts_are_never_uploaded_to_the_owner_store_automatically() {
        let (_, draft) = build(OK, "backend", 7, "hash1", "codex").unwrap();
        assert!(crate::personal_sync::payload(&draft["card"]).is_err());
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
        assert_ne!(third, second);
        // After expiry of the first copies' window a new draft can still be made, under an id
        // none of the earlier copies used.
        let fourth = store_draft(&mut s, &key, &draft, now + TTL + 100).unwrap();
        assert!(![&key, &second, &third].contains(&&fourth));
    }
    #[test]
    fn a_pending_record_is_kept_not_overwritten() {
        let (_dir, mut s) = store();
        let (key, draft) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        let first = serde_json::to_vec(&draft).unwrap();
        s.enqueue(&key, &first, 1_000, 3_600).unwrap();
        let mut other = draft.clone();
        other["assistant"]["client"] = json!("copilot");
        assert_eq!(store_draft(&mut s, &key, &other, 1_500).unwrap(), key);
        assert_eq!(s.read_record(&key, 1_500).unwrap(), Some(first));
    }
    #[test]
    fn a_closed_id_moves_to_the_next_copy_even_when_closed_after_a_check() {
        let (_dir, mut s) = store();
        let (key, draft) = build(OK, "backend", 7, "hash1", "codex").unwrap();
        let bytes = serde_json::to_vec(&draft).unwrap();
        // Another process submitted the id between a check and the write.
        assert!(place(&mut s, &key, &bytes, 1_000).unwrap());
        s.ack(&key).unwrap();
        assert!(
            !place(&mut s, &key, &bytes, 1_001).unwrap(),
            "closed, not an error"
        );
        let id = store_draft(&mut s, &key, &draft, 1_002).unwrap();
        assert_eq!(id, copy_key(&key, 1));
    }
    #[test]
    fn dropped_items_are_counted_and_reported_with_a_note() {
        let short = OK.replace(
            r#""open_questions":[]"#,
            r#""open_questions":["yes","Is the cap per call?"]"#,
        );
        let (_, record) = build(&short, "backend", 7, "h", "x").unwrap();
        assert_eq!(dropped(&short, &record), 1);
        assert_eq!(dropped(OK, &record), 0, "nothing given, nothing dropped");
        let dir = std::path::Path::new("/tmp/wk state");
        let out = result("draft:abc", dir, 1);
        assert_eq!(out["dropped"], 1);
        assert!(out["note"]
            .as_str()
            .unwrap()
            .contains("1 short list items were dropped"));
        let none = result("draft:abc", dir, 0);
        assert_eq!(none["dropped"], 0);
        assert!(none.get("note").is_none());
        let next = none["next"].as_str().unwrap();
        assert!(
            next.contains("--state-dir '/tmp/wk state' drafts"),
            "{next}"
        );
        assert!(next.contains("submit --id draft:abc --reviewed"));
    }
    fn settled(
        draft: &Value,
        hash: &str,
        revision: &Value,
        owner_only: bool,
    ) -> (Result<Readiness, String>, bool) {
        let (_dir, mut s) = store();
        let (key, _) = build(OK, "backend", 7, "hash1", "codex").unwrap();
        s.enqueue(&key, &serde_json::to_vec(draft).unwrap(), 1_000, 3_600)
            .unwrap();
        let out = settle_assistant(&mut s, &key, draft, hash, revision, owner_only);
        (out, s.contains(&key, 1_001).unwrap())
    }
    #[test]
    fn settling_consumes_only_a_real_policy_mismatch() {
        let (_, draft) = build(OK, "backend", 7, "hash1", "codex").unwrap();
        assert_eq!(
            settled(&draft, "hash1", &json!(7), false),
            (Ok(Readiness::Ready), true)
        );
        assert_eq!(
            settled(&draft, "other", &json!(7), false),
            (Ok(Readiness::Consume), false)
        );
        assert_eq!(
            settled(&draft, "hash1", &json!(8), false),
            (Ok(Readiness::Consume), false)
        );
        // A malformed policy reply never consumes a valid draft.
        for bad in [json!(null), json!("7"), json!(-1), json!(7.5)] {
            let (out, live) = settled(&draft, "other", &bad, false);
            assert_eq!(out, Err("invalid policy".into()));
            assert!(live, "{bad}");
        }
        // Sensitive content holds without consuming.
        let mut sensitive = draft.clone();
        sensitive["card"]["rationale"] = json!(["pass", "word"].concat());
        assert_eq!(
            settled(&sensitive, "hash1", &json!(7), false),
            (Ok(Readiness::Hold), true)
        );
        // Owner-only sync changes nothing.
        assert_eq!(
            settled(&draft, "other", &json!(7), true),
            (Ok(Readiness::Consume), true)
        );
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
