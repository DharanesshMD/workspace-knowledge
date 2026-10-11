mod common;
use common::{code::*, Fixture};
use wk_contracts::device::*;
use wk_knowledge::{code_provenance::*, preflight::*, Error};
async fn device(f: &Fixture) -> ProtectedIdentity {
    let reg = DeviceRegistration {
        id: uuid::Uuid::new_v4(),
        product: "demo".into(),
        services: vec!["backend".into()],
        public_key: "11".repeat(32),
        approval_hash: "22".repeat(32),
    };
    let d = f.k.device_register(&f.bob, reg.clone()).await.unwrap();
    let id =
        f.k.bind_workspace(
            &f.bob,
            "demo",
            &d.id.to_string(),
            &reg.approval_hash,
            &reg.services,
            1,
        )
        .await
        .unwrap();
    let epoch: i64 =
        sqlx::query_scalar("SELECT not_before FROM collector_security_epoch WHERE singleton")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    ProtectedIdentity {
        device_id: d.id,
        binding_id: id,
        security_epoch: epoch,
    }
}
fn protected_request(repo: uuid::Uuid) -> wk_contracts::change_context::ChangeContextRequest {
    let mut r = request(repo);
    r.operation_class = "edit".into();
    r.operation_handle = Some(uuid::Uuid::new_v4());
    r
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn acknowledgement_is_owner_device_operation_bound_and_single_use() {
    let f = common::setup().await;
    let (repo, m) = repository(&f).await;
    confirmed(&f, repo, &m).await;
    let who = device(&f).await;
    let r = protected_request(repo);
    let op = r.operation_handle.unwrap();
    let (k, pool, role) = common::runtime(&f).await;
    let b = k
        .create_preflight(&f.bob, "demo", who.clone(), r)
        .await
        .unwrap();
    let id = b.brief_id.unwrap();
    assert!(b.expires_at_ms.is_some());
    assert_eq!(
        k.consume_preflight(&f.bob, "demo", who.clone(), id, op)
            .await
            .unwrap_err(),
        Error::Denied
    );
    assert!(k.get_preflight(&f.alice, "demo", id).await.is_err());
    assert!(k.acknowledge_preflight(&f.alice, "demo", id).await.is_err());
    k.acknowledge_preflight(&f.bob, "demo", id).await.unwrap();
    assert!(k.acknowledge_preflight(&f.bob, "demo", id).await.is_err());
    let mut other = who.clone();
    other.device_id = uuid::Uuid::new_v4();
    assert!(k
        .consume_preflight(&f.bob, "demo", other, id, op)
        .await
        .is_err());
    assert!(k
        .consume_preflight(&f.bob, "demo", who.clone(), id, uuid::Uuid::new_v4())
        .await
        .is_err());
    let first = k.consume_preflight(&f.bob, "demo", who.clone(), id, op);
    let second = k.consume_preflight(&f.bob, "demo", who.clone(), id, op);
    let (x, y) = tokio::join!(first, second);
    assert_eq!(usize::from(x.is_ok()) + usize::from(y.is_ok()), 1);
    assert!(k
        .consume_preflight(&f.bob, "demo", who, id, op)
        .await
        .is_err());
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn read_only_foreign_epoch_and_expired_briefs_cannot_produce_permission() {
    let f = common::setup().await;
    let (repo, _) = repository(&f).await;
    let who = device(&f).await;
    assert_eq!(
        f.k.create_preflight(&f.bob, "demo", who.clone(), request(repo))
            .await
            .unwrap_err(),
        Error::Invalid
    );
    let mut foreign = who.clone();
    foreign.security_epoch += 1;
    assert!(f
        .k
        .create_preflight(&f.bob, "demo", foreign, protected_request(repo))
        .await
        .is_err());
    let r = protected_request(repo);
    let op = r.operation_handle.unwrap();
    let b =
        f.k.create_preflight(&f.bob, "demo", who.clone(), r)
            .await
            .unwrap();
    let id = b.brief_id.unwrap();
    sqlx::query(
        "UPDATE code_preflights SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(id)
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(f.k.get_preflight(&f.bob, "demo", id).await.is_err());
    assert!(f.k.acknowledge_preflight(&f.bob, "demo", id).await.is_err());
    assert!(f
        .k
        .consume_preflight(&f.bob, "demo", who, id, op)
        .await
        .is_err());
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn changed_sources_and_authorization_require_a_new_brief_before_acknowledgement_or_consumption(
) {
    for change in [
        "profile",
        "mapping",
        "repository",
        "maintainer",
        "card",
        "device",
        "membership",
        "source_removal",
        "new_edge",
    ] {
        let f = common::setup().await;
        let (repo, m) = repository(&f).await;
        let (card, anchor) = confirmed(&f, repo, &m).await;
        let who = device(&f).await;
        let r = protected_request(repo);
        let op = r.operation_handle.unwrap();
        let b =
            f.k.create_preflight(&f.bob, "demo", who.clone(), r)
                .await
                .unwrap();
        let id = b.brief_id.unwrap();
        f.k.acknowledge_preflight(&f.bob, "demo", id).await.unwrap();
        match change {
            "profile" => {
                f.k.put_code_profile(
                    &f.alice,
                    "demo",
                    ProfileInput {
                        display_name: Some("Alice Updated".into()),
                        contact_url: None,
                        services: vec!["backend".into()],
                        base_revision: Some(1),
                    },
                )
                .await
                .unwrap();
            }
            "mapping" => {
                f.k.configure_code_mapping(
                    &f.admin,
                    "demo",
                    MappingInput {
                        repository_id: repo,
                        user_id: "alice".into(),
                        identity_reference: "team:alice".into(),
                        proof_reference: "Rechecked attestation".into(),
                        services: vec!["backend".into()],
                        base_revision: Some(m.revision),
                    },
                )
                .await
                .unwrap();
            }
            "repository" => {
                f.k.configure_repository(
                    &f.admin,
                    "demo",
                    RepositoryInput {
                        id: repo,
                        services: vec!["backend".into()],
                        high_risk_paths: vec![],
                        contact_hosts: vec![],
                        base_revision: Some(1),
                    },
                )
                .await
                .unwrap();
            }
            "maintainer" => {
                f.k.configure_code_maintainer(
                    &f.admin,
                    "demo",
                    MaintainerInput {
                        repository_id: repo,
                        user_id: "bob".into(),
                        targets: targets(),
                        services: vec!["backend".into()],
                        base_revision: None,
                    },
                )
                .await
                .unwrap();
            }
            "card" => {
                let mut input = body("changed", vec!["backend".into()]);
                input.base_revision = Some(card.revision as u64);
                f.k.correct(&f.alice, "demo", card.id, input).await.unwrap();
            }
            "device" => {
                f.k.device_revoke(&f.bob, "demo", who.device_id, 1)
                    .await
                    .unwrap();
            }
            "membership" => {
                f.k.grant(&f.bob, "demo", &["frontend".into()], true)
                    .await
                    .unwrap();
            }
            "source_removal" => {
                sqlx::query("UPDATE code_rationale_anchors SET revoked=true,revision=revision+1 WHERE id=$1").bind(anchor.id).execute(&f.pool).await.unwrap();
            }
            "new_edge" => {
                let other =
                    f.k.submit(&f.alice, "demo", body("consumer", vec!["backend".into()]))
                        .await
                        .unwrap();
                f.k.publish(&f.bob, "demo", other.id).await.unwrap();
                let a =
                    f.k.propose_rationale_anchor(
                        &f.bob,
                        "demo",
                        AnchorInput {
                            repository_id: repo,
                            card_id: other.id,
                            card_revision: other.revision,
                            code_revision: "a".repeat(40),
                            targets: vec![wk_contracts::change_context::CodeTarget {
                                path: "backend/consumer.rs".into(),
                                start: None,
                                end: None,
                            }],
                            contribution_id: None,
                        },
                    )
                    .await
                    .unwrap();
                let a =
                    f.k.confirm_rationale_anchor(&f.alice, "demo", a.id, a.revision)
                        .await
                        .unwrap();
                f.k.review_rationale_anchor(&f.bob, "demo", a.id, a.revision)
                    .await
                    .unwrap();
                f.k.relate_cards(
                    &f.bob,
                    "demo",
                    wk_knowledge::graph::CardRelationship {
                        source_card: other.id,
                        target_card: card.id,
                        source_revision: other.revision,
                        target_revision: card.revision,
                        kind: "depends_on".into(),
                    },
                )
                .await
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            f.k.consume_preflight(&f.bob, "demo", who, id, op)
                .await
                .is_err(),
            "changed {change}"
        );
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn a_device_cannot_acknowledge_and_its_service_ceiling_limits_a_wide_member() {
    let f = common::setup().await;
    let (repo, m) = repository(&f).await;
    confirmed(&f, repo, &m).await;
    let who = device(&f).await;
    let mut hidden = body("hidden", vec!["frontend".into()]);
    hidden.rationale = "PRIVATE_CANARY".into();
    let hidden = f.k.submit(&f.alice, "demo", hidden).await.unwrap();
    f.k.publish(&f.bob, "demo", hidden.id).await.unwrap();
    let a =
        f.k.propose_rationale_anchor(
            &f.bob,
            "demo",
            AnchorInput {
                repository_id: repo,
                card_id: hidden.id,
                card_revision: hidden.revision,
                code_revision: "a".repeat(40),
                targets: targets(),
                contribution_id: None,
            },
        )
        .await
        .unwrap();
    let a =
        f.k.confirm_rationale_anchor(&f.alice, "demo", a.id, a.revision)
            .await
            .unwrap();
    f.k.review_rationale_anchor(&f.bob, "demo", a.id, a.revision)
        .await
        .unwrap();
    let device = f.k.device_get(&f.bob, "demo", who.device_id).await.unwrap();
    let bound =
        f.k.device_bound(&device, wk_knowledge::devices::DevicePermission::Read);
    let b = bound
        .create_preflight(&f.bob, "demo", who, protected_request(repo))
        .await
        .unwrap();
    assert!(!serde_json::to_string(&b)
        .unwrap()
        .contains("PRIVATE_CANARY"));
    assert_eq!(
        bound
            .acknowledge_preflight(&f.bob, "demo", b.brief_id.unwrap())
            .await
            .unwrap_err(),
        Error::Denied
    );
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn storage_outage_never_becomes_an_empty_allowing_brief() {
    let f = common::setup().await;
    let (repo, _) = repository(&f).await;
    let who = device(&f).await;
    let (k, pool, role) = common::runtime(&f).await;
    pool.close().await;
    assert_eq!(
        k.create_preflight(&f.bob, "demo", who, protected_request(repo))
            .await
            .unwrap_err(),
        Error::Storage
    );
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn changed_profile_before_acknowledgement_and_runtime_epoch_write_are_rejected() {
    let f = common::setup().await;
    let (repo, m) = repository(&f).await;
    confirmed(&f, repo, &m).await;
    let who = device(&f).await;
    let (k, pool, role) = common::runtime(&f).await;
    let b = k
        .create_preflight(&f.bob, "demo", who, protected_request(repo))
        .await
        .unwrap();
    f.k.put_code_profile(
        &f.alice,
        "demo",
        ProfileInput {
            display_name: None,
            contact_url: None,
            services: vec!["backend".into()],
            base_revision: Some(1),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        k.acknowledge_preflight(&f.bob, "demo", b.brief_id.unwrap())
            .await
            .unwrap_err(),
        Error::Conflict
    );
    assert!(sqlx::query(
        "UPDATE collector_security_epoch SET not_before=not_before+1 WHERE singleton"
    )
    .execute(&pool)
    .await
    .is_err());
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}
