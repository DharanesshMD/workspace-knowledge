use serde_json::json;
use std::path::Path;
use wk_adapters::{config, install};
#[test]
fn native_install_preserves_unrelated_configuration_and_is_reversible() {
    for client in ["codex", "antigravity"] {
        let before = if client == "codex" {
            json!({"custom":"preserve","hooks":{}})
        } else {
            json!({"other":{"enabled":false}})
        };
        let fragment = config(
            client,
            Path::new("/tmp/wk-agent"),
            Path::new("/tmp/wk-state"),
        )
        .unwrap();
        let once = install::merge(&before, &fragment).unwrap();
        assert_eq!(install::merge(&once, &fragment).unwrap(), once);
        let restored = install::remove(&once, &fragment).unwrap();
        if client == "antigravity" {
            assert_eq!(restored, before);
        } else {
            assert_eq!(restored["custom"], "preserve");
            assert!(restored["hooks"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v.as_array().unwrap().is_empty()));
        }
        assert!(!fragment
            .to_string()
            .contains("dangerously-bypass-hook-trust"));
    }
}
#[test]
fn antigravity_name_conflict_is_not_overwritten_and_modified_entry_is_not_removed() {
    let fragment = config(
        "antigravity",
        Path::new("/tmp/wk-agent"),
        Path::new("/tmp/wk-state"),
    )
    .unwrap();
    let existing = json!({"workspace-knowledge":{"enabled":false}});
    assert!(install::merge(&existing, &fragment).is_err());
    assert_eq!(install::remove(&existing, &fragment).unwrap(), existing);
}
