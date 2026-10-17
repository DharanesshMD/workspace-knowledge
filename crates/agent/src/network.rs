use crate::state::Config;
use serde_json::Value;
pub const AUTH_DENIED: &str = "collector_authorization_denied";
pub fn client(seconds: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(seconds))
        .build()
        .map_err(|_| "network unavailable".into())
}
pub fn server_url(value: &str) -> Result<String, String> {
    let u = reqwest::Url::parse(value).map_err(|_| "invalid server URL")?;
    if (u.scheme() != "https"
        && !(u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))))
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || u.path() != "/"
    {
        return Err("server requires HTTPS or literal loopback HTTP origin".into());
    }
    Ok(value.trim_end_matches('/').into())
}
pub fn inference_allowed(endpoint: &str, restrictions: &Value) -> Result<(), String> {
    let url = reqwest::Url::parse(endpoint).map_err(|_| "invalid inference endpoint")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid inference endpoint".into());
    }
    let local =
        url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
    let allowed = local && restrictions["allow_loopback"] == true
        || url.scheme() == "https"
            && restrictions["https_origins"]
                .as_array()
                .is_some_and(|origins| {
                    origins.iter().any(|origin| {
                        origin.as_str() == Some(url.origin().ascii_serialization().as_str())
                    })
                });
    if !allowed {
        return Err("inference destination denied by product policy".into());
    }
    Ok(())
}
fn check_inference(c: &Config, policy: &Value) -> Result<(), String> {
    if c.device.is_some() && policy.get("restrictions").is_none() {
        return Err("missing inference policy".into());
    }
    // Restrictions govern inference and embedding destinations. Rule drafts send nothing to a
    // model, so they do not apply, but a hosted policy must still be well formed (checked above).
    if c.summarizer == crate::state::Summarizer::Rules {
        return Ok(());
    }
    let restrictions = policy
        .get("restrictions")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"allow_loopback":true,"https_origins":[]}));
    inference_allowed(&c.inference, &restrictions)
}
pub async fn policy(
    c: &Config,
    b: &wk_scope::enrollment::Binding,
    token: &str,
) -> Result<Value, String> {
    crate::device::require_device_for_remote(&c.server, c.device.is_some())?;
    if c.device.is_some() {
        let value = crate::device::signed(c, token, "policy", &serde_json::json!({}), 3).await?;
        if value["device"]["settings"]["paused"] == true {
            return Err("remote capture pause requested".into());
        }
        if !value["policy"]["services"]
            .as_array()
            .is_some_and(|v| v.iter().any(|s| s == &c.service))
        {
            return Err(AUTH_DENIED.into());
        }
        check_inference(c, &value["policy"])?;
        return Ok(value["policy"].clone());
    }
    let r = client(3)?
        .get(format!("{}/v1/policy", server_url(&c.server)?))
        .query(&[("product", &b.product)])
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "membership service unavailable")?;
    if matches!(r.status().as_u16(), 401 | 403) {
        return Err(AUTH_DENIED.into());
    }
    let v = wk_summarize::bounded_json(r, 65536).await?;
    if !v["policy"]["services"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s == &c.service))
    {
        return Err(AUTH_DENIED.into());
    }
    check_inference(c, &v["policy"])?;
    Ok(v["policy"].clone())
}
pub async fn post(c: &Config, token: &str, path: &str, body: &Value) -> Result<Value, String> {
    crate::device::require_device_for_remote(&c.server, c.device.is_some())?;
    if c.device.is_some() {
        let b = crate::state::base_binding(c)?;
        let operation = if path == "/v1/cards" {
            "cards"
        } else if path == format!("/v1/products/{}/personal", b.product) {
            "personal"
        } else {
            return Err("unsupported collector operation".into());
        };
        return crate::device::signed(c, token, operation, body, 10).await;
    }
    let r = client(10)?
        .post(format!("{}{path}", server_url(&c.server)?))
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .map_err(|_| "knowledge service unavailable")?;
    wk_summarize::bounded_json(r, 1024 * 1024).await
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    #[test]
    fn inference_policy_restricts_approved_destination_without_rewriting_it() {
        let local = "http://127.0.0.1:11434/v1/chat/completions";
        assert!(inference_allowed(
            local,
            &serde_json::json!({"allow_loopback":true,"https_origins":[]})
        )
        .is_ok());
        assert!(inference_allowed(
            local,
            &serde_json::json!({"allow_loopback":false,"https_origins":[]})
        )
        .is_err());
        let url = "https://model.synthetic.example/v1/chat/completions";
        assert!(inference_allowed(
            url,
            &serde_json::json!({"allow_loopback":true,"https_origins":[]})
        )
        .is_err());
        assert!(inference_allowed(url,&serde_json::json!({"allow_loopback":false,"https_origins":["https://model.synthetic.example"]})).is_ok());
        assert!(inference_allowed("https://model.synthetic.example.evil.test/v1",&serde_json::json!({"allow_loopback":false,"https_origins":["https://model.synthetic.example"]})).is_err());
        assert!(inference_allowed("https://user:secret@model.synthetic.example/v1",&serde_json::json!({"allow_loopback":false,"https_origins":["https://model.synthetic.example"]})).is_err());
        assert!(inference_allowed(url, &serde_json::Value::Null).is_err());
    }
    fn config(server: &str, summarizer: crate::state::Summarizer, inference: &str) -> Config {
        Config {
            root: "/w".into(),
            binding_file: "/b.json".into(),
            service: "backend".into(),
            server: server.into(),
            model: String::new(),
            inference: inference.into(),
            summarizer,
            port: 4319,
            account: "workspace".into(),
            device_id: "d".into(),
            device: None,
            repository: None,
        }
    }
    #[test]
    fn rules_configs_have_no_inference_to_check() {
        use crate::state::Summarizer::{Local, Rules};
        let strict = serde_json::json!({"allow_loopback":false,"https_origins":[]});
        let mut rules = config("http://127.0.0.1:1", Rules, "");
        assert!(check_inference(&rules, &strict).is_ok());
        assert!(check_inference(&rules, &serde_json::json!({})).is_ok());
        // Rule drafts ignore the restrictions, but a hosted policy must still carry them.
        rules.device = Some(crate::device::Connection {
            state_dir: "/s".into(),
        });
        assert_eq!(
            check_inference(&rules, &serde_json::json!({})).unwrap_err(),
            "missing inference policy"
        );
        assert!(check_inference(&rules, &serde_json::json!({"restrictions":{}})).is_ok());
        assert!(
            check_inference(&rules, &serde_json::json!({"restrictions":strict.clone()})).is_ok()
        );
        let local = config("http://127.0.0.1:1", Local, "");
        assert!(check_inference(&local, &serde_json::json!({})).is_err());
        let mut hosted_local = config(
            "http://127.0.0.1:1",
            Local,
            "http://127.0.0.1:11434/v1/chat/completions",
        );
        hosted_local.device = rules.device.clone();
        assert!(check_inference(&hosted_local, &serde_json::json!({})).is_err());
        assert!(check_inference(
            &hosted_local,
            &serde_json::json!({"restrictions":{"allow_loopback":false,"https_origins":[]}})
        )
        .is_err());
    }
    #[tokio::test]
    async fn server_policy_succeeds_for_rules_and_still_checks_local_inference() {
        use crate::state::Summarizer::{Local, Rules};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/v1/policy",
            axum::routing::get(|| async {
                axum::Json(
                    serde_json::json!({"policy":{"services":["backend"],"policy_revision":1}}),
                )
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("http://{address}");
        let b = wk_scope::enrollment::Binding {
            root: "/tmp".into(),
            product: "demo".into(),
            services: vec!["backend".into()],
            enabled: true,
            policy_hash: "h".into(),
        };
        let rules = config(&url, Rules, "");
        let policy = super::policy(&rules, &b, "token").await.unwrap();
        assert_eq!(policy["policy_revision"], 1);
        let bad = config(&url, Local, "");
        assert!(super::policy(&bad, &b, "token").await.is_err());
        let good = config(&url, Local, "http://127.0.0.1:11434/v1/chat/completions");
        assert!(super::policy(&good, &b, "token").await.is_ok());
        server.abort();
    }
}
