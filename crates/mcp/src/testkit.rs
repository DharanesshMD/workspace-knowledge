//! Test helpers: a raw HTTP client for the approval page (so Host and Origin can be anything).
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// `(host:port, path)` of a page URL.
pub fn split(url: &str) -> (String, String) {
    let rest = url.strip_prefix("http://").expect("http url");
    let (host, path) = rest.split_once('/').expect("path");
    (host.to_string(), format!("/{path}"))
}
/// Send `request` to `address` and return `(status, body)`.
pub async fn raw(address: &str, request: &str) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    // The server may answer and close before it has read everything it was sent; whatever
    // arrived is the response, and none at all is status 0.
    let _ = stream.write_all(request.as_bytes()).await;
    let mut bytes = Vec::new();
    let _ = stream.read_to_end(&mut bytes).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map_or("", |(_, b)| b)
        .to_string();
    (status, body)
}
pub async fn get(address: &str, path: &str, host: &str) -> (u16, String) {
    raw(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
    )
    .await
}
/// A form POST. `origin` of `None` sends no Origin header.
pub async fn post(
    address: &str,
    path: &str,
    host: &str,
    origin: Option<&str>,
    content_type: &str,
    body: &str,
) -> (u16, String) {
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    raw(
        address,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\n{origin}Content-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
    .await
}
/// The CSRF token embedded in the approval page.
pub fn csrf(page: &str) -> String {
    let (_, rest) = page
        .split_once("name=\"csrf\" value=\"")
        .expect("csrf field");
    rest.split('"').next().unwrap().to_string()
}
/// A same-origin form submission exactly as a browser sends it.
pub async fn answer(url: &str, decision: &str) -> (u16, String) {
    submit(url, &format!("decision={decision}")).await
}
/// The same, with the form fields a browser would send after the CSRF token.
pub async fn submit(url: &str, fields: &str) -> (u16, String) {
    let (host, path) = split(url);
    let (status, page) = get(&host, &path, &host).await;
    assert_eq!(status, 200, "{page}");
    post(
        &host,
        &path,
        &host,
        Some(&format!("http://{host}")),
        "application/x-www-form-urlencoded",
        &format!("csrf={}&{fields}", csrf(&page)),
    )
    .await
}
