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
