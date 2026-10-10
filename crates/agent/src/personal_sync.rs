//! Explicit local opt-in to synchronize complete sanitized drafts to the owner store.
use crate::{daemon, network, process, state};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Choice {
    enabled: bool,
    policy_hash: String,
    server: String,
    product: String,
    service: String,
    device_id: String,
    model: String,
    inference: String,
    policy_revision: u64,
}
fn read(dir: &Path) -> Result<Choice, String> {
    let path = dir.join("owner-sync.json");
    if !path.exists() {
        return Ok(Choice::default());
    }
    serde_json::from_str(&state::read(&path, 65536)?)
        .map_err(|_| "invalid owner sync choice".into())
}
fn matches(choice: &Choice, c: &state::Config, b: &wk_scope::enrollment::Binding) -> bool {
    choice.enabled
        && choice.policy_hash == b.policy_hash
        && choice.server == c.server
        && choice.product == b.product
        && choice.service == c.service
        && choice.device_id == c.device_id
        && choice.model == c.model
        && choice.inference == c.inference
}
pub fn status(dir: &Path) -> Result<Value, String> {
    let choice = read(dir)?;
    let enabled = choice.enabled
        && state::load(dir)
            .ok()
            .and_then(|c| state::binding(&c).ok().map(|b| matches(&choice, &c, &b)))
            .unwrap_or(false);
    Ok(json!({"enabled":enabled,"remote_policy":"not_checked","automatic_publication":false}))
}
pub async fn configure(dir: &Path, action: &str, yes: bool) -> Result<Value, String> {
    match action {
        "status" => status(dir),
        "disable" => {
            state::save(&dir.join("owner-sync.json"), &Choice::default())?;
            status(dir)
        }
        "enable" => {
            if !yes {
                return Err("owner-only hosted summary synchronization is off; review its visibility and retention, then pass --yes to enable".into());
            }
            let c = state::load(dir)?;
            let b = state::binding(&c)?;
            let token = state::token(dir, "server-token")?;
            let policy = network::policy(&c, &b, &token).await?;
            state::save(
                &dir.join("owner-sync.json"),
                &Choice {
                    enabled: true,
                    policy_hash: b.policy_hash,
                    server: c.server,
                    product: b.product,
                    service: c.service,
                    device_id: c.device_id,
                    model: c.model,
                    inference: c.inference,
                    policy_revision: policy["policy_revision"]
                        .as_u64()
                        .ok_or("invalid membership policy")?,
                },
            )?;
            status(dir)
        }
        _ => Err("owner-sync action must be status, enable or disable".into()),
    }
}
pub fn permits(dir: &Path, hash: &str, revision: u64) -> Result<bool, String> {
    let choice = read(dir)?;
    let c = state::load(dir)?;
    let b = state::binding(&c)?;
    Ok(
        matches(&choice, &c, &b)
            && choice.policy_hash == hash
            && choice.policy_revision == revision,
    )
}
pub fn payload(card: &Value) -> Result<Value, String> {
    let typed: wk_contracts::CardSubmission =
        serde_json::from_value(card.clone()).map_err(|_| "invalid sanitized summary")?;
    typed.validate().map_err(|_| "invalid sanitized summary")?;
    if !matches!(
        typed.capture_completeness,
        wk_contracts::Completeness::Complete
    ) || wk_summarize::sensitive(&card.to_string())
    {
        return Err("ineligible sanitized summary".into());
    }
    Ok(json!({"body":typed,"provenance":"captured"}))
}
pub async fn send(
    c: &state::Config,
    product: &str,
    token: &str,
    card: &Value,
) -> Result<Value, String> {
    if !wk_contracts::valid_id(product) {
        return Err("invalid product".into());
    }
    let response = network::post(
        c,
        token,
        &format!("/v1/products/{product}/personal"),
        &payload(card)?,
    )
    .await?;
    if !response["summary"]["id"].is_string() {
        return Err("invalid summary acknowledgment".into());
    }
    Ok(response)
}
fn marker(id: &str) -> String {
    format!("owner-sync:{id}")
}
fn mark_success(store: &mut wk_local::LocalStore, id: &str, now: i64) -> Result<(), String> {
    let key = marker(id);
    store.update(&key, now, 1, |_| Ok(Vec::new()))?;
    store.ack(&key)
}
pub async fn run(dir: &Path) -> Result<Value, String> {
    if status(dir)?["enabled"] != true {
        return Ok(json!({"enabled":false,"uploaded":0,"held":0}));
    }
    let mut store = state::store(dir)?;
    let ids = store.next_ids("owner-sync", "draft:", daemon::now()?, 50)?;
    let mut budget = process::InferenceBudget::new(50, std::time::Duration::from_secs(20));
    let mut uploaded = 0;
    let mut held = 0;
    for id in ids {
        if store.seen(&marker(&id))? {
            continue;
        }
        let Some(result) = budget.attempt(process::sync_owner(dir, &id)).await else {
            break;
        };
        match result {
            Ok(_) => {
                mark_success(&mut store, &id, daemon::now()?)?;
                uploaded += 1;
            }
            Err(_) => held += 1,
        }
    }
    Ok(json!({"enabled":true,"uploaded":uploaded,"held":held,"automatic_publication":false}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn sync_is_disabled_without_credentials_or_approval_and_enable_needs_consent() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(status(temp.path()).unwrap()["enabled"], false);
        assert_eq!(run(temp.path()).await.unwrap()["uploaded"], 0);
        assert!(configure(temp.path(), "enable", false).await.is_err());
    }
    #[test]
    fn captured_payload_is_strict_complete_and_screened() {
        assert!(payload(&json!({"capture_completeness":"partial"})).is_err());
        let card = json!({"schema_version":1,"idempotency_key":"sync-one","policy_revision":1,"title":"Tenant limiter","intent":"Prevent overload","rationale":"Shared middleware","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null});
        let sent = payload(&card).unwrap();
        assert_eq!(sent["provenance"], "captured");
        assert_eq!(sent["body"], card);
        let mut unknown = card.clone();
        unknown["messages"] = json!(["RAW_CANARY"]);
        assert!(payload(&unknown).is_err());
        let mut secret = card;
        secret["intent"] = json!("sk-proj-SYNTHETIC_SECRET_CANARY");
        assert!(payload(&secret).is_err());
    }
    fn fixture(
        server: &str,
    ) -> (
        tempfile::TempDir,
        state::Config,
        wk_scope::enrollment::Binding,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("backend")).unwrap();
        let binding =
            wk_scope::enrollment::Binding::approve(&root, "demo", vec!["backend".into()]).unwrap();
        let c = state::Config {
            root: binding.root.clone(),
            binding_file: temp.path().join("binding.json"),
            service: "backend".into(),
            server: server.into(),
            model: "synthetic".into(),
            inference: "http://127.0.0.1:11434/v1/chat/completions".into(),
            port: 0,
            account: "test".into(),
            device_id: "test-device".into(),
        };
        state::save(&c.binding_file, &binding).unwrap();
        state::save(&temp.path().join("agent.json"), &c).unwrap();
        (temp, c, binding)
    }
    fn card() -> Value {
        json!({"schema_version":1,"idempotency_key":"sync-test","policy_revision":1,"title":"Limiter","intent":"Prevent overload","rationale":"Shared middleware","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})
    }
    #[test]
    fn consent_is_invalidated_by_scope_destination_model_or_remote_policy_changes() {
        let (temp, mut c, b) = fixture("http://127.0.0.1:4318");
        let choice = Choice {
            enabled: true,
            policy_hash: b.policy_hash.clone(),
            server: c.server.clone(),
            product: b.product.clone(),
            service: c.service.clone(),
            device_id: c.device_id.clone(),
            model: c.model.clone(),
            inference: c.inference.clone(),
            policy_revision: 1,
        };
        state::save(&temp.path().join("owner-sync.json"), &choice).unwrap();
        assert_eq!(status(temp.path()).unwrap()["enabled"], true);
        assert!(permits(temp.path(), &b.policy_hash, 1).unwrap());
        assert!(!permits(temp.path(), &b.policy_hash, 2).unwrap());
        assert!(!permits(temp.path(), "changed-hash", 1).unwrap());
        c.server = "https://another.example".into();
        state::save(&temp.path().join("agent.json"), &c).unwrap();
        assert_eq!(status(temp.path()).unwrap()["enabled"], false);
        c.server = choice.server.clone();
        c.model = "changed-model".into();
        state::save(&temp.path().join("agent.json"), &c).unwrap();
        assert_eq!(status(temp.path()).unwrap()["enabled"], false);
    }
    #[test]
    fn successful_sync_marker_survives_restart_without_extending_draft_retention() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("capture.db");
        let id = format!("draft:{}", "a".repeat(64));
        let key = [7; 32];
        let mut store = wk_local::LocalStore::open(&path, key, 1024 * 1024).unwrap();
        store
            .update(&id, 100, 10, |_| Ok(b"synthetic-summary".to_vec()))
            .unwrap();
        mark_success(&mut store, &id, 105).unwrap();
        assert!(store.read_record(&id, 106).unwrap().is_some());
        assert!(store.read_record(&id, 111).unwrap().is_none());
        drop(store);
        let store = wk_local::LocalStore::open(&path, key, 1024 * 1024).unwrap();
        assert!(store.seen(&marker(&id)).unwrap());
        assert!(store.read_record(&marker(&id), 106).unwrap().is_none());
    }
    #[tokio::test]
    async fn loopback_delivery_sends_only_summary_and_failed_upload_is_retryable() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let app = axum::Router::new().route(
            "/v1/products/demo/personal",
            axum::routing::post(
                move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                    let seen = seen.clone();
                    async move {
                        assert_eq!(headers["authorization"], "Bearer synthetic-token");
                        assert_eq!(body.as_object().unwrap().len(), 2);
                        assert_eq!(body["provenance"], "captured");
                        assert!(body.get("source_id").is_none());
                        if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                            return (
                                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                                axum::Json(json!({"error":"unavailable"})),
                            );
                        }
                        (
                            axum::http::StatusCode::OK,
                            axum::Json(json!({"summary":{"id":uuid::Uuid::new_v4().to_string()}})),
                        )
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (_temp, c, _b) = fixture(&origin);
        assert!(send(&c, "demo", "synthetic-token", &card()).await.is_err());
        assert!(send(&c, "demo", "synthetic-token", &card()).await.is_ok());
        assert_eq!(count.load(Ordering::SeqCst), 2);
        task.abort();
    }
}
