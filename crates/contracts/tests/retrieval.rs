use serde_json::json;
#[test]
fn context_diagnostics_and_references_share_the_same_size_budget() {
    let cards = vec![
        json!({"id":"one","revision":1,"body":{"title":"Reviewed rationale","lifecycle":"proposed"}}),
    ];
    let status = json!({"mode":"lexical","reason":"embedding_unavailable"});
    for budget in [64, 200, 2000] {
        let result = wk_contracts::retrieval::context(&cards, budget, Some(&status));
        assert!(result.to_string().len() <= budget);
        assert_eq!(result["retrieval"]["mode"], "lexical");
        if budget == 64 {
            assert!(result["cards"].as_array().unwrap().is_empty());
            assert_eq!(result["omitted"], true);
        } else {
            assert_eq!(result["retrieval"]["reason"], "embedding_unavailable");
            assert_eq!(result["cards"][0]["id"], "one");
        }
    }
    let raw = json!({"mode":"hybrid","reason":"PRIVATE_TRANSCRIPT_CANARY"});
    assert!(!wk_contracts::retrieval::context(&[], 200, Some(&raw))
        .to_string()
        .contains("PRIVATE_TRANSCRIPT_CANARY"));
}
