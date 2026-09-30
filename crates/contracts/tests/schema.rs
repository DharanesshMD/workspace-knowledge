use serde_json::{json, Value};
use wk_contracts::validate_card;
fn card() -> Value {
    json!({"schema_version":1,"idempotency_key":"turn-1","policy_revision":1,"title":"Rate limiter","intent":"Prevent overload","rationale":"Shared enforcement","decisions":["Use tenant limits"],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})
}
#[test]
fn accepts_valid_card() {
    assert!(validate_card(&card()).is_ok());
}
#[test]
fn rejects_raw_payload() {
    let mut c = card();
    c["private_payload"] = json!({"text":"PRIVATE_CANARY"});
    assert!(validate_card(&c).is_err());
}
#[test]
fn rejects_future_schema() {
    let mut c = card();
    c["schema_version"] = json!(99);
    assert!(validate_card(&c).is_err());
}
#[test]
fn rejects_empty_scope() {
    let mut c = card();
    c["service_ids"] = json!([]);
    assert!(validate_card(&c).is_err());
}
#[test]
fn rejects_unbacked_verification() {
    let mut c = card();
    c["lifecycle"] = json!("verified");
    assert!(validate_card(&c).is_err());
}
#[test]
fn rejects_unknown_nested_evidence() {
    let mut c = card();
    c["evidence"] = json!([{"kind":"conversation_report","reference":"msg-1","secret":"CANARY"}]);
    assert!(validate_card(&c).is_err());
}
#[test]
fn errors_do_not_echo_content() {
    let mut c = card();
    c["lifecycle"] = json!("SECRET_CANARY");
    let e = validate_card(&c).unwrap_err();
    assert!(!e.contains("SECRET_CANARY"));
}
