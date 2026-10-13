mod common;
use wk_contracts::{change_context::CodeTarget, CardSubmission};
use wk_knowledge::{code_provenance::*, Error};

fn targets() -> Vec<CodeTarget> {
    vec![CodeTarget {
        path: "api/retry.rs".into(),
        start: Some(1),
        end: Some(4),
    }]
}
async fn register(f: &common::Fixture) -> (uuid::Uuid, MappingRecord) {
    let id = uuid::Uuid::new_v4();
    f.k.configure_repository(
        &f.admin,
        "demo",
        RepositoryInput {
            id,
            services: vec!["backend".into()],
            high_risk_paths: vec![],
            contact_hosts: vec![],
            base_revision: None,
        },
    )
    .await
    .unwrap();
    let mapping =
        f.k.configure_code_mapping(
            &f.admin,
            "demo",
            MappingInput {
                repository_id: id,
                user_id: "alice".into(),
                identity_reference: "team:alice".into(),
                proof_reference: "Alice confirmed team identity".into(),
                services: vec!["backend".into()],
                base_revision: None,
            },
        )
        .await
        .unwrap();
    (id, mapping)
}
fn body() -> CardSubmission {
    serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":"reason","policy_revision":1,"title":"Preserve operation identity","intent":"Prevent duplicate operations","rationale":"The billing worker deduplicates using the original operation id","decisions":[],"constraints":["Keep the operation id across retries"],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn contribution_requires_actual_owner_confirmation_and_current_mapping() {
    let f = common::setup().await;
    let (repo, mapping) = register(&f).await;
    let (k, pool, role) = common::runtime(&f).await;
    let c = k
        .propose_contribution(
            &f.bob,
            "demo",
            ContributionInput {
                repository_id: repo,
                mapping_id: mapping.id,
                code_revision: "a".repeat(40),
                targets: targets(),
            },
        )
        .await
        .unwrap();
    assert!(c.confirmed_by.is_none());
    assert_eq!(c.owner, "alice");
    assert_eq!(c.submitted_by, "bob");
    assert_eq!(
        k.confirm_contribution(&f.bob, "demo", c.id, c.revision)
            .await
            .unwrap_err(),
        Error::Denied
    );
    assert!(k
        .review_contribution(&f.bob, "demo", c.id, c.revision)
        .await
        .is_err());
    let c = k
        .confirm_contribution(&f.alice, "demo", c.id, c.revision)
        .await
        .unwrap();
    assert_eq!(c.revision, 2);
    assert_eq!(
        k.review_contribution(&f.reader, "demo", c.id, c.revision)
            .await
            .unwrap_err(),
        Error::Denied
    );
    let reviewed = k
        .review_contribution(&f.bob, "demo", c.id, c.revision)
        .await
        .unwrap();
    assert_eq!(reviewed.revision, 3);
    assert_eq!(reviewed.confirmed_by.as_deref(), Some("alice"));
    assert_eq!(reviewed.reviewed_by.as_deref(), Some("bob"));
    f.k.configure_code_mapping(
        &f.admin,
        "demo",
        MappingInput {
            repository_id: repo,
            user_id: "bob".into(),
            identity_reference: "team:alice".into(),
            proof_reference: "Explicit identity correction".into(),
            services: vec!["backend".into()],
            base_revision: Some(mapping.revision),
        },
    )
    .await
    .unwrap();
    assert!(k
        .confirm_contribution(&f.alice, "demo", reviewed.id, reviewed.revision)
        .await
        .is_err());
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn rationale_confirmation_cannot_be_forged_by_its_submitter_or_reused_after_correction() {
    let f = common::setup().await;
    let (repo, _) = register(&f).await;
    let card = f.k.submit(&f.alice, "demo", body()).await.unwrap();
    let (k, pool, role) = common::runtime(&f).await;
    let a = k
        .propose_rationale_anchor(
            &f.bob,
            "demo",
            AnchorInput {
                repository_id: repo,
                card_id: card.id,
                card_revision: card.revision,
                code_revision: "a".repeat(40),
                targets: targets(),
                contribution_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        k.confirm_rationale_anchor(&f.bob, "demo", a.id, a.revision)
            .await
            .unwrap_err(),
        Error::Denied
    );
    let a = k
        .confirm_rationale_anchor(&f.alice, "demo", a.id, a.revision)
        .await
        .unwrap();
    assert!(k
        .review_rationale_anchor(&f.bob, "demo", a.id, a.revision)
        .await
        .is_err());
    f.k.publish(&f.bob, "demo", card.id).await.unwrap();
    let a = k
        .review_rationale_anchor(&f.bob, "demo", a.id, a.revision)
        .await
        .unwrap();
    assert_eq!(a.confirmed_by.as_deref(), Some("alice"));
    assert_eq!(a.reviewed_by.as_deref(), Some("bob"));
    let mut corrected = body();
    corrected.base_revision = Some(card.revision as u64);
    corrected.idempotency_key = "corrected".into();
    corrected.rationale = "The operation contract changed".into();
    let revised =
        f.k.correct(&f.alice, "demo", card.id, corrected)
            .await
            .unwrap();
    assert!(revised.revision > card.revision);
    assert_eq!(
        k.confirm_rationale_anchor(&f.alice, "demo", a.id, a.revision)
            .await
            .unwrap_err(),
        Error::Conflict
    );
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn contribution_link_rejects_another_repository_revision_or_uncovered_range() {
    let f = common::setup().await;
    let (repo, mapping) = register(&f).await;
    let card = f.k.submit(&f.alice, "demo", body()).await.unwrap();
    f.k.publish(&f.bob, "demo", card.id).await.unwrap();
    let c =
        f.k.propose_contribution(
            &f.bob,
            "demo",
            ContributionInput {
                repository_id: repo,
                mapping_id: mapping.id,
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
    for (revision, end) in [("b".repeat(40), Some(4)), ("a".repeat(40), Some(10))] {
        let mut target = targets();
        target[0].end = end;
        assert!(f
            .k
            .propose_rationale_anchor(
                &f.bob,
                "demo",
                AnchorInput {
                    repository_id: repo,
                    card_id: card.id,
                    card_revision: card.revision,
                    code_revision: revision,
                    targets: target,
                    contribution_id: Some(c.id)
                }
            )
            .await
            .is_err());
    }
    let (other, _) = register(&f).await;
    assert!(f
        .k
        .propose_rationale_anchor(
            &f.bob,
            "demo",
            AnchorInput {
                repository_id: other,
                card_id: card.id,
                card_revision: card.revision,
                code_revision: "a".repeat(40),
                targets: targets(),
                contribution_id: Some(c.id)
            }
        )
        .await
        .is_err());
    let accepted =
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
    assert_eq!(accepted.contribution_id, Some(c.id));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn maintainer_assignment_requires_operator_and_does_not_infer_authorship() {
    let f = common::setup().await;
    let (repo, _) = register(&f).await;
    let input = MaintainerInput {
        repository_id: repo,
        user_id: "bob".into(),
        targets: targets(),
        services: vec!["backend".into()],
        base_revision: None,
    };
    assert_eq!(
        f.k.configure_code_maintainer(&f.alice, "demo", input.clone())
            .await
            .unwrap_err(),
        Error::Denied
    );
    f.k.configure_code_maintainer(&f.admin, "demo", input.clone())
        .await
        .unwrap();
    assert_eq!(
        f.k.configure_code_maintainer(&f.admin, "demo", input)
            .await
            .unwrap_err(),
        Error::Conflict
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn validation_row_locks_do_not_grant_operator_metadata_writes() {
    let f = common::setup().await;
    let (repo, mapping) = register(&f).await;
    let (_, pool, role) = common::runtime(&f).await;
    for (table, id) in [("code_repositories", repo), ("code_mappings", mapping.id)] {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT set_config('wk.org',$1,true),set_config('wk.user',$2,true)")
            .bind(&f.alice.org)
            .bind(&f.alice.user)
            .execute(&mut *tx)
            .await
            .unwrap();
        let locked: uuid::Uuid =
            sqlx::query_scalar(&format!("SELECT id FROM {table} WHERE id=$1 FOR SHARE"))
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(locked, id);
        assert!(sqlx::query(&format!(
            "UPDATE {table} SET revision=revision+1 WHERE id=$1"
        ))
        .bind(id)
        .execute(&mut *tx)
        .await
        .is_err());
        tx.rollback().await.unwrap();
    }
    common::cleanup_runtime(&f, pool, role).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn another_contributor_and_unknown_mapping_do_not_reuse_alices_confirmation() {
    let f = common::setup().await;
    let (repo, _) = register(&f).await;
    assert_eq!(
        f.k.propose_contribution(
            &f.bob,
            "demo",
            ContributionInput {
                repository_id: repo,
                mapping_id: uuid::Uuid::new_v4(),
                code_revision: "a".repeat(40),
                targets: targets()
            }
        )
        .await
        .unwrap_err(),
        Error::NotFound
    );
    let mapping =
        f.k.configure_code_mapping(
            &f.admin,
            "demo",
            MappingInput {
                repository_id: repo,
                user_id: "bob".into(),
                identity_reference: "team:bob".into(),
                proof_reference: "Bob confirmed team identity".into(),
                services: vec!["backend".into()],
                base_revision: None,
            },
        )
        .await
        .unwrap();
    let c =
        f.k.propose_contribution(
            &f.alice,
            "demo",
            ContributionInput {
                repository_id: repo,
                mapping_id: mapping.id,
                code_revision: "a".repeat(40),
                targets: targets(),
            },
        )
        .await
        .unwrap();
    assert_eq!(c.owner, "bob");
    assert_eq!(
        f.k.confirm_contribution(&f.alice, "demo", c.id, c.revision)
            .await
            .unwrap_err(),
        Error::Denied
    );
    let c =
        f.k.confirm_contribution(&f.bob, "demo", c.id, c.revision)
            .await
            .unwrap();
    assert_eq!(c.confirmed_by.as_deref(), Some("bob"));
    assert!(c.reviewed_by.is_none());
}
