use serde_json::{json, Value};
use wk_contracts::{validate_card, CardSubmission, Provenance};

fn card() -> Value {
    json!({"schema_version":1,"idempotency_key":"k1","policy_revision":1,"title":"Cap retries","intent":"Avoid repeat charges","rationale":"The gateway charges twice on duplicate calls.","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"partial","base_revision":null})
}

#[test]
fn provenance_is_absent_and_omitted_by_default() {
    let c: CardSubmission = serde_json::from_value(card()).unwrap();
    assert!(c.provenance.is_none());
    // Old servers use deny_unknown_fields: an ordinary card must serialize without the key.
    assert!(serde_json::to_value(&c)
        .unwrap()
        .get("provenance")
        .is_none());
}

#[test]
fn accepts_assistant_written_on_a_partial_card() {
    let mut v = card();
    v["provenance"] = json!("assistant_written");
    assert!(validate_card(&v).is_ok());
    let c: CardSubmission = serde_json::from_value(v).unwrap();
    assert_eq!(c.provenance, Some(Provenance::AssistantWritten));
}

#[test]
fn rejects_unknown_provenance() {
    let mut v = card();
    v["provenance"] = json!("hand_waved");
    assert!(validate_card(&v).is_err());
}

#[test]
fn rejects_provenance_on_a_truncated_card() {
    let mut v = card();
    v["provenance"] = json!("assistant_written");
    v["capture_completeness"] = json!("truncated");
    assert_eq!(
        validate_card(&v).unwrap_err(),
        "provenance cannot be set on a truncated card"
    );
}

#[test]
fn tool_callers_may_not_set_provenance() {
    use wk_contracts::reject_collector_only_fields;
    assert!(reject_collector_only_fields(&card()).is_ok());
    let mut v = card();
    v["provenance"] = json!("assistant_written");
    assert!(reject_collector_only_fields(&v).is_err());
}
