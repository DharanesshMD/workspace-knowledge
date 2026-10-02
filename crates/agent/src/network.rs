use crate::state::Config;
use serde_json::Value;
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
pub async fn policy(
    c: &Config,
    b: &wk_scope::enrollment::Binding,
    token: &str,
) -> Result<Value, String> {
    let r = client(3)?
        .get(format!("{}/v1/policy", server_url(&c.server)?))
        .query(&[("product", &b.product)])
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "membership service unavailable")?;
    let v = wk_summarize::bounded_json(r, 65536).await?;
    if !v["policy"]["services"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s == &c.service))
    {
        return Err("service membership denied".into());
    }
    Ok(v["policy"].clone())
}
pub async fn post(c: &Config, token: &str, path: &str, body: &Value) -> Result<Value, String> {
    let r = client(10)?
        .post(format!("{}{path}", server_url(&c.server)?))
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .map_err(|_| "knowledge service unavailable")?;
    wk_summarize::bounded_json(r, 1024 * 1024).await
}
