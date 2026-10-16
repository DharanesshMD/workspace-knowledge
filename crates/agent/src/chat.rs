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
/// Records a validated export as one journal session and turn per part, so `process`
/// drafts one summary per part. Each part has its own session to stay within the
/// journal's per-session bounds; event IDs and sequences are stable across re-imports.
fn import_into(
    store: &mut wk_local::LocalStore,
    b: &wk_scope::enrollment::Binding,
    service: &str,
    export: &str,
    session: &str,
) -> Result<Value, String> {
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
    let mut ids = std::collections::BTreeSet::new();
    for m in &e.messages {
        kind(&m.role)?;
        if !ids.insert(&m.id) {
            return Err("duplicate export message ID".into());
        }
    }
    let parts = parts(&e.messages, 96 * 1024);
    let (mut seq, mut truncated_parts) = (0, 0);
    for (n, part) in parts.iter().enumerate() {
        let turn = format!("part-{}", n + 1);
        let journal = format!("export:{session}:{turn}");
        let mut truncated = false;
        for m in part {
            truncated |= m.text != e.messages[seq].text;
            seq += 1;
            record(
                store,
                b,
                service,
                Record {
                    session: &journal,
                    turn: &turn,
                    id: &m.id,
                    kind: kind(&m.role)?,
                    text: &m.text,
                    seq: seq as u32,
                    agent: "explicit-export",
                },
            )?;
        }
        truncated_parts += usize::from(truncated);
    }
    Ok(
        json!({"captured_messages":seq,"parts":parts.len(),"truncated_parts":truncated_parts,"coverage":"partial_export","published":false}),
    )
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
/// Group messages into parts of whole exchanges (a user prompt and the replies after it)
/// whose combined text stays under `limit` bytes; an oversized exchange is its own part,
/// truncated at a char boundary with " [truncated]".
pub(crate) fn parts(messages: &[Message], limit: usize) -> Vec<Vec<Message>> {
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
    for e in exchanges {
        if !part.is_empty()
            && (text_len(&part) + text_len(e) > limit || part.len() + e.len() > PART_MESSAGES)
        {
            out.push(std::mem::take(&mut part));
        }
        if text_len(e) > limit {
            out.push(truncate(e, limit));
        } else {
            part.extend_from_slice(e);
        }
    }
    if !part.is_empty() {
        out.push(part);
    }
    out
}
// Every message keeps a fair share of the limit: messages under the share stay whole and
// longer ones are cut to it (marker included), so prompts and replies all reach the summary.
fn truncate(exchange: &[Message], limit: usize) -> Vec<Message> {
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
    exchange
        .iter()
        .map(|m| {
            let mut m = m.clone();
            if m.text.len() > cap {
                let end = m
                    .text
                    .floor_char_boundary(cap.saturating_sub(TRUNCATED.len()));
                m.text.truncate(end);
                m.text.push_str(TRUNCATED);
            }
            m
        })
        .collect()
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
    fn ids(parts: &[Vec<Message>]) -> Vec<Vec<&str>> {
        parts
            .iter()
            .map(|p| p.iter().map(|m| m.id.as_str()).collect())
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
        assert!(p.iter().all(|p| text_len(p) <= 14));
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
        let big = &p[1];
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
        assert_eq!(p[0], s[..2]);
        assert_eq!(p[2], s[4..]);
    }
    #[test]
    fn every_message_of_an_oversized_exchange_keeps_some_text() {
        let s = [
            m("1", "user", &"p".repeat(500)),
            m("2", "assistant", &"r".repeat(500)),
        ];
        let p = parts(&s, 200);
        assert_eq!(ids(&p), [vec!["1", "2"]]);
        assert!(p[0]
            .iter()
            .all(|m| m.text.len() > " [truncated]".len() && m.text.ends_with(" [truncated]")));
        assert!(text_len(&p[0]) <= 200);
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
    async fn drafts(f: &mut Fixture) -> usize {
        let mut made = 0;
        for id in f.store.next_session_ids(daemon::now().unwrap()).unwrap() {
            let bytes = f
                .store
                .read_record(&id, daemon::now().unwrap())
                .unwrap()
                .unwrap();
            let session: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(session["held"], false);
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
        let r = import_into(&mut f.store, &f.b, &f.c.service, &small, "s1").unwrap();
        assert_eq!(
            (
                r["captured_messages"].as_u64(),
                r["parts"].as_u64(),
                r["truncated_parts"].as_u64()
            ),
            (Some(6), Some(1), Some(0))
        );
        // Re-importing the same export is consistent and does not hold the session.
        import_into(&mut f.store, &f.b, &f.c.service, &small, "s1").unwrap();
        assert_eq!(drafts(&mut f).await, 1);
        {
            let sources = f.sources.lock().unwrap();
            assert_eq!(sources.len(), 1);
            for text in [
                "Bound request work",
                "Use middleware",
                "What about retries",
                "Retry with idempotency keys",
            ] {
                assert!(sources[0].contains(text), "draft input contains {text}");
            }
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
        let r = import_into(&mut f.store, &f.b, &f.c.service, &large, "s1").unwrap();
        assert_eq!(
            (
                r["captured_messages"].as_u64(),
                r["parts"].as_u64(),
                r["truncated_parts"].as_u64()
            ),
            (Some(4), Some(2), Some(1))
        );
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
        let r = import_into(&mut f.store, &f.b, &f.c.service, &big, "s1").unwrap();
        assert_eq!(
            (r["parts"].as_u64(), r["truncated_parts"].as_u64()),
            (Some(9), Some(0))
        );
        let ids = f.store.next_session_ids(daemon::now().unwrap()).unwrap();
        assert_eq!(ids.len(), 9);
        for id in ids {
            let bytes = f
                .store
                .read_record(&id, daemon::now().unwrap())
                .unwrap()
                .unwrap();
            let session: Value = serde_json::from_slice(&bytes).unwrap();
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
    #[test]
    fn invalid_exports_are_rejected_before_anything_is_recorded() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut f = rt.block_on(fixture());
        let base = export(&f, &[("q", "a".into())]);
        let mut duplicate: Value = serde_json::from_str(&base).unwrap();
        duplicate["messages"][1]["id"] = json!("u0");
        let mut role: Value = serde_json::from_str(&base).unwrap();
        role["messages"][1]["role"] = json!("system");
        let mut many: Value = serde_json::from_str(&base).unwrap();
        many["messages"] = json!((0..5001)
            .map(|i| json!({"id":i.to_string(),"role":"user","text":"q"}))
            .collect::<Vec<_>>());
        for bad in [duplicate, role, many] {
            assert!(import_into(&mut f.store, &f.b, "backend", &bad.to_string(), "s1").is_err());
        }
        assert!(import_into(&mut f.store, &f.b, "backend", &base, "other").is_err());
        assert!(f
            .store
            .next_session_ids(daemon::now().unwrap())
            .unwrap()
            .is_empty());
        f.server.abort();
    }
}
