//! Explicit local opt-in to publish the developer's own assistant-written cards on `draft`,
//! without the separate `submit --reviewed` step. Off by default; every other draft kind still
//! needs review. The server must also advertise `publish_on_submit` for the product.
use crate::{network, process, state};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

const FILE: &str = "auto-publish.json";
/// Server policy feature set when the operator enabled publish on submit for this product.
const FEATURE: &str = "publish_on_submit";
/// A server that does not accept `provenance` cannot publish an assistant card at all.
const PROVENANCE: &str = "card_provenance";

pub(crate) const MEANING: &str = "your assistant-written cards from `wk-agent draft` will be published to the team WITHOUT a review step, after the usual sensitive-content screening; you can withdraw any card afterwards; rule drafts, hook captures, memory imports and owner sync are never published this way; it only works if the server operator enabled it for this product";

pub(crate) const WITHDRAW: &str = "wk-agent has no withdraw command; withdraw the card in the portal (My context, open the card, Withdraw knowledge) or with POST /v1/cards/{id}/withdraw";

/// The consent, bound to the approval and destination it was given for.
#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Choice {
    enabled: bool,
    policy_hash: String,
    server: String,
    product: String,
    service: String,
}

fn read(dir: &Path) -> Result<Choice, String> {
    let path = dir.join(FILE);
    if !path.exists() {
        return Ok(Choice::default());
    }
    serde_json::from_str(&state::read(&path, 65536)?)
        .map_err(|_| "invalid auto-publish choice".into())
}

fn matches(choice: &Choice, c: &state::Config, b: &wk_scope::enrollment::Binding) -> bool {
    choice.enabled
        && choice.policy_hash == b.policy_hash
        && choice.server == c.server
        && choice.product == b.product
        && choice.service == c.service
}

/// Consent is current: given for this approval, server, product and service.
pub(crate) fn enabled(dir: &Path, c: &state::Config, b: &wk_scope::enrollment::Binding) -> bool {
    read(dir).is_ok_and(|choice| matches(&choice, c, b))
}

/// The server accepts the request and can publish an assistant card.
pub(crate) fn offered(policy: &Value) -> bool {
    let has = |name: &str| {
        policy["features"]
            .as_array()
            .is_some_and(|f| f.iter().any(|x| x == name))
    };
    has(FEATURE) && has(PROVENANCE)
}

/// `wk-agent --state-dir DIR` as the developer would type it.
fn invocation(dir: &Path) -> String {
    format!(
        "wk-agent --state-dir {}",
        crate::assistant_draft::quote(&dir.to_string_lossy())
    )
}

/// What the local choice says. A choice made under an earlier approval or server is `stale`.
/// With no choice file at all (a collector set up before this default existed) it is off, and says how to turn it on.
fn local_status(dir: &Path) -> Result<Value, String> {
    let choice = read(dir)?;
    let current = state::load(dir)
        .ok()
        .and_then(|c| state::binding(&c).ok().map(|b| matches(&choice, &c, &b)))
        .unwrap_or(false);
    let mut out = json!({"enabled":choice.enabled && current,"stale":choice.enabled && !current,"scope":"assistant-written cards from wk-agent draft only"});
    if !dir.join(FILE).exists() {
        out["how"] = json!(format!("{} auto-publish enable --yes", invocation(dir)));
    }
    Ok(out)
}

/// The default for a NEW collector, written by `device connect` and `init` for the approval just
/// accepted: on, unless the developer passed `--no-auto-publish`. Existing collectors are never
/// touched. It only takes effect where the server operator also enabled it for the product, so
/// a server that does not offer it leaves the choice dormant. Returns the output field that tells
/// the developer what this means and how to turn it off.
pub(crate) fn initial(
    dir: &Path,
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    opted_out: bool,
) -> Result<Value, String> {
    let path = dir.join(FILE);
    let how_to_enable = format!("{} auto-publish enable --yes", invocation(dir));
    if opted_out {
        // A leftover choice from an interrupted earlier attempt must not survive an explicit no.
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err("state unavailable".into())
            }
            _ => {}
        }
        return Ok(json!({"enabled":false,"how":how_to_enable}));
    }
    state::save(
        &path,
        &Choice {
            enabled: true,
            policy_hash: b.policy_hash.clone(),
            server: c.server.clone(),
            product: b.product.clone(),
            service: c.service.clone(),
        },
    )?;
    Ok(json!({
        "enabled":true,
        "scope":"assistant-written cards from wk-agent draft only",
        "meaning":MEANING,
        "turn_off":format!("{} auto-publish disable", invocation(dir)),
        "dormant_until":"the server operator enables publish on submit for this product; until then every card stays a private draft you review and submit"
    }))
}

