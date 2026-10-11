use serde_json::{json, Value};
use wk_contracts::change_context::{ChangeContextRequest, CodeTarget};

fn input() -> Value {
    json!({"schema_version":1,"repository_id":"10000000-0000-0000-0000-000000000001","base_revision":"a".repeat(40),"targets":[{"path":"api/retry.rs","start":1,"end":4}],"operation_class":"edit","operation_handle":null,"budget_bytes":16384})
}

#[test]
fn rejects_unsafe_targets_and_invalid_ranges() {
    let good: ChangeContextRequest = serde_json::from_value(input()).unwrap();
    assert!(good.validate().is_ok());
    for path in [
        "../secret",
        "/etc/passwd",
        "C:/secret",
        "~/.env",
        "api\\retry.rs",
        "api//retry.rs",
        "api/./retry.rs",
        "-options",
        "api/secret\n",
    ] {
        let mut bad = good.clone();
        bad.targets[0].path = path.into();
        assert!(bad.validate().is_err(), "unsafe target accepted");
    }
    for (start, end) in [
        (Some(0), Some(1)),
        (Some(5), Some(4)),
        (Some(1), None),
        (None, Some(2)),
    ] {
        let mut bad = good.clone();
        bad.targets[0].start = start;
        bad.targets[0].end = end;
        assert!(bad.validate().is_err());
    }
}

#[test]
fn bounded_requests_do_not_accept_raw_content_or_short_revisions() {
    let mut bad = input();
    bad["raw_prompt"] = json!("PRIVATE_CANARY");
    assert!(serde_json::from_value::<ChangeContextRequest>(bad).is_err());
    let good: ChangeContextRequest = serde_json::from_value(input()).unwrap();
    for revision in [
        "abc123",
        "main",
        "A".repeat(40).as_str(),
        "g".repeat(40).as_str(),
    ] {
        let mut bad = good.clone();
        bad.base_revision = revision.into();
        assert!(bad.validate().is_err());
    }
    for count in [0, 21] {
        let mut bad = good.clone();
        bad.targets = (0..count)
            .map(|i| CodeTarget {
                path: format!("api/{i}.rs"),
                start: None,
                end: None,
            })
            .collect();
        assert!(bad.validate().is_err());
    }
    for budget in [0, 1023, 16385] {
        let mut bad = good.clone();
        bad.budget_bytes = budget;
        assert!(bad.validate().is_err());
    }
    let mut bad = good.clone();
    bad.operation_class = "acknowledge".into();
    assert!(bad.validate().is_err());
    let mut bad = good;
    bad.targets.push(bad.targets[0].clone());
    assert!(bad.validate().is_err());
}
