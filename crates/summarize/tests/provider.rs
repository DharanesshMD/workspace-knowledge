use std::io::{Read, Write};
fn provider(content: &str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let content = content.to_owned();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = vec![0; 32768];
        let _ = stream.read(&mut bytes).unwrap();
        let body = serde_json::json!({"choices":[{"message":{"content":content}}]}).to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    format!("http://{addr}/v1/chat/completions")
}
#[tokio::test]
async fn local_provider_output_is_validated_before_card_creation() {
    let value = serde_json::json!({"title":"Tenant limits","intent":"Reduce load","rationale":"Reuse middleware","decisions":[],"constraints":[],"open_questions":[],"sensitive":false});
    let endpoint = provider(&value.to_string());
    let result = wk_summarize::extract(&endpoint, "synthetic", "Add tenant rate limiting")
        .await
        .unwrap();
    assert_eq!(result.title, "Tenant limits");
    let mut malicious = value;
    malicious["intent"] = serde_json::json!("password=FAKE");
    let endpoint = provider(&malicious.to_string());
    assert!(
        wk_summarize::extract(&endpoint, "synthetic", "Add rate limiting")
            .await
            .is_err()
    );
}

/// Fake Ollama `/api/chat`: returns `content` and hands back the request body it received.
fn native_provider(content: &str) -> (String, std::sync::mpsc::Receiver<serde_json::Value>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let content = content.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut raw = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = stream.read(&mut buf).unwrap();
            raw.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&raw);
            if let Some(split) = text.find("\r\n\r\n") {
                let length = text[..split]
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if raw.len() >= split + 4 + length {
                    tx.send(serde_json::from_slice(&raw[split + 4..split + 4 + length]).unwrap())
                        .unwrap();
                    break;
                }
            }
        }
        let body =
            serde_json::json!({"message":{"role":"assistant","content":content}}).to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    (format!("http://{addr}/api/chat"), rx)
}

#[tokio::test]
async fn ollama_native_endpoint_sends_schema_with_thinking_disabled() {
    let value = serde_json::json!({"title":"Billing limits","intent":"Limit billing calls","rationale":"","decisions":["Redis was rejected"],"constraints":[],"open_questions":[],"sensitive":false});
    let (endpoint, sent) = native_provider(&value.to_string());
    let result = wk_summarize::extract(&endpoint, "gemma4:e2b-mlx", "Add billing limits")
        .await
        .unwrap();
    assert_eq!(result.decisions, ["Redis was rejected"]);
    let request = sent.recv().unwrap();
    assert_eq!(request["think"], false);
    assert_eq!(request["format"]["required"][0], "title");
    assert!(request.get("response_format").is_none());
}