/// Best effort and offline safe: `null` when the server cannot be asked.
async fn server_offers(dir: &Path) -> Value {
    let ask = async {
        let c = state::load(dir).ok()?;
        let b = state::binding(&c).ok()?;
        let t = state::token(dir, "server-token").ok()?;
        network::policy(&c, &b, &t).await.ok()
    };
    ask.await.map_or(Value::Null, |p| json!(offered(&p)))
}

/// `auto-publish enable --yes | disable | status`.
pub(crate) async fn command(dir: &Path, action: &str, yes: bool) -> Result<Value, String> {
    match action {
        "status" => {
            let mut out = local_status(dir)?;
            out["server_offers_it"] = server_offers(dir).await;
            Ok(out)
        }
        "disable" => {
            state::save(&dir.join(FILE), &Choice::default())?;
            local_status(dir)
        }
        "enable" => {
            if !yes {
                return Err(format!(
                    "auto-publish is off; if you enable it, {MEANING}; review that, then pass --yes"
                ));
            }
            let c = state::load(dir)?;
            let b = state::binding(&c)?;
            state::save(
                &dir.join(FILE),
                &Choice {
                    enabled: true,
                    policy_hash: b.policy_hash,
                    server: c.server,
                    product: b.product,
                    service: c.service,
                },
            )?;
            let mut out = local_status(dir)?;
            out["meaning"] = json!(MEANING);
            out["server_offers_it"] = server_offers(dir).await;
            Ok(out)
        }
        _ => Err("auto-publish action must be status, enable or disable".into()),
    }
}

/// Try to publish a freshly stored assistant draft. `None`: auto-publish is off, so nothing changes.
/// `Some(Ok(card id))`: published, and the local draft is consumed. `Some(Err(reason))`: the draft
/// is exactly as the manual flow leaves it, and the developer reviews and submits it as usual.
/// Runs once; it never retries.
pub(crate) async fn attempt(
    dir: &Path,
    c: &state::Config,
    b: &wk_scope::enrollment::Binding,
    token: &str,
    policy: &Value,
    store: &mut wk_local::LocalStore,
    id: &str,
) -> Option<Result<Value, String>> {
    if !enabled(dir, c, b) {
        return None;
    }
    if !offered(policy) {
        return Some(Err("this server does not offer publish on submit for this product, so nothing was published".into()));
    }
    if let Err(e) = process::check_gap(dir) {
        return Some(Err(format!("nothing was published ({e})")));
    }
    let response =
        match process::deliver_loaded(dir, c, b, token, policy, store, id, false, true).await {
            Ok(response) => response,
            Err(e) => return Some(Err(format!("nothing was published ({e})"))),
        };
    if response["card"]["published"] == true {
        Some(Ok(response["card"]["id"].clone()))
    } else {
        Some(Err(
            "the server kept the card as an ordinary review copy and did not publish it".into(),
        ))
    }
}

