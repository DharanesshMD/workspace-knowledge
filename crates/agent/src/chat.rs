use crate::{daemon, network, state};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
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
/// The journal event for `r`, as `ingest` receives it.
fn event_json(b: &wk_scope::enrollment::Binding, service: &str, r: &Record<'_>) -> String {
    let Record {
        session,
        turn,
        id,
        kind,
        text,
        seq,
        agent,
    } = r;
    json!({"schema_version":1,"event_id":id,"session_id":session,"agent_id":agent,"turn_id":turn,"sequence":seq,"kind":kind,"path":b.root.join(service),"text":text}).to_string()
}
/// The journal's per-event size limit, checked by `ingest`.
const MAX_EVENT_BYTES: usize = 256 * 1024;
pub fn record(
    store: &mut wk_local::LocalStore,
    b: &wk_scope::enrollment::Binding,
    service: &str,
    r: Record<'_>,
) -> Result<(), String> {
    let event = event_json(b, service, &r);
    let r = wk_capture::journal::ingest(store, b, &event, daemon::now()?)?;
    if r["held"] == true {
        return Err("session held; start a clean conversation".into());
    }
    Ok(())
}
/// Managed chat talks to the local model, so it needs the `local` summarizer.
fn require_local(c: &state::Config) -> Result<(), String> {
    if c.summarizer != state::Summarizer::Local {
        return Err(
            "managed chat needs a local model; set up with --summarizer local --inference URL"
                .into(),
        );
    }
    Ok(())
}
pub async fn run(dir: &Path, file: &Path, session: Option<String>) -> Result<Value, String> {
    let c = state::load(dir)?;
    require_local(&c)?;
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
    crate::process::processing_current(dir, &c, &b)?;
    network::policy(&c, &b, &token).await?;
    let url = wk_summarize::local_url(&c.inference)?;
    let response = network::client(60)?
        .post(url)
        .json(&json!({"model":c.model,"messages":messages,"stream":false,"max_tokens":4096}))
        .send()
        .await
        .map_err(|_| "local chat provider unavailable")?;
    let v = wk_summarize::bounded_json(response, 128 * 1024).await?;
    crate::process::processing_current(dir, &c, &b)?;
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
    let export = state::read(file, 8 * 1024 * 1024)?;
    import_into(&mut state::store(dir)?, &b, &c.service, &export, session)
}
// `ingest` bounds every identity to 256 bytes; part sessions add this suffix to the session ID.
const PART_SESSION_SUFFIX: usize = ":0123456789ab:part-5000".len();
/// Records a validated export as one journal session and turn per part, so `process`
/// drafts one summary per part. Each part has its own session to stay within the
/// journal's per-session bounds. Session names include a digest of the messages: an
/// identical re-import adds nothing, while a grown or edited export gets new sessions,
/// so earlier parts are never held by conflicting text. Event sequences are positions
/// in the export, stable only while the message prefix is unchanged.
pub(crate) fn import_into(
    store: &mut wk_local::LocalStore,
    b: &wk_scope::enrollment::Binding,
    service: &str,
    export: &str,
    session: &str,
) -> Result<Value, String> {
    let e = validated(b, export, session)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&e.messages).map_err(|_| "invalid scoped export")?)
    );
    let parts = parts(&e.messages, 96 * 1024);
    // Each message with the journal session and turn of its part; sequence numbers follow.
    let mut placed: Vec<(String, String, &Message)> = vec![];
    for (n, part) in parts.iter().enumerate() {
        let turn = format!("part-{}", n + 1);
        let journal = format!("export:{session}:{}:{turn}", &digest[..12]);
        placed.extend(
            part.messages
                .iter()
                .map(|m| (journal.clone(), turn.clone(), m)),
        );
    }
    let records = placed
        .iter()
        .zip(1..)
        .map(|((journal, turn, m), seq)| {
            Ok(Record {
                session: journal,
                turn,
                id: &m.id,
                kind: kind(&m.role)?,
                text: &m.text,
                seq,
                agent: "explicit-export",
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    // Text that JSON-escapes heavily (e.g. mostly control characters) can outgrow the journal's
    // per-event limit even within a part's text budget. Check every event before recording
    // any, so an import never records only some of its messages.
    if records
        .iter()
        .any(|r| event_json(b, service, r).len() > MAX_EVENT_BYTES)
    {
        return Err("export message too large to record".into());
    }
    let seq = records.len();
    for r in records {
        record(store, b, service, r)?;
    }
    let truncated_parts = parts.iter().filter(|p| p.truncated).count();
    // `process` drafts at most 10 journal sessions per run, one session per part.
    let process_runs_needed = parts.len().div_ceil(10);
    Ok(
        json!({"captured_messages":seq,"parts":parts.len(),"truncated_parts":truncated_parts,"process_runs_needed":process_runs_needed,"coverage":"partial_export","published":false}),
    )
}
/// Checks the export's shape and identities before anything is recorded. `import_into` adds a
/// check of every event's size, so an import never records only some of its messages.
fn validated(
    b: &wk_scope::enrollment::Binding,
    export: &str,
    session: &str,
) -> Result<Export, String> {
    let e: Export = serde_json::from_str(export).map_err(|_| "invalid scoped export")?;
    if e.schema_version != 1
        || e.session_id != session
        || e.product != b.product
        || Path::new(&e.workspace)
            .canonicalize()
            .map_err(|_| "export workspace unavailable")?
            != b.root
        || e.messages.len() > 5000
    {
        return Err("export binding mismatch".into());
    }
    if session.is_empty() || "export:".len() + session.len() + PART_SESSION_SUFFIX > 256 {
        return Err("export session ID too long".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for m in &e.messages {
        kind(&m.role)?;
        if m.id.is_empty() || m.id.len() > 256 || !ids.insert(&m.id) {
            return Err("empty, oversized or duplicate export message ID".into());
        }
    }
    Ok(e)
}
fn kind(role: &str) -> Result<&'static str, String> {
    match role {
        "user" => Ok("user_prompt"),
        "assistant" => Ok("assistant_response"),
        _ => Err("unsupported export role".into()),
    }
}
const TRUNCATED: &str = " [truncated]";
// Keeps a part's journal session and summarizer input well inside their event and size bounds.
const PART_MESSAGES: usize = 256;
fn text_len(messages: &[Message]) -> usize {
    messages.iter().map(|m| m.text.len()).sum()
}
#[derive(Debug, PartialEq)]
pub(crate) struct Part {
    pub messages: Vec<Message>,
    pub truncated: bool,
}
/// Group messages into parts of whole exchanges (a user prompt and the replies after it)
/// whose combined text stays under `limit` bytes and that hold at most `PART_MESSAGES`
/// messages. An exchange with more messages is split into consecutive chunks first; an
/// exchange (or chunk) over `limit` is its own part, truncated at a char boundary with
/// " [truncated]".
pub(crate) fn parts(messages: &[Message], limit: usize) -> Vec<Part> {
    let mut exchanges = vec![];
    let (mut start, mut prompted) = (0, false);
    for (i, m) in messages.iter().enumerate() {
        if m.role == "user" {
            if prompted {
                exchanges.push(&messages[start..i]);
                start = i;
            }
            prompted = true;
        }
    }
    if start < messages.len() {
        exchanges.push(&messages[start..]);
    }
    let mut out = vec![];
    let mut part: Vec<Message> = vec![];
    for e in exchanges.into_iter().flat_map(|e| e.chunks(PART_MESSAGES)) {
        if !part.is_empty()
            && (text_len(&part) + text_len(e) > limit || part.len() + e.len() > PART_MESSAGES)
        {
            out.push(Part {
                messages: std::mem::take(&mut part),
                truncated: false,
            });
        }
        if text_len(e) > limit {
            out.push(truncate(e, limit));
        } else {
            part.extend_from_slice(e);
        }
    }
    if !part.is_empty() {
        out.push(Part {
            messages: part,
            truncated: false,
        });
    }
    out
}
// Every message keeps a fair share of the limit: messages under the share stay whole and
// longer ones are cut to it (marker included), so prompts and replies all reach the summary.
fn truncate(exchange: &[Message], limit: usize) -> Part {
    let mut lens: Vec<usize> = exchange.iter().map(|m| m.text.len()).collect();
    lens.sort_unstable();
    let (mut left, mut cap) = (limit, limit);
    for (i, len) in lens.iter().enumerate() {
        let share = left / (lens.len() - i);
        if *len > share {
            cap = share;
            break;
        }
        left -= len;
    }
    let mut truncated = false;
    let messages = exchange
        .iter()
        .map(|m| {
            let mut m = m.clone();
            if m.text.len() > cap {
                let end = m
                    .text
                    .floor_char_boundary(cap.saturating_sub(TRUNCATED.len()));
                m.text.truncate(end);
                m.text.push_str(TRUNCATED);
                truncated = true;
            }
            m
        })
        .collect();
    Part {
        messages,
        truncated,
    }
}

#[cfg(test)]
mod part_tests {
    use super::*;
    fn m(id: &str, role: &str, text: &str) -> Message {
        Message {
            id: id.into(),
            role: role.into(),
            text: text.into(),
        }
    }
    fn ids(parts: &[Part]) -> Vec<Vec<&str>> {
        parts
            .iter()
            .map(|p| p.messages.iter().map(|m| m.id.as_str()).collect())
            .collect()
    }
    #[test]
    fn small_session_is_one_part() {
        let s = [
            m("1", "user", "a"),
            m("2", "assistant", "b"),
            m("3", "user", "c"),
            m("4", "assistant", "d"),
        ];
        assert_eq!(ids(&parts(&s, 100)), [vec!["1", "2", "3", "4"]]);
    }
    #[test]
    fn parts_break_only_between_whole_exchanges() {
        let s = [
            m("1", "user", "aaaa"),
            m("2", "assistant", "bbbb"),
            m("3", "assistant", "cc"),
            m("4", "user", "dddd"),
            m("5", "assistant", "eeee"),
            m("6", "user", "ff"),
            m("7", "assistant", "gg"),
        ];
        let p = parts(&s, 14);
        assert_eq!(
            ids(&p),
            [vec!["1", "2", "3"], vec!["4", "5", "6", "7"]],
            "an exchange is never split across parts"
        );
        assert!(p
            .iter()
            .all(|p| text_len(&p.messages) <= 14 && !p.truncated));
        assert_eq!(p, parts(&s, 14), "grouping is deterministic");
    }
    #[test]
    fn leading_replies_stay_with_the_first_exchange() {
        let s = [
            m("1", "assistant", "hello"),
            m("2", "user", "q"),
            m("3", "assistant", "a"),
            m("4", "user", "q2"),
            m("5", "assistant", "a2"),
        ];
        assert_eq!(ids(&parts(&s, 7)), [vec!["1", "2", "3"], vec!["4", "5"]]);
    }
    #[test]
    fn oversized_exchange_is_its_own_truncated_part() {
        let s = [
            m("1", "user", "q"),
            m("2", "assistant", "a"),
            m("3", "user", "short prompt"),
            m("4", "assistant", &"é".repeat(200)),
            m("5", "user", "next"),
            m("6", "assistant", "ok"),
        ];
        let limit = 100;
        let p = parts(&s, limit);
        assert_eq!(ids(&p), [vec!["1", "2"], vec!["3", "4"], vec!["5", "6"]]);
        assert_eq!(
            p.iter().map(|p| p.truncated).collect::<Vec<_>>(),
            [false, true, false]
        );
        let big = &p[1].messages;
        assert_eq!(
            big[0].text, "short prompt",
            "messages that fit are kept whole"
        );
        assert!(big[1].text.ends_with(" [truncated]"));
        assert!(big[1].text.starts_with("éé"));
        assert!(
            text_len(big) <= limit,
            "truncated part stays within the limit"
        );
        assert_eq!(p[0].messages, s[..2]);
        assert_eq!(p[2].messages, s[4..]);
    }
    #[test]
    fn every_message_of_an_oversized_exchange_keeps_some_text() {
        let s = [
            m("1", "user", &"p".repeat(500)),
            m("2", "assistant", &"r".repeat(500)),
        ];
        let p = parts(&s, 200);
        assert_eq!(ids(&p), [vec!["1", "2"]]);
        assert!(p[0].truncated);
        assert!(p[0]
            .messages
            .iter()
            .all(|m| m.text.len() > " [truncated]".len() && m.text.ends_with(" [truncated]")));
        assert!(text_len(&p[0].messages) <= 200);
    }
    #[test]
    fn long_exchange_is_split_by_message_count() {
        let s = std::iter::once(m("u", "user", "q"))
            .chain((0..299).map(|i| m(&format!("a{i}"), "assistant", "a")))
            .collect::<Vec<_>>();
        let p = parts(&s, 96 * 1024);
        assert!(p.len() >= 2);
        assert!(p.iter().all(|p| p.messages.len() <= PART_MESSAGES));
        assert_eq!(
            p.iter().flat_map(|p| &p.messages).collect::<Vec<_>>(),
            s.iter().collect::<Vec<_>>()
        );
    }
    #[test]
    fn replies_without_prompts_are_split_by_message_count() {
        let s = (0..600)
            .map(|i| m(&i.to_string(), "assistant", "a"))
            .collect::<Vec<_>>();
        let p = parts(&s, 96 * 1024);
        assert!(p.len() >= 3);
        assert!(p.iter().all(|p| p.messages.len() <= PART_MESSAGES));
    }
    #[test]
    fn empty_session_has_no_parts() {
        assert!(parts(&[], 10).is_empty());
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Fixture {
        _temp: tempfile::TempDir,
        dir: std::path::PathBuf,
        c: state::Config,
        b: wk_scope::enrollment::Binding,
        store: wk_local::LocalStore,
        sources: Arc<Mutex<Vec<String>>>,
        server: tokio::task::JoinHandle<()>,
    }
    async fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir_all(root.join("backend")).unwrap();
        let dir = state::private_dir(&temp.path().join("state"), true).unwrap();
        let b =
            wk_scope::enrollment::Binding::approve(&root, "demo", vec!["backend".into()]).unwrap();
        let sources = Arc::new(Mutex::new(vec![]));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/inference",
            axum::routing::post({
                let sources = sources.clone();
                move |axum::Json(request): axum::Json<Value>| {
                    let sources = sources.clone();
                    async move {
                        let source = request["messages"][1]["content"].as_str().unwrap();
                        sources.lock().unwrap().push(source.to_string());
                        let content = json!({"title":"Limiter","intent":"Bound request work","rationale":"Middleware","decisions":[],"constraints":[],"open_questions":[],"sensitive":false});
                        axum::Json(json!({"choices":[{"message":{"content":content.to_string()}}]}))
                    }
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let c = state::Config {
            root: b.root.clone(),
            binding_file: dir.join("binding.json"),
            service: "backend".into(),
            server: "http://127.0.0.1:4318".into(),
            model: "synthetic-local".into(),
            inference: format!("http://{address}/inference"),
            summarizer: crate::state::Summarizer::Local,
            port: 4319,
            account: "workspace".into(),
            device_id: uuid::Uuid::new_v4().to_string(),
            repository: None,
            device: None,
        };
        state::save(&c.binding_file, &b).unwrap();
        state::save(&dir.join("agent.json"), &c).unwrap();
        let store =
            wk_local::LocalStore::open(&dir.join("journal.db"), [7; 32], 10_000_000).unwrap();
        Fixture {
            _temp: temp,
            dir,
            c,
            b,
            store,
            sources,
            server,
        }
    }
    fn export(f: &Fixture, exchanges: &[(&str, String)]) -> String {
        let messages = exchanges
            .iter()
            .enumerate()
            .flat_map(|(i, (prompt, reply))| {
                [
                    json!({"id":format!("u{i}"),"role":"user","text":prompt}),
                    json!({"id":format!("a{i}"),"role":"assistant","text":reply}),
                ]
            })
            .collect::<Vec<_>>();
        json!({"schema_version":1,"workspace":f.b.root,"product":"demo","session_id":"s1","messages":messages}).to_string()
    }
    #[tokio::test]
    async fn managed_chat_needs_the_local_summarizer() {
        let mut f = fixture().await;
        assert!(require_local(&f.c).is_ok());
        f.c.summarizer = state::Summarizer::Rules;
        state::save(&f.dir.join("agent.json"), &f.c).unwrap();
        let prompt = f.dir.join("prompt.txt");
        std::fs::write(&prompt, "hello").unwrap();
        let e = run(&f.dir, &prompt, None).await.unwrap_err();
        assert!(e.contains("managed chat needs a local model"), "{e}");
        f.server.abort();
    }
    fn import(f: &mut Fixture, export: &str) -> Result<Value, String> {
        import_into(&mut f.store, &f.b, &f.c.service, export, "s1")
    }
    fn counts(r: &Value) -> (Option<u64>, Option<u64>, Option<u64>) {
        (
            r["captured_messages"].as_u64(),
            r["parts"].as_u64(),
            r["truncated_parts"].as_u64(),
        )
    }
    /// Every live journal session, scanning past the store's 10-per-call cursor.
    fn sessions(f: &mut Fixture) -> std::collections::BTreeMap<String, Value> {
        let mut all = std::collections::BTreeMap::new();
        loop {
            let before = all.len();
            for id in f.store.next_session_ids(daemon::now().unwrap()).unwrap() {
                let bytes = f
                    .store
                    .read_record(&id, daemon::now().unwrap())
                    .unwrap()
                    .unwrap();
                all.entry(id)
                    .or_insert_with(|| serde_json::from_slice(&bytes).unwrap());
            }
            if all.len() == before {
                return all;
            }
        }
    }
    /// Drafts every session, as repeated `process` runs would; no session may be held.
    async fn drafts(f: &mut Fixture) -> usize {
        let mut made = 0;
        for (id, session) in sessions(f) {
            assert_eq!(session["held"], false, "{id} held");
            let (ok, failed) = crate::process::journal_turns(
                &f.dir,
                &f.c,
                &f.b,
                &json!({"policy_revision":1}),
                &mut f.store,
                &id,
                &session,
            )
            .await
            .unwrap();
            assert_eq!(failed, 0);
            made += ok;
        }
        made
    }
    #[tokio::test]
    async fn one_draft_per_part_contains_prompts_and_replies() {
        let mut f = fixture().await;
        let small = export(
            &f,
            &[
                ("Bound request work", "Use middleware".into()),
                ("Where should limits live", "In the gateway".into()),
                ("What about retries", "Retry with idempotency keys".into()),
            ],
        );
        let r = import(&mut f, &small).unwrap();
        assert_eq!(counts(&r), (Some(6), Some(1), Some(0)));
        assert_eq!(r["process_runs_needed"], 1);
        assert_eq!(drafts(&mut f).await, 1);
        let sources = f.sources.lock().unwrap().clone();
        assert_eq!(sources.len(), 1);
        for text in [
            "Bound request work",
            "Use middleware",
            "What about retries",
            "Retry with idempotency keys",
        ] {
            assert!(sources[0].contains(text), "draft input contains {text}");
        }
        f.server.abort();
    }
    #[tokio::test]
    async fn large_export_drafts_each_part_and_marks_truncation() {
        let mut f = fixture().await;
        let long = "Use middleware to bound request work in the gateway. ";
        let large = export(
            &f,
            &[
                ("Bound request work", long.repeat(1200)),
                ("Explain the limiter", long.repeat(2000)),
            ],
        );
        let r = import(&mut f, &large).unwrap();
        assert_eq!(counts(&r), (Some(4), Some(2), Some(1)));
        assert_eq!(drafts(&mut f).await, 2);
        let mut sources = f.sources.lock().unwrap().clone();
        sources.sort_by_key(|s| s.contains("Explain the limiter"));
        assert!(sources[0].contains("Bound request work") && !sources[0].contains("[truncated]"));
        assert!(sources[1].contains("Explain the limiter") && sources[1].contains(" [truncated]"));
        assert!(sources
            .iter()
            .all(|s| s.contains("assistant_response: Use middleware")));
        f.server.abort();
    }
    #[tokio::test]
    async fn export_beyond_one_journal_session_is_recorded_without_hold() {
        let mut f = fixture().await;
        let reply = "Keep the limiter in the gateway. ".repeat(2900);
        let exchanges = (0..9)
            .map(|_| ("Bound request work", reply.clone()))
            .collect::<Vec<_>>();
        let big = export(&f, &exchanges);
        assert!(
            big.len() > 768 * 1024,
            "larger than one journal session holds"
        );
        let r = import(&mut f, &big).unwrap();
        assert_eq!(counts(&r), (Some(18), Some(9), Some(0)));
        let all = sessions(&mut f);
        assert_eq!(all.len(), 9);
        for session in all.values() {
            assert_eq!(
                (
                    session["held"].as_bool(),
                    session["events"].as_array().map(Vec::len)
                ),
                (Some(false), Some(2))
            );
        }
        f.server.abort();
    }
    #[tokio::test]
    async fn replies_without_prompts_are_drafted_in_bounded_parts() {
        let mut f = fixture().await;
        let messages = (0..600)
            .map(|i| json!({"id":format!("a{i}"),"role":"assistant","text":"Use middleware"}))
            .collect::<Vec<_>>();
        let only = json!({"schema_version":1,"workspace":f.b.root,"product":"demo","session_id":"s1","messages":messages}).to_string();
        let r = import(&mut f, &only).unwrap();
        let parts = r["parts"].as_u64().unwrap();
        assert!(parts >= 3);
        assert!(sessions(&mut f)
            .values()
            .all(|s| s["events"].as_array().unwrap().len() <= PART_MESSAGES));
        assert_eq!(drafts(&mut f).await as u64, parts, "no part is held");
        f.server.abort();
    }
    #[tokio::test]
    async fn changed_export_gets_new_sessions_and_identical_reimport_is_a_no_op() {
        let mut f = fixture().await;
        let mut exchanges = vec![
            ("Bound request work", "Use middleware".to_string()),
            ("Where should limits live", "In the gateway".to_string()),
        ];
        let first = export(&f, &exchanges);
        import(&mut f, &first).unwrap();
        assert_eq!(drafts(&mut f).await, 1);
        // The session continued: the grown export now ends with an oversized, truncated exchange.
        exchanges.push((
            "Explain the limiter",
            "Use middleware to bound request work in the gateway. ".repeat(2000),
        ));
        let grown = export(&f, &exchanges);
        let r = import(&mut f, &grown).unwrap();
        assert_eq!(counts(&r), (Some(6), Some(2), Some(1)));
        assert_eq!(
            drafts(&mut f).await,
            2,
            "earlier sessions are untouched; new parts draft"
        );
        let before = sessions(&mut f);
        assert_eq!(before.len(), 3);
        import(&mut f, &grown).unwrap();
        assert_eq!(
            sessions(&mut f),
            before,
            "identical re-import adds no events"
        );
        assert_eq!(drafts(&mut f).await, 0);
        f.server.abort();
    }
    #[tokio::test]
    async fn more_than_ten_parts_need_several_process_runs() {
        let mut f = fixture().await;
        let reply = "Keep the limiter in the gateway. ".repeat(2900);
        let exchanges = (0..11)
            .map(|_| ("Bound request work", reply.clone()))
            .collect::<Vec<_>>();
        let big = export(&f, &exchanges);
        let r = import(&mut f, &big).unwrap();
        assert_eq!(counts(&r), (Some(22), Some(11), Some(0)));
        assert_eq!(r["process_runs_needed"], 2);
        assert_eq!(drafts(&mut f).await, 11);
        f.server.abort();
    }
    #[tokio::test]
    async fn maximum_message_count_is_accepted() {
        let f = fixture().await;
        let max = |n: usize| {
            let messages = (0..n)
                .map(|i| json!({"id":i.to_string(),"role":"user","text":"q"}))
                .collect::<Vec<_>>();
            json!({"schema_version":1,"workspace":f.b.root,"product":"demo","session_id":"s1","messages":messages}).to_string()
        };
        assert_eq!(
            validated(&f.b, &max(5000), "s1").unwrap().messages.len(),
            5000
        );
        assert!(validated(&f.b, &max(5001), "s1").is_err());
        f.server.abort();
    }
    #[tokio::test]
    async fn invalid_exports_are_rejected_before_anything_is_recorded() {
        let mut f = fixture().await;
        let base = export(&f, &[("q", "a".into()), ("q2", "a2".into())]);
        let with = |change: &dyn Fn(&mut Value)| {
            let mut v: Value = serde_json::from_str(&base).unwrap();
            change(&mut v);
            v.to_string()
        };
        let bad = [
            with(&|v| v["messages"][3]["id"] = json!("u0")),
            with(&|v| v["messages"][3]["id"] = json!("")),
            with(&|v| v["messages"][3]["id"] = json!("x".repeat(257))),
            with(&|v| v["messages"][3]["role"] = json!("system")),
            with(&|v| {
                v["messages"] = json!((0..5001)
                    .map(|i| json!({"id":i.to_string(),"role":"user","text":"q"}))
                    .collect::<Vec<_>>())
            }),
        ];
        for bad in bad {
            assert!(import(&mut f, &bad).is_err());
        }
        assert!(import_into(&mut f.store, &f.b, "backend", &base, "other").is_err());
        let long = "s".repeat(256);
        let err = import_into(
            &mut f.store,
            &f.b,
            "backend",
            &with(&|v| v["session_id"] = json!(long)),
            &long,
        )
        .unwrap_err();
        assert!(err.contains("session ID"), "{err}");
        assert!(sessions(&mut f).is_empty());
        f.server.abort();
    }
    #[tokio::test]
    async fn an_event_over_the_journal_limit_is_rejected_before_anything_is_recorded() {
        let mut f = fixture().await;
        // 90 KiB of control characters fits a part but escapes to 6 bytes each in the event.
        let heavy = "\u{1}".repeat(90 * 1024);
        let ok = export(&f, &[("q", "a".into()), ("q2", heavy)]);
        let err = import(&mut f, &ok).unwrap_err();
        assert!(err.contains("too large to record"), "{err}");
        assert!(
            sessions(&mut f).is_empty(),
            "earlier messages must not be recorded"
        );
        // The same text in a smaller message is fine.
        let fine = export(&f, &[("q", "a".into()), ("q2", "\u{1}".repeat(10 * 1024))]);
        assert_eq!(import(&mut f, &fine).unwrap()["captured_messages"], 4);
        f.server.abort();
    }
}
