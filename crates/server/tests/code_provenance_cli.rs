use serde_json::{json, Value};
use wk_server::admin_cli::{parse, AdminRequest};

#[test]
fn operator_commands_parse_strict_metadata_without_claimed_actor_override() {
    let repository = "10000000-0000-0000-0000-000000000001";
    let commands: Vec<Value> = vec![
        json!({"kind":"code-repository","repository":{"id":repository,"services":["backend"],"high_risk_paths":["api/"],"contact_hosts":["chat.synthetic.example"],"base_revision":null}}),
        json!({"kind":"code-mapping","mapping":{"repository_id":repository,"user_id":"alice","identity_reference":"team:alice","proof_reference":"Identity confirmed by Alice","services":["backend"],"base_revision":null}}),
        json!({"kind":"code-maintainer","maintainer":{"repository_id":repository,"user_id":"bob","targets":[{"path":"api/retry.rs","start":null,"end":null}],"services":["backend"],"base_revision":null}}),
    ];
    for command in commands {
        let mut input = json!({"token":"synthetic-token","product":"demo","command":command});
        assert!(parse::<AdminRequest, _>(input.to_string().as_bytes()).is_ok());
        input["org"] = json!("foreign-org");
        assert!(parse::<AdminRequest, _>(input.to_string().as_bytes()).is_err());
    }
    let raw = json!({"token":"synthetic-token","product":"demo","command":{"kind":"code-repository","repository":{"id":repository,"services":["backend"],"high_risk_paths":[],"contact_hosts":[],"base_revision":null,"raw_prompt":"PRIVATE_CANARY"}}});
    assert!(parse::<AdminRequest, _>(raw.to_string().as_bytes()).is_err());
}
