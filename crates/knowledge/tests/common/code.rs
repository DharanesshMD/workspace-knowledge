use super::Fixture;
use wk_contracts::{change_context::*, CardSubmission};
use wk_knowledge::code_provenance::*;
pub fn targets() -> Vec<CodeTarget> {
    vec![CodeTarget {
        path: "backend/retry.rs".into(),
        start: Some(1),
        end: Some(4),
    }]
}
pub fn body(key: &str, services: Vec<String>) -> CardSubmission {
    serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Retry invariant","intent":"Prevent duplicate billing","rationale":"Keep the original operation id because the billing worker deduplicates by that id","decisions":[],"constraints":["Preserve the operation id across retries"],"open_questions":[],"service_ids":services,"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()
}
pub fn request(repo: uuid::Uuid) -> ChangeContextRequest {
    ChangeContextRequest {
        schema_version: 1,
        repository_id: repo,
        base_revision: "a".repeat(40),
        targets: targets(),
        operation_class: "read_context".into(),
        operation_handle: None,
        budget_bytes: 16384,
    }
}
pub async fn repository(f: &Fixture) -> (uuid::Uuid, MappingRecord) {
    let id = uuid::Uuid::new_v4();
    f.k.configure_repository(
        &f.admin,
        "demo",
        RepositoryInput {
            id,
            services: vec!["backend".into()],
            high_risk_paths: vec!["backend/retry.rs".into()],
            contact_hosts: vec!["team.example.invalid".into()],
            base_revision: None,
        },
    )
    .await
    .unwrap();
    let m =
        f.k.configure_code_mapping(
            &f.admin,
            "demo",
            MappingInput {
                repository_id: id,
                user_id: "alice".into(),
                identity_reference: "team:alice".into(),
                proof_reference: "Team attestation".into(),
                services: vec!["backend".into()],
                base_revision: None,
            },
        )
        .await
        .unwrap();
    (id, m)
}
pub async fn confirmed(
    f: &Fixture,
    repo: uuid::Uuid,
    m: &MappingRecord,
) -> (wk_knowledge::Card, AnchorRecord) {
    f.k.put_code_profile(
        &f.alice,
        "demo",
        ProfileInput {
            display_name: Some("Alice Synthetic".into()),
            contact_url: Some("https://team.example.invalid/alice".into()),
            services: vec!["backend".into()],
            base_revision: None,
        },
    )
    .await
    .unwrap();
    let c =
        f.k.propose_contribution(
            &f.bob,
            "demo",
            ContributionInput {
                repository_id: repo,
                mapping_id: m.id,
                code_revision: "a".repeat(40),
                targets: targets(),
            },
        )
        .await
        .unwrap();
    let c =
        f.k.confirm_contribution(&f.alice, "demo", c.id, c.revision)
            .await
            .unwrap();
    let c =
        f.k.review_contribution(&f.bob, "demo", c.id, c.revision)
            .await
            .unwrap();
    let card =
        f.k.submit(&f.alice, "demo", body("confirmed", vec!["backend".into()]))
            .await
            .unwrap();
    f.k.publish(&f.bob, "demo", card.id).await.unwrap();
    let a =
        f.k.propose_rationale_anchor(
            &f.bob,
            "demo",
            AnchorInput {
                repository_id: repo,
                card_id: card.id,
                card_revision: card.revision,
                code_revision: "a".repeat(40),
                targets: targets(),
                contribution_id: Some(c.id),
            },
        )
        .await
        .unwrap();
    let a =
        f.k.confirm_rationale_anchor(&f.alice, "demo", a.id, a.revision)
            .await
            .unwrap();
    let a =
        f.k.review_rationale_anchor(&f.bob, "demo", a.id, a.revision)
            .await
            .unwrap();
    (card, a)
}
