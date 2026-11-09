//! The portal's tests run against `web/tests/fixtures/agent-setup*.json`. It must be exactly what the server
//! sends, so it is generated from the Rust definition and this test fails when they drift apart.
//! Regenerate with: UPDATE_FIXTURES=1 cargo test -p wk-contracts --test agent_setup_fixture
use serde_json::json;
use wk_contracts::agent_setup::{clients, rules, TOKEN_ENV, TOKEN_PLACEHOLDER};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/tests/fixtures");

#[test]
fn the_portal_fixtures_are_what_the_server_sends() {
    let url = "https://wk.example/mcp/alpha";
    for (file, auto) in [("agent-setup.json", false), ("agent-setup-auto.json", true)] {
        let body = json!({
            "product": "alpha", "mcp_url": url, "token_env": TOKEN_ENV, "token_placeholder": TOKEN_PLACEHOLDER,
            "rules": rules(auto), "clients": clients(url, auto),
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&body).unwrap());
        let path = format!("{DIR}/{file}");
        if std::env::var("UPDATE_FIXTURES").is_ok() {
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        assert_eq!(
            std::fs::read_to_string(&path)
                .expect("fixture missing; run with UPDATE_FIXTURES=1")
                .replace("\r\n", "\n"),
            text,
            "web/tests/fixtures/{file} is stale; regenerate it with UPDATE_FIXTURES=1"
        );
    }
}
