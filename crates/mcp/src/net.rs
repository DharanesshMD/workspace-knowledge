//! Server address and trust rules shared by the bridge and `wk-mcp login`.
use std::path::Path;

/// The server's origin. Only HTTPS, or HTTP to a literal loopback address, with no credentials,
/// path, query or fragment: a sign-in or bearer token never travels in clear to another host.
pub fn server_origin(base: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(base).map_err(|_| "invalid server URL")?;
    if !(url.scheme() == "https"
        || (url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("The bridge requires HTTPS or literal loopback HTTP".into());
    }
    Ok(base.trim_end_matches('/').into())
}
/// The private CA bundle the operator configured: `file` if given, else `WK_SERVER_CA_FILE`.
/// A configured bundle that cannot be used is an error; it never falls back to public roots.
pub fn trust(file: Option<&Path>) -> Result<Option<Vec<reqwest::Certificate>>, String> {
    match file {
        Some(path) => {
            wk_summarize::tls::parse_ca_bundle(&wk_summarize::tls::read_ca_file(path)?).map(Some)
        }
        None => wk_summarize::tls::from_env("WK_SERVER_CA_FILE"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_https_or_literal_loopback_origins_are_accepted() {
        for ok in [
            "https://wk.example",
            "https://wk.example/",
            "https://wk.example:8443",
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
        ] {
            assert!(server_origin(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            server_origin("https://wk.example/").unwrap(),
            "https://wk.example"
        );
        for bad in [
            "ftp://example.com",
            "http://wk.example",
            "http://localhost:3000",
            "http://127.0.0.1/path",
            "http://user@127.0.0.1",
            "http://127.0.0.1?token=x",
            "https://wk.example/mcp/demo",
            "https://u:p@wk.example",
            "https://wk.example/#x",
        ] {
            assert!(server_origin(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn a_named_bundle_is_validated_and_a_bad_one_is_never_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.pem");
        std::fs::write(&junk, "not a certificate").unwrap();
        assert!(trust(Some(&junk)).is_err());
        assert!(trust(Some(&dir.path().join("missing.pem"))).is_err());
    }
}