/// Turn the stored-draft result of `draft` into the published result.
pub(crate) fn mark_published(out: &mut Value, card_id: Value) {
    if let Some(o) = out.as_object_mut() {
        o.remove("draft_id");
        o.remove("next");
        o.insert("private".into(), json!(false));
        o.insert("published".into(), json!(true));
        o.insert("card_id".into(), card_id);
        o.insert("withdraw".into(), json!(WITHDRAW));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant_draft;
    use std::sync::{Arc, Mutex};

    const OK: &str = r#"{"title":"Cap retries at 3","intent":"Avoid repeated charges","rationale":"The gateway charges again on duplicate calls; an idempotency key plus a cap keeps retries safe.","decisions":["Retries capped at 3"],"constraints":["Gateway repeats charges"],"open_questions":[]}"#;
    const ID_LEN: usize = 70;

    /// A loopback server that records every `/v1/cards` body and answers with `published`.
    struct Cards {
        origin: String,
        bodies: Arc<Mutex<Vec<Value>>>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Cards {
        async fn start(answer: Option<bool>) -> Self {
            let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
            let seen = bodies.clone();
            let app = axum::Router::new().route(
                "/v1/cards",
                axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                    let seen = seen.clone();
                    async move {
                        let wants = body.get("publish_on_submit") == Some(&json!(true));
                        seen.lock().unwrap().push(body);
                        match answer {
                            Some(published) => (
                                axum::http::StatusCode::OK,
                                axum::Json(
                                    json!({"card":{"id":"c1","published":published && wants}}),
                                ),
                            ),
                            None => (
                                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                                axum::Json(json!({"error":"unavailable"})),
                            ),
                        }
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            Self {
                origin,
                bodies,
                task,
            }
        }
        fn bodies(&self) -> Vec<Value> {
            self.bodies.lock().unwrap().clone()
        }
    }
    impl Drop for Cards {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        dir: std::path::PathBuf,
        c: state::Config,
        b: wk_scope::enrollment::Binding,
        store: wk_local::LocalStore,
    }
    fn fixture(origin: &str) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("workspace");
        std::fs::create_dir_all(root.join("backend")).unwrap();
        let dir = state::private_dir(&base.join("state"), true).unwrap();
        let b =
            wk_scope::enrollment::Binding::approve(&root, "demo", vec!["backend".into()]).unwrap();
        let c = crate::process::rules_test::rules_config(state::Config {
            root: b.root.clone(),
            binding_file: dir.join("binding.json"),
            service: "backend".into(),
            server: origin.into(),
            model: String::new(),
            inference: String::new(),
            summarizer: state::Summarizer::Rules,
            port: 4319,
            account: "workspace".into(),
            device_id: "test-device".into(),
            repository: None,
            device: None,
        });
        state::save(&c.binding_file, &b).unwrap();
        state::save(&dir.join("agent.json"), &c).unwrap();
        let store = wk_local::LocalStore::open(&dir.join("t.db"), [7; 32], 50_000_000).unwrap();
        Fixture {
            _temp: temp,
            dir,
            c,
            b,
            store,
        }
    }
    fn policy(features: &[&str]) -> Value {
        json!({"policy_revision":1,"services":["backend"],"features":features})
    }
    const BOTH: [&str; 2] = ["card_provenance", "publish_on_submit"];
    /// An assistant draft is stored like `wk-agent draft` stores it.
    fn assistant(f: &mut Fixture, text: &str) -> String {
        let (key, record) =
            assistant_draft::build(text, "backend", 1, &f.b.policy_hash, "codex").unwrap();
        assistant_draft::store_draft(&mut f.store, &key, &record, crate::daemon::now().unwrap())
            .unwrap()
    }
    fn pending(f: &Fixture) -> usize {
        f.store
            .pending_ids("draft:", "", crate::daemon::now().unwrap(), 100)
            .unwrap()
            .len()
    }
    async fn try_publish(
        f: &mut Fixture,
        policy: &Value,
        id: &str,
    ) -> Option<Result<Value, String>> {
        attempt(&f.dir, &f.c, &f.b, "token", policy, &mut f.store, id).await
    }
    async fn enable(f: &Fixture) {
        command(&f.dir, "enable", true).await.unwrap();
    }

    #[tokio::test]
    async fn enabled_and_offered_sends_the_flag_and_consumes_the_draft() {
        let server = Cards::start(Some(true)).await;
        let mut f = fixture(&server.origin);
        enable(&f).await;
        let id = assistant(&mut f, OK);
        assert_eq!(id.len(), ID_LEN);
        let outcome = try_publish(&mut f, &policy(&BOTH), &id).await;
        assert_eq!(outcome, Some(Ok(json!("c1"))));
        let bodies = server.bodies();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["publish_on_submit"], true);
        assert_eq!(bodies[0]["product"], "demo");
        assert_eq!(bodies[0]["card"]["provenance"], "assistant_written");
        assert_eq!(
            pending(&f),
            0,
            "published, so the private draft is consumed"
        );
        let mut out =
            json!({"draft_id":id,"private":true,"published":false,"dropped":0,"next":"submit"});
        mark_published(&mut out, json!("c1"));
        assert_eq!(out["published"], true);
        assert_eq!(out["card_id"], "c1");
        assert!(out.get("next").is_none() && out.get("draft_id").is_none());
        assert!(out["withdraw"]
            .as_str()
            .unwrap()
            .contains("Withdraw knowledge"));
    }

    #[tokio::test]
    async fn without_the_server_feature_nothing_is_sent_and_the_draft_stays() {
        let server = Cards::start(Some(true)).await;
        let mut f = fixture(&server.origin);
        enable(&f).await;
        let id = assistant(&mut f, OK);
        for features in [
            &["card_provenance"][..],
            &["publish_on_submit"][..],
            &[][..],
        ] {
            let outcome = try_publish(&mut f, &policy(features), &id).await.unwrap();
            let reason = outcome.unwrap_err();
            assert!(
                reason.contains("does not offer publish on submit"),
                "{reason}"
            );
        }
        assert!(server.bodies().is_empty());
        assert_eq!(pending(&f), 1);
    }

    #[tokio::test]
    async fn disabled_changes_nothing_and_the_manual_submit_never_sends_the_flag() {
        let server = Cards::start(Some(true)).await;
        let mut f = fixture(&server.origin);
        let id = assistant(&mut f, OK);
        assert_eq!(try_publish(&mut f, &policy(&BOTH), &id).await, None);
        assert!(server.bodies().is_empty());
        // The manual path posts the same draft without the field, even if the server offers it.
        let response = process::deliver_loaded(
            &f.dir,
            &f.c,
            &f.b,
            "token",
            &policy(&BOTH),
            &mut f.store,
            &id,
            false,
            false,
        )
        .await
        .unwrap();
        assert_eq!(response["card"]["published"], false);
        let bodies = server.bodies();
        assert_eq!(bodies.len(), 1);
        assert!(
            bodies[0].get("publish_on_submit").is_none(),
            "{}",
            bodies[0]
        );
        assert_eq!(pending(&f), 0, "a manual submit still consumes the draft");
    }

    #[tokio::test]
    async fn a_server_that_does_not_publish_keeps_the_draft() {
        let server = Cards::start(Some(false)).await;
        let mut f = fixture(&server.origin);
        enable(&f).await;
        let id = assistant(&mut f, OK);
        let reason = try_publish(&mut f, &policy(&BOTH), &id)
            .await
            .unwrap()
            .unwrap_err();
        assert!(reason.contains("ordinary review copy"), "{reason}");
        assert_eq!(server.bodies().len(), 1, "one attempt, no retry");
        assert_eq!(pending(&f), 1);
    }

    #[tokio::test]
    async fn a_network_failure_keeps_the_draft_and_is_not_retried() {
        let server = Cards::start(None).await;
        let mut f = fixture(&server.origin);
        enable(&f).await;
        let id = assistant(&mut f, OK);
        let reason = try_publish(&mut f, &policy(&BOTH), &id)
            .await
            .unwrap()
            .unwrap_err();
        assert!(reason.starts_with("nothing was published"), "{reason}");
        assert_eq!(server.bodies().len(), 1);
        assert_eq!(pending(&f), 1);
    }

    #[tokio::test]
    async fn a_sensitive_draft_is_held_and_nothing_is_sent() {
        let server = Cards::start(Some(true)).await;
        let mut f = fixture(&server.origin);
        enable(&f).await;
        let id = assistant(&mut f, OK);
        // Edit the stored draft the way a screening rule would later reject it.
        let now = crate::daemon::now().unwrap();
        let mut record: Value =
            serde_json::from_slice(&f.store.read_record(&id, now).unwrap().unwrap()).unwrap();
        record["card"]["rationale"] = json!(["pass", "word"].concat());
        f.store
            .update(&id, now, assistant_draft::TTL, |_| {
                Ok(serde_json::to_vec(&record).unwrap())
            })
            .unwrap();
        let reason = try_publish(&mut f, &policy(&BOTH), &id)
            .await
            .unwrap()
            .unwrap_err();
        assert!(reason.contains("sensitive-content review"), "{reason}");
        assert!(server.bodies().is_empty());
        assert_eq!(pending(&f), 1);
    }

    #[tokio::test]
    async fn a_changed_approval_ends_the_consent_and_a_capture_gap_blocks_it() {
        let server = Cards::start(Some(true)).await;
        let mut f = fixture(&server.origin);
        enable(&f).await;
        let id = assistant(&mut f, OK);
        std::fs::write(f.dir.join("capture-gap.json"), "{\"gap\":true}").unwrap();
        let reason = try_publish(&mut f, &policy(&BOTH), &id)
            .await
            .unwrap()
            .unwrap_err();
        assert!(reason.contains("capture gap"), "{reason}");
        std::fs::remove_file(f.dir.join("capture-gap.json")).unwrap();
        let other = wk_scope::enrollment::Binding {
            root: f.b.root.clone(),
            product: f.b.product.clone(),
            services: f.b.services.clone(),
            enabled: true,
            policy_hash: "another-approval".into(),
        };
        let outcome = attempt(
            &f.dir,
            &f.c,
            &other,
            "token",
            &policy(&BOTH),
            &mut f.store,
            &id,
        )
        .await;
        assert_eq!(outcome, None, "consent was given for the earlier approval");
        let mut moved = f.c.clone();
        moved.server = "http://127.0.0.1:1".into();
        let outcome = attempt(
            &f.dir,
            &moved,
            &f.b,
            "token",
            &policy(&BOTH),
            &mut f.store,
            &id,
        )
        .await;
        assert_eq!(outcome, None, "consent was given for another server");
        assert!(server.bodies().is_empty());
        assert_eq!(pending(&f), 1);
    }

    #[tokio::test]
    async fn only_assistant_drafts_can_ask_to_publish() {
        let server = Cards::start(Some(true)).await;
        let mut f = fixture(&server.origin);
        enable(&f).await;
        let now = crate::daemon::now().unwrap();
        let (_, assistant_record) =
            assistant_draft::build(OK, "backend", 1, &f.b.policy_hash, "codex").unwrap();
        let card = assistant_record["card"].clone();
        let kinds = [
            (
                "rule",
                json!({"card":card,"source_id":"s","turn_key":"t","policy_hash":f.b.policy_hash,"model":"rules"}),
            ),
            (
                "memory",
                json!({"card":card,"memory":{"path":"CLAUDE.md"},"policy_hash":f.b.policy_hash}),
            ),
            (
                "native",
                json!({"card":card,"native_adapter":"claude-code","policy_hash":f.b.policy_hash}),
            ),
            (
                "cursor",
                json!({"card":card,"native_turn_index":0,"source_id":"s","policy_hash":f.b.policy_hash}),
            ),
            (
                "assistant+memory",
                json!({"card":card,"assistant":{"client":"x"},"memory":{"path":"x"},"policy_hash":f.b.policy_hash}),
            ),
        ];
        for (n, (kind, record)) in kinds.iter().enumerate() {
            let id = format!("draft:{:064x}", n + 1);
            f.store
                .update(&id, now, 3600, |_| Ok(serde_json::to_vec(record).unwrap()))
                .unwrap();
            let err = process::deliver_loaded(
                &f.dir,
                &f.c,
                &f.b,
                "token",
                &policy(&BOTH),
                &mut f.store,
                &id,
                false,
                true,
            )
            .await
            .unwrap_err();
            assert_eq!(
                err, "only an assistant-written card is published on submit",
                "{kind}"
            );
        }
        // Owner sync never publishes either.
        let id = assistant(&mut f, OK);
        let err = process::deliver_loaded(
            &f.dir,
            &f.c,
            &f.b,
            "token",
            &policy(&BOTH),
            &mut f.store,
            &id,
            true,
            true,
        )
        .await
        .unwrap_err();
        assert!(err.contains("only an assistant-written card"));
        assert!(server.bodies().is_empty());
        assert_eq!(pending(&f), 6, "nothing was consumed");
    }

    #[tokio::test]
    async fn rule_drafts_and_other_posts_stay_unflagged_with_auto_publish_on() {
        let server = Cards::start(Some(true)).await;
        let f = fixture(&server.origin);
        enable(&f).await;
        let (_, record) =
            assistant_draft::build(OK, "backend", 1, &f.b.policy_hash, "codex").unwrap();
        // `post_draft` is what rule drafts, hook captures, memory imports and Cursor turns use.
        let response = process::post_draft(&f.dir, &f.c, &f.b, "token", &record, false)
            .await
            .unwrap();
        assert_eq!(response["card"]["published"], false);
        let bodies = server.bodies();
        assert_eq!(bodies.len(), 1);
        assert!(
            bodies[0].get("publish_on_submit").is_none(),
            "{}",
            bodies[0]
        );
        assert_eq!(
            bodies[0].as_object().unwrap().len(),
            2,
            "product and card only"
        );
    }

    #[test]
    fn only_the_assistant_draft_flow_asks_to_publish() {
        // Pin the call sites so a new caller of the publishing path is noticed in review.
        assert_eq!(
            include_str!("process.rs").matches("post_draft_as(").count(),
            3
        );
        assert_eq!(
            include_str!("process.rs")
                .matches("deliver_loaded(")
                .count(),
            2
        );
        assert_eq!(
            include_str!("assistant_draft.rs")
                .matches("auto_publish::attempt(")
                .count(),
            1
        );
        for src in [
            include_str!("rules_draft.rs"),
            include_str!("memory_import.rs"),
            include_str!("personal_sync.rs"),
            include_str!("native_process.rs"),
            include_str!("chat.rs"),
            include_str!("daemon.rs"),
        ] {
            assert!(!src.contains("deliver_loaded") && !src.contains("publish_on_submit"));
        }
    }

    #[tokio::test]
    async fn enable_needs_yes_and_says_what_it_means() {
        let f = fixture("http://127.0.0.1:1");
        let err = command(&f.dir, "enable", false).await.unwrap_err();
        for words in [
            "WITHOUT a review step",
            "sensitive-content screening",
            "withdraw any card",
            "rule drafts, hook captures",
            "operator enabled it",
        ] {
            assert!(err.contains(words), "{words}: {err}");
        }
        assert!(!f.dir.join(FILE).exists(), "refused without --yes");
        assert!(command(&f.dir, "bogus", true).await.is_err());
        let out = command(&f.dir, "enable", true).await.unwrap();
        assert_eq!(out["enabled"], true);
        assert!(out["meaning"]
            .as_str()
            .unwrap()
            .contains("WITHOUT a review step"));
        // No credential is stored in this test, so the server's answer is unknown, not an error.
        assert!(out["server_offers_it"].is_null());
        assert_eq!(
            command(&f.dir, "status", false).await.unwrap()["enabled"],
            true
        );
        let off = command(&f.dir, "disable", false).await.unwrap();
        assert_eq!(off["enabled"], false);
        assert_eq!(
            command(&f.dir, "status", false).await.unwrap()["enabled"],
            false
        );
    }

    #[tokio::test]
    async fn a_stale_choice_is_reported_and_not_honored() {
        let f = fixture("http://127.0.0.1:1");
        command(&f.dir, "enable", true).await.unwrap();
        let mut moved = f.c.clone();
        moved.server = "http://127.0.0.1:2".into();
        state::save(&f.dir.join("agent.json"), &moved).unwrap();
        let out = local_status(&f.dir).unwrap();
        assert_eq!(
            (out["enabled"].clone(), out["stale"].clone()),
            (json!(false), json!(true))
        );
        assert!(!enabled(&f.dir, &moved, &f.b));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_choice_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let f = fixture("http://127.0.0.1:1");
        command(&f.dir, "enable", true).await.unwrap();
        let mode = std::fs::metadata(f.dir.join(FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        command(&f.dir, "disable", false).await.unwrap();
        let mode = std::fs::metadata(f.dir.join(FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[tokio::test]
    async fn a_new_collector_defaults_to_on_bound_to_its_approval() {
        let f = fixture("http://127.0.0.1:1");
        let out = initial(&f.dir, &f.c, &f.b, false).unwrap();
        assert_eq!(out["enabled"], true);
        assert_eq!(out["meaning"], MEANING);
        let off = out["turn_off"].as_str().unwrap();
        assert!(off.starts_with("wk-agent --state-dir "), "{off}");
        assert!(off.ends_with(" auto-publish disable"), "{off}");
        assert!(out["dormant_until"].as_str().unwrap().contains("operator"));
        // The same file and format `auto-publish enable --yes` writes, so every command agrees.
        let choice = read(&f.dir).unwrap();
        assert!(choice.enabled);
        assert_eq!(
            (
                choice.policy_hash.as_str(),
                choice.server.as_str(),
                choice.product.as_str(),
                choice.service.as_str()
            ),
            (
                f.b.policy_hash.as_str(),
                "http://127.0.0.1:1",
                "demo",
                "backend"
            )
        );
        assert!(enabled(&f.dir, &f.c, &f.b));
        assert_eq!(
            command(&f.dir, "status", false).await.unwrap()["enabled"],
            true
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(f.dir.join(FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Turning it off later is the ordinary command.
        command(&f.dir, "disable", false).await.unwrap();
        assert!(!enabled(&f.dir, &f.c, &f.b));
    }

    #[test]
    fn no_auto_publish_writes_no_choice_and_removes_a_leftover_one() {
        let f = fixture("http://127.0.0.1:1");
        let out = initial(&f.dir, &f.c, &f.b, true).unwrap();
        assert_eq!(out["enabled"], false);
        assert!(out["how"]
            .as_str()
            .unwrap()
            .ends_with(" auto-publish enable --yes"));
        assert!(out.get("meaning").is_none());
        assert!(!f.dir.join(FILE).exists());
        assert!(!enabled(&f.dir, &f.c, &f.b));
        // An interrupted earlier attempt left a default choice behind; an explicit no removes it.
        initial(&f.dir, &f.c, &f.b, false).unwrap();
        assert!(f.dir.join(FILE).exists());
        initial(&f.dir, &f.c, &f.b, true).unwrap();
        assert!(!f.dir.join(FILE).exists());
        assert!(!enabled(&f.dir, &f.c, &f.b));
    }

    #[tokio::test]
    async fn an_existing_collector_without_the_file_stays_off_and_says_how_to_enable_it() {
        // `fixture` is a collector set up before the default existed: no choice file.
        let server = Cards::start(Some(true)).await;
        let mut f = fixture(&server.origin);
        assert!(!f.dir.join(FILE).exists());
        let out = command(&f.dir, "status", false).await.unwrap();
        assert_eq!(out["enabled"], false);
        assert_eq!(out["stale"], false);
        let how = out["how"].as_str().unwrap();
        assert!(how.ends_with(" auto-publish enable --yes"), "{how}");
        assert!(how.starts_with("wk-agent --state-dir "), "{how}");
        // Reading, status and a draft that the server would publish change nothing on disk.
        let id = assistant(&mut f, OK);
        assert_eq!(try_publish(&mut f, &policy(&BOTH), &id).await, None);
        assert!(server.bodies().is_empty());
        assert!(!f.dir.join(FILE).exists(), "nothing migrated or enabled");
        // Once the developer has made any choice the hint is gone.
        command(&f.dir, "disable", false).await.unwrap();
        let out = command(&f.dir, "status", false).await.unwrap();
        assert!(out.get("how").is_none(), "{out}");
    }

    #[tokio::test]
    async fn the_default_choice_publishes_when_the_policy_offers_it() {
        let server = Cards::start(Some(true)).await;
        let mut f = fixture(&server.origin);
        initial(&f.dir, &f.c, &f.b, false).unwrap();
        let id = assistant(&mut f, OK);
        let outcome = try_publish(&mut f, &policy(&BOTH), &id).await;
        assert_eq!(outcome, Some(Ok(json!("c1"))));
        let bodies = server.bodies();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["publish_on_submit"], true);
        assert_eq!(pending(&f), 0);
    }

    #[tokio::test]
    async fn the_default_choice_stays_dormant_when_the_server_does_not_offer_it() {
        let server = Cards::start(Some(true)).await;
        let mut f = fixture(&server.origin);
        initial(&f.dir, &f.c, &f.b, false).unwrap();
        let id = assistant(&mut f, OK);
        for features in [
            &[][..],
            &["card_provenance"][..],
            &["publish_on_submit"][..],
        ] {
            let reason = try_publish(&mut f, &policy(features), &id)
                .await
                .unwrap()
                .unwrap_err();
            assert!(
                reason.contains("does not offer publish on submit"),
                "{reason}"
            );
        }
        assert!(server.bodies().is_empty(), "nothing is sent");
        assert_eq!(pending(&f), 1, "a normal private draft remains");
        // The choice itself is still on; it simply has nothing to act on.
        assert!(enabled(&f.dir, &f.c, &f.b));
    }

    #[test]
    fn the_feature_needs_both_names_and_a_list() {
        assert!(offered(&policy(&BOTH)));
        assert!(!offered(&json!({"policy_revision":1})));
        assert!(!offered(&json!({"features":"publish_on_submit"})));
    }
}
