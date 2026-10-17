//! End to end with `summarizer: rules`: every kind of draft is made and listed, and no model is contacted.
use crate::assistant_draft::{self, Readiness};
use crate::process::rules_test::{rules_config, Model};
use crate::rules_draft::MARK;
use crate::{chat, daemon, memory_import, process, state};
use serde_json::{json, Value};

/// Filler that is neither sensitive nor structured, `n` bytes or a little more.
fn filler(n: usize) -> String {
    "Keep the limiter in the gateway. ".repeat(n / 33 + 1)
}

/// Unique words that must never reach the listing: each sits where a rule draft does not quote.
const BEYOND_FIRST_PROMPT: [&str; 2] = ["zebracanaryprompta", "zebracanarypromptb"];
const BEYOND_LAST_REPLY: [&str; 2] = ["zebracanaryreplya", "zebracanaryreplyb"];
const MIDDLE_EXCHANGE: [&str; 2] = ["zebracanarymidprompt", "zebracanarymidreply"];
const BEYOND_NOTE: &str = "zebracanarynote";

#[tokio::test]
async fn rules_mode_drafts_lists_and_readies_everything_without_a_model() {
    let model = Model::start().await;
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("workspace");
    std::fs::create_dir_all(root.join("backend")).unwrap();
    let dir = state::private_dir(&base.join("state"), true).unwrap();
    let b = wk_scope::enrollment::Binding::approve(&root, "demo", vec!["backend".into()]).unwrap();
    let mut c = rules_config(state::Config {
        root: b.root.clone(),
        binding_file: dir.join("binding.json"),
        service: "backend".into(),
        server: "http://127.0.0.1:4318".into(),
        model: "synthetic-local".into(),
        inference: String::new(),
        summarizer: state::Summarizer::Rules,
        port: 4319,
        account: "workspace".into(),
        device_id: "test-device".into(),
        repository: None,
        device: None,
    });
    // A reachable endpoint that must stay unused: rules mode never calls it.
    c.inference = model.url.clone();
    state::save(&c.binding_file, &b).unwrap();
    state::save(&dir.join("agent.json"), &c).unwrap();
    let mut store =
        wk_local::LocalStore::open(&dir.join("journal.db"), [7; 32], 50_000_000).unwrap();
    let policy = json!({"policy_revision": 1});

    // (1) A multi-exchange export: two exchanges share the first part, the third gets its own.
    let prompt = |title: &str, canary: &str| format!("{title}\n{} {canary}", filler(600));
    let reply = |canary: &str| format!("{} {canary} {}", filler(2000), filler(70_000));
    let exchanges = [
        (
            prompt("Bound request work", BEYOND_FIRST_PROMPT[0]),
            format!("Use middleware {}", MIDDLE_EXCHANGE[1]),
        ),
        (
            format!("Where should limits live {}", MIDDLE_EXCHANGE[0]),
            reply(BEYOND_LAST_REPLY[0]),
        ),
        (
            prompt("Explain the limiter", BEYOND_FIRST_PROMPT[1]),
            reply(BEYOND_LAST_REPLY[1]),
        ),
    ];
    let messages = exchanges
        .iter()
        .enumerate()
        .flat_map(|(i, (p, r))| {
            [
                json!({"id": format!("u{i}"), "role": "user", "text": p}),
                json!({"id": format!("a{i}"), "role": "assistant", "text": r}),
            ]
        })
        .collect::<Vec<_>>();
    let export = json!({"schema_version": 1, "workspace": b.root, "product": "demo", "session_id": "s1", "messages": messages}).to_string();
    let imported = chat::import_into(&mut store, &b, &c.service, &export, "s1").unwrap();
    assert_eq!(imported["parts"], 2, "{imported}");

    // (2) One rule draft per part.
    let now = daemon::now().unwrap();
    let mut sessions = vec![];
    for id in store.next_session_ids(now).unwrap() {
        let bytes = store.read_record(&id, now).unwrap().unwrap();
        sessions.push((id, serde_json::from_slice::<Value>(&bytes).unwrap()));
    }
    assert_eq!(sessions.len(), 2);
    let mut part_drafts = 0;
    for (id, session) in &sessions {
        let (made, held) = process::journal_turns(&dir, &c, &b, &policy, &mut store, id, session)
            .await
            .unwrap();
        assert_eq!((made, held), (1, 0), "{id}");
        part_drafts += made;
    }
    assert_eq!(part_drafts, 2);

    // (3) A memory note becomes a rule draft too.
    let note = format!(
        "## Retries\nReuse the idempotency key.\n{} {BEYOND_NOTE}\n",
        filler(2500)
    );
    std::fs::write(b.root.join("backend/CLAUDE.md"), note).unwrap();
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    memory_import::import_with(&dir, &b, &c.service, &mut store, Some(&home), &[], now).unwrap();
    let (made, held) = memory_import::draft_all(&dir, &c, &b, &policy, &mut store)
        .await
        .unwrap();
    assert_eq!((made, held), (1, 0));

    // (4) An assistant-written card.
    let card = r#"{"title":"Cap retries at 3","intent":"Avoid repeated charges","rationale":"The gateway charges again on duplicate calls; an idempotency key plus a cap keeps retries safe.","decisions":["Retries capped at 3"],"constraints":["Gateway repeats charges"],"open_questions":[]}"#;
    let (key, record) =
        assistant_draft::build(card, &c.service, 1, &b.policy_hash, "claude-code").unwrap();
    let assistant_id = assistant_draft::store_draft(&mut store, &key, &record, now).unwrap();

    // (5) The listing holds all four drafts and quotes nothing beyond the card fields.
    let listing = process::list_from(&store, "").unwrap();
    let rows = listing["private_drafts"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "{listing}");
    let assistant: Vec<_> = rows
        .iter()
        .filter(|r| r.get("assistant").is_some())
        .collect();
    assert_eq!(assistant.len(), 1);
    assert_eq!(assistant[0]["id"], assistant_id.as_str());
    assert_eq!(assistant[0]["assistant"]["client"], "claude-code");
    assert_eq!(assistant[0]["model"], "assistant");
    let rules: Vec<_> = rows
        .iter()
        .filter(|r| r.get("assistant").is_none())
        .collect();
    assert_eq!(rules.len(), 3);
    for row in &rules {
        let title = row["card"]["title"].as_str().unwrap();
        assert!(title.starts_with(MARK), "{title}");
        assert_eq!(row["card"]["capture_completeness"], "partial", "{row}");
    }
    assert_eq!(
        rules.iter().filter(|r| r.get("memory").is_some()).count(),
        1,
        "exactly one rule row is the memory note"
    );
    // Only the leading clip of the first prompt and last reply is quoted; nothing else.
    let text = listing.to_string();
    for canary in BEYOND_FIRST_PROMPT
        .iter()
        .chain(&BEYOND_LAST_REPLY)
        .chain(&MIDDLE_EXCHANGE)
        .chain(&[BEYOND_NOTE])
    {
        assert!(!text.contains(canary), "{canary} leaked into the listing");
    }
    assert!(text.contains("Bound request work") && text.contains("Explain the limiter"));

    // (6) The fresh assistant draft is ready; a changed policy hash consumes it.
    let stored: Value =
        serde_json::from_slice(&store.read_record(&assistant_id, now).unwrap().unwrap()).unwrap();
    assert_eq!(
        assistant_draft::assistant_ready(&stored, &b.policy_hash, 1),
        Readiness::Ready
    );
    assert_eq!(
        assistant_draft::assistant_ready(&stored, "another-policy-hash", 1),
        Readiness::Consume
    );

    // (7) No model was ever contacted.
    assert_eq!(model.hits(), 0, "rules mode never contacts the model");
}
