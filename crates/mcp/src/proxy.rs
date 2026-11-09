//! `wk-mcp proxy`: a stdio MCP server that forwards each message to one product's hosted MCP endpoint with a
//! personal token. It is for clients that can only start a local process (Claude Desktop): they get the same
//! twelve tools as a remote connection, with the token and any private CA handled here instead of in the client.
//!
//! The hosted endpoint is stateless (one JSON reply per POST), so there is no session to keep: every line read
//! from stdin is one JSON-RPC message, sent as one POST, and every message in the reply is written back as one
//! line. The token is read from the environment or a private file, never as an argument value,
//! and is never printed or written by the proxy.
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// Largest reply accepted, like the bridge's limit for REST replies.
const MAX_REPLY: usize = 2 * 1024 * 1024;
/// A tool call can take a while on a busy server.
const TIMEOUT_SECS: u64 = 60;

/// The product's MCP address: HTTPS (or literal loopback HTTP) to `/mcp/{product}`, nothing else in the URL.
pub fn validate_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "invalid MCP URL")?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
    let product = url
        .path()
        .strip_prefix("/mcp/")
        .filter(|p| !p.is_empty() && wk_contracts::valid_id(p));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || product.is_none()
    {
        return Err(
            "the MCP URL must be https://HOST/mcp/PRODUCT (or http to a literal loopback address)"
                .into(),
        );
    }
    Ok(url)
}
/// A token that can safely be sent as a header value: printable ASCII, no whitespace.
fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= 512 && token.bytes().all(|b| b.is_ascii_graphic())
}

/// Read only a bounded, private regular file; never follow a token-file symlink.
#[cfg(unix)]
pub fn token_from_file(path: &std::path::Path) -> Result<String, String> {
    use std::{
        io::Read,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| "token file unavailable")?;
    let meta = file.metadata().map_err(|_| "token file unavailable")?;
    if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 {
        return Err("token file must be a private regular file (mode 0600)".into());
    }
    let mut text = String::new();
    file.take(515)
        .read_to_string(&mut text)
        .map_err(|_| "token file unreadable")?;
    let token = text.trim();
    if text.len() > 514 || !valid_token(token) {
        return Err("invalid token file".into());
    }
    Ok(token.into())
}
#[cfg(not(unix))]
pub fn token_from_file(_: &std::path::Path) -> Result<String, String> {
    Err("token-file onboarding is currently supported on macOS and Linux; use WK_TOKEN on other platforms".into())
}

fn rpc_error(id: &Value, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32001,"message":message}})
}
/// The messages in one reply body: a JSON message, a JSON batch, or an event stream of `data:` lines.
fn messages(content_type: &str, body: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(body);
    if content_type.starts_with("text/event-stream") {
        return text
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
            .collect();
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Array(batch)) => batch,
        Ok(one) => vec![one],
        Err(_) => vec![],
    }
}

pub struct Proxy {
    client: reqwest::Client,
    url: reqwest::Url,
    token: String,
}
impl Proxy {
    pub fn new(
        url: &str,
        token: &str,
        ca: Option<&[reqwest::Certificate]>,
    ) -> Result<Self, String> {
        if !valid_token(token) {
            return Err("the token is empty or contains characters that cannot be sent".into());
        }
        Ok(Self {
            client: wk_login::client_trusting(ca, TIMEOUT_SECS)?,
            url: validate_url(url)?,
            token: token.into(),
        })
    }
    /// Forward one message. Returns what to write back, which is nothing for a notification.
    async fn forward(&self, message: &Value) -> Vec<Value> {
        let id = message.get("id").cloned();
        let is_request = id.is_some() && message.get("method").is_some();
        let fail = |text: &str| -> Vec<Value> {
            match (&id, is_request) {
                (Some(id), true) => vec![rpc_error(id, text)],
                _ => {
                    eprintln!("wk-mcp proxy: {text}");
                    vec![]
                }
            }
        };
        let response = match self
            .client
            .post(self.url.clone())
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(message.to_string())
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return fail("Workspace Knowledge could not be reached. Check the network and the server address."),
        };
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return fail("Workspace Knowledge refused the token (HTTP 401). It may have expired or been revoked; ask your admin for a new one.");
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return fail("Workspace Knowledge denied access (HTTP 403). The token's user may no longer belong to this product.");
        }
        if status == reqwest::StatusCode::ACCEPTED || status == reqwest::StatusCode::NO_CONTENT {
            return vec![];
        }
        if !status.is_success() {
            return fail(&format!(
                "Workspace Knowledge answered HTTP {}.",
                status.as_u16()
            ));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let mut body: Vec<u8> = Vec::new();
        let mut response = response;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len() + chunk.len() > MAX_REPLY {
                        return fail("The reply from Workspace Knowledge was too large.");
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => return fail("The connection to Workspace Knowledge broke mid-reply."),
            }
        }
        let out = messages(&content_type, &body);
        if out.is_empty() && is_request {
            return fail("Workspace Knowledge sent a reply that could not be read.");
        }
        out
    }
}

