use serde_json::json;
#[test]
fn visible_fields_and_external_paths_are_preserved_without_reasoning() {
    let v = json!({"hook_event_name":"beforeSubmitPrompt","conversation_id":"s","generation_id":"t","workspace_roots":["/product"],"attachments":[{"file_path":"/personal/x"}],"prompt":"visible"});
    let o = wk_adapters::normalize("cursor", &v).unwrap().unwrap();
    assert_eq!(o.text, "visible");
    assert_eq!(o.paths.len(), 2);
    assert!(!o.complete_coverage);
    assert!(wk_adapters::normalize(
        "cursor",
        &json!({"hook_event_name":"afterAgentThought","text":"PRIVATE_REASONING"})
    )
    .unwrap()
    .is_none());
}
#[test]
fn unsupported_capture_and_missing_attribution_fail_closed() {
    assert!(wk_adapters::normalize("codex", &json!({"hook_event_name":"Stop"})).is_err());
    assert!(wk_adapters::normalize(
        "claude-code",
        &json!({"hook_event_name":"Stop","session_id":"s","last_assistant_message":"hello"})
    )
    .is_err());
}
#[test]
fn config_merge_is_idempotent_and_uninstall_preserves_edits() {
    let fragment = json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"wk-owned"}]}]}});
    let old = json!({"model":"user-choice","hooks":{"Stop":[{"hooks":[{"type":"command","command":"user-owned"}]}]}});
    let merged = wk_adapters::install::merge(&old, &fragment).unwrap();
    assert_eq!(
        wk_adapters::install::merge(&merged, &fragment).unwrap(),
        merged
    );
    assert_eq!(
        wk_adapters::install::remove(&merged, &fragment).unwrap(),
        old
    );
    let mut edited = merged;
    edited["hooks"]["Stop"][1]["hooks"][0]["command"] = json!("changed-by-user");
    assert_eq!(
        wk_adapters::install::remove(&edited, &fragment).unwrap(),
        edited
    );
}
#[test]
fn observational_outputs_never_block_or_request_continuation() {
    assert_eq!(
        wk_adapters::success("cursor", "beforeReadFile"),
        json!({"permission":"allow"})
    );
    assert_eq!(
        wk_adapters::success("cursor", "beforeSubmitPrompt"),
        json!({"continue":true})
    );
    assert_eq!(wk_adapters::success("cursor", "stop"), json!({}));
    assert_eq!(wk_adapters::success("claude-code", "Stop"), json!({}));
}
