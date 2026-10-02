#[test]
fn published_schema_matches_current_types() {
    let published: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/architecture/schemas/effective-policy-v1.json"
    ))
    .unwrap();
    assert_eq!(published, wk_knowledge::lifecycle::policy_schema());
}