/// Serve until `input` ends: one JSON message per line in, one per line out, in order.
pub async fn run<R, W>(proxy: &Proxy, input: R, mut output: W) -> Result<(), String>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = input.lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|_| "could not read from the client")?
    {
        if line.trim().is_empty() {
            continue;
        }
        let replies = match serde_json::from_str::<Value>(&line) {
            Ok(message) => proxy.forward(&message).await,
            Err(_) => vec![
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            ],
        };
        for reply in replies {
            output
                .write_all(reply.to_string().as_bytes())
                .await
                .map_err(|_| "could not write to the client")?;
            output
                .write_all(b"\n")
                .await
                .map_err(|_| "could not write to the client")?;
            output
                .flush()
                .await
                .map_err(|_| "could not write to the client")?;
        }
    }
    Ok(())
}
/// The real thing: stdin to stdout.
pub async fn run_stdio(proxy: &Proxy) -> Result<(), String> {
    run(
        proxy,
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn token_files_are_private_bounded_regular_files_and_are_read_again_after_rotation() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "first-token\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(token_from_file(&path).unwrap(), "first-token");
        std::fs::write(&path, "replacement-token\n").unwrap();
        assert_eq!(token_from_file(&path).unwrap(), "replacement-token");
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(token_from_file(&link).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(token_from_file(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        for bad in [String::new(), "a b".into(), "x".repeat(515)] {
            std::fs::write(&path, bad).unwrap();
            assert!(token_from_file(&path).is_err());
        }
        assert!(token_from_file(dir.path()).is_err());
    }
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(axum::http::HeaderMap, Value)>>>;
    /// A stand-in for the hosted endpoint. `reply` decides the answer from the request's method.
    async fn endpoint(
        reply: impl Fn(&Value) -> (u16, &'static str, String) + Clone + Send + Sync + 'static,
    ) -> (String, Seen) {
        let seen: Seen = Default::default();
        let recorded = seen.clone();
        let app = axum::Router::new().route(
            "/mcp/demo",
            axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
                let (recorded, reply) = (recorded.clone(), reply.clone());
                async move {
                    let message: Value = serde_json::from_str(&body).unwrap();
                    recorded.lock().unwrap().push((headers, message.clone()));
                    let (status, content_type, text) = reply(&message);
                    (
                        axum::http::StatusCode::from_u16(status).unwrap(),
                        [("content-type", content_type)],
                        text,
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp/demo", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, seen)
    }
    async fn talk(proxy: &Proxy, input: &str) -> Vec<Value> {
        let mut out: Vec<u8> = Vec::new();
        run(proxy, tokio::io::BufReader::new(input.as_bytes()), &mut out)
            .await
            .unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
    fn ok_reply(message: &Value) -> (u16, &'static str, String) {
        match message["method"].as_str() {
            Some("notifications/initialized") => (202, "text/plain", String::new()),
            _ => (
                200,
                "application/json",
                json!({"jsonrpc":"2.0","id":message["id"],"result":{"echo":message["method"]}})
                    .to_string(),
            ),
        }
    }
    const TOKEN: &str = "wkpt_test_token_value";

    #[tokio::test]
    async fn every_message_is_forwarded_with_the_token_and_the_answers_come_back_in_order() {
        let (url, seen) = endpoint(ok_reply).await;
        let proxy = Proxy::new(&url, TOKEN, None).unwrap();
        let replies = talk(
            &proxy,
            concat!(
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
                "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
                "\n",
                "{\"jsonrpc\":\"2.0\",\"id\":\"two\",\"method\":\"tools/list\"}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"knowledge.team_work\",\"arguments\":{}}}\n",
            ),
        )
        .await;
        // The notification produced no answer; the three requests were answered in order, ids intact.
        assert_eq!(replies.len(), 3);
        assert_eq!(
            (
                replies[0]["id"].clone(),
                replies[0]["result"]["echo"].clone()
            ),
            (json!(1), json!("initialize"))
        );
        assert_eq!(
            (
                replies[1]["id"].clone(),
                replies[1]["result"]["echo"].clone()
            ),
            (json!("two"), json!("tools/list"))
        );
        assert_eq!(replies[2]["id"], 3);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        for (headers, _) in seen.iter() {
            assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
            assert_eq!(headers["content-type"], "application/json");
            assert!(headers["accept"]
                .to_str()
                .unwrap()
                .contains("text/event-stream"));
        }
        assert_eq!(seen[3].1["params"]["name"], "knowledge.team_work");
    }
    #[tokio::test]
    async fn event_stream_and_batch_replies_are_unpacked_into_lines() {
        let (url, _) = endpoint(|m| match m["method"].as_str() {
            Some("a") => (200, "text/event-stream", format!("event: message\ndata: {}\n\n", json!({"jsonrpc":"2.0","id":m["id"],"result":"sse"}))),
            _ => (200, "application/json", json!([{"jsonrpc":"2.0","id":m["id"],"result":"one"},{"jsonrpc":"2.0","id":99,"result":"two"}]).to_string()),
        })
        .await;
        let proxy = Proxy::new(&url, TOKEN, None).unwrap();
        let replies = talk(&proxy, "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"a\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"b\"}\n").await;
        assert_eq!(
            replies
                .iter()
                .map(|r| r["result"].clone())
                .collect::<Vec<_>>(),
            [json!("sse"), json!("one"), json!("two")]
        );
    }
    #[tokio::test]
    async fn refusals_become_readable_errors_that_keep_the_request_id_and_never_echo_the_body() {
        for (status, needle) in [
            (401u16, "refused the token"),
            (403, "denied access"),
            (500, "HTTP 500"),
        ] {
            let (url, _) = endpoint(move |_| {
                (
                    status,
                    "application/json",
                    "{\"error\":\"SECRET-DETAIL\"}".into(),
                )
            })
            .await;
            let proxy = Proxy::new(&url, TOKEN, None).unwrap();
            let replies = talk(
                &proxy,
                "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/list\"}\n",
            )
            .await;
            assert_eq!(replies.len(), 1);
            assert_eq!(replies[0]["id"], 7);
            let message = replies[0]["error"]["message"].as_str().unwrap();
            assert!(message.contains(needle), "{message}");
            assert!(!message.contains("SECRET-DETAIL") && !message.contains(TOKEN));
        }
        // A refused notification has nobody to answer: nothing is written.
        let (url, _) = endpoint(|_| (401, "text/plain", String::new())).await;
        let proxy = Proxy::new(&url, TOKEN, None).unwrap();
        assert!(talk(
            &proxy,
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n"
        )
        .await
        .is_empty());
    }
    #[tokio::test]
    async fn an_unreachable_server_an_unreadable_reply_and_a_bad_line_are_all_answered() {
        let proxy = Proxy::new("http://127.0.0.1:9/mcp/demo", TOKEN, None).unwrap();
        let replies = talk(
            &proxy,
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\nthis is not json\n",
        )
        .await;
        assert!(replies[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("could not be reached"));
        assert_eq!(
            (
                replies[1]["id"].clone(),
                replies[1]["error"]["code"].clone()
            ),
            (Value::Null, json!(-32700))
        );
        let (url, _) =
            endpoint(|_| (200, "application/json", "<html>not json</html>".into())).await;
        let proxy = Proxy::new(&url, TOKEN, None).unwrap();
        let replies = talk(&proxy, "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"x\"}\n").await;
        assert!(replies[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("could not be read"));
    }
    #[tokio::test]
    async fn a_reply_over_the_size_limit_is_refused() {
        let big = "x".repeat(MAX_REPLY + 10);
        let (url, _) = endpoint(move |m| {
            (
                200,
                "application/json",
                json!({"jsonrpc":"2.0","id":m["id"],"result":big}).to_string(),
            )
        })
        .await;
        let proxy = Proxy::new(&url, TOKEN, None).unwrap();
        let replies = talk(&proxy, "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\"}\n").await;
        assert!(replies[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("too large"));
    }
    #[test]
    fn only_a_product_mcp_address_over_https_or_loopback_is_accepted_and_the_token_must_be_sendable(
    ) {
        for ok in [
            "https://wk.example/mcp/demo",
            "https://wk.example:8444/mcp/with_under-score",
            "http://127.0.0.1:8080/mcp/demo",
            "http://[::1]:8080/mcp/demo",
        ] {
            assert!(validate_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://wk.example/mcp/demo",
            "https://wk.example/",
            "https://wk.example/mcp/",
            "https://wk.example/mcp/de mo",
            "https://wk.example/v1/me",
            "https://u:p@wk.example/mcp/demo",
            "https://wk.example/mcp/demo?x=1",
            "https://wk.example/mcp/demo#f",
            "https://wk.example/mcp/demo/extra",
            "not a url",
        ] {
            assert!(validate_url(bad).is_err(), "{bad}");
        }
        for bad in [
            "",
            "has space",
            "line\nbreak",
            "tab\t",
            "é",
            &"x".repeat(513),
        ] {
            assert!(
                Proxy::new("https://wk.example/mcp/demo", bad, None).is_err(),
                "{bad:?}"
            );
        }
        assert!(Proxy::new("https://wk.example/mcp/demo", TOKEN, None).is_ok());
    }
}
