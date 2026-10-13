mod common;
use common::code::*;
use wk_contracts::change_context::*;
use wk_knowledge::{code_provenance::*, graph::CardRelationship, Error};
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn exact_scoped_context_separates_contributor_rationale_maintainer_and_incomplete_coverage() {
    let f = common::setup().await;
    let (repo, m) = repository(&f).await;
    let (_, a) = confirmed(&f, repo, &m).await;
    f.k.put_code_profile(
        &f.bob,
        "demo",
        ProfileInput {
            display_name: Some("Bob Synthetic".into()),
            contact_url: None,
            services: vec!["backend".into()],
            base_revision: None,
        },
    )
    .await
    .unwrap();
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
    let (k, pool, role) = common::runtime(&f).await;
    let b = k
        .change_context(&f.reader, "demo", request(repo))
        .await
        .unwrap();
    assert_eq!(b.decision, Decision::NeedsReview);
    assert_eq!(b.coverage, "reviewed records only");
    assert!(b.gaps.contains(&"dependency_coverage_incomplete".into()));
    assert!(b.reasons.contains(&"high_risk_path".into()));
    assert!(b
        .items
        .iter()
        .any(|i| i.role == "contributor" && i.display_name.as_deref() == Some("Alice Synthetic")));
    assert!(b.items.iter().any(|i| i.role == "rationale_author"
        && i.text.contains("deduplicates")
        && i.sources.iter().any(|s| s.id == a.id.to_string())));
    assert!(b
        .items
        .iter()
        .any(|i| i.role == "maintainer" && i.display_name.as_deref() == Some("Bob Synthetic")));
    assert!(!b
        .items
        .iter()
        .any(|i| i.role == "contributor" && i.user_id.as_deref() == Some("bob")));
    assert!(k
        .change_context(&f.alice, "demo", request(repo))
        .await
        .unwrap()
        .items
        .iter()
        .any(|i| i.role == "contributor"));
    let (other, _) = repository(&f).await;
    let other = k
        .change_context(&f.reader, "demo", request(other))
        .await
        .unwrap();
    assert!(other.items.is_empty());
    assert!(other.reasons.contains(&"reason_not_recorded".into()));
    let mut historical = request(repo);
    historical.base_revision = "b".repeat(40);
    let historical = k
        .change_context(&f.reader, "demo", historical)
        .await
        .unwrap();
    assert!(historical
        .items
        .iter()
        .flat_map(|i| &i.sources)
        .any(|s| s.freshness == "historical"));
    assert!(historical.reasons.contains(&"reason_not_recorded".into()));
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn withdrawn_profile_mapping_and_card_do_not_leave_trusted_names_or_current_intent() {
    let f = common::setup().await;
    let (repo, m) = repository(&f).await;
    let (card, _) = confirmed(&f, repo, &m).await;
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
    let b =
        f.k.change_context(&f.bob, "demo", request(repo))
            .await
            .unwrap();
    assert!(!serde_json::to_string(&b)
        .unwrap()
        .contains("Alice Synthetic"));
    assert!(b.gaps.contains(&"contributor_name_unavailable".into()));
    sqlx::query("UPDATE code_mappings SET revoked=true,revision=revision+1 WHERE id=$1")
        .bind(m.id)
        .execute(&f.pool)
        .await
        .unwrap();
    let b =
        f.k.change_context(&f.bob, "demo", request(repo))
            .await
            .unwrap();
    assert!(!b.items.iter().any(|i| i.role == "contributor"));
    f.k.withdraw(&f.alice, "demo", card.id, card.revision)
        .await
        .unwrap();
    let b =
        f.k.change_context(&f.bob, "demo", request(repo))
            .await
            .unwrap();
    assert!(!b.items.iter().any(|i| i.role == "rationale_author"));
    assert!(b.reasons.contains(&"reason_not_recorded".into()));
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn reviewed_edges_and_test_references_require_both_current_visible_bound_sources() {
    let f = common::setup().await;
    let (repo, m) = repository(&f).await;
    let (card, _) = confirmed(&f, repo, &m).await;
    for (key, services) in [
        ("consumer", vec!["backend".into()]),
        ("PRIVATE_CANARY", vec!["frontend".into()]),
    ] {
        let mut input = body(key, services);
        input.title = key.into();
        let other = f.k.submit(&f.alice, "demo", input).await.unwrap();
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
                    targets: vec![CodeTarget {
                        path: format!("backend/{key}.rs"),
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
            CardRelationship {
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
    f.k.attest(&f.bob,"demo",card.id,card.revision,wk_knowledge::evidence::VerifiedEvidence{provider:"synthetic".into(),reference:"https://ci.example.invalid/run/1".into(),code_revision:"b".repeat(40),kind:"tests_verified".into(),metadata:serde_json::json!({"passed":true,"command":"cargo test retry","executed_at":"2026-10-11T00:00:00Z","environment":"synthetic"})},&["ci.example.invalid".into()]).await.unwrap();
    let (k, pool, role) = common::runtime(&f).await;
    let b = k
        .change_context(&f.reader, "demo", request(repo))
        .await
        .unwrap();
    assert!(b
        .items
        .iter()
        .any(|i| i.role == "impact" && i.text.contains("consumer")));
    assert!(!serde_json::to_string(&b)
        .unwrap()
        .contains("PRIVATE_CANARY"));
    assert!(b
        .items
        .iter()
        .any(|i| i.role == "test" && i.sources.iter().any(|s| s.freshness == "historical")));
    assert!(b.gaps.contains(&"current_tests_not_recorded".into()));
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn missing_and_heuristic_only_context_remains_unknown_and_budget_preserves_gaps() {
    let f = common::setup().await;
    let (repo, m) = repository(&f).await;
    let unbound =
        f.k.submit(&f.alice, "demo", body("legacy", vec!["backend".into()]))
            .await
            .unwrap();
    f.k.publish(&f.bob, "demo", unbound.id).await.unwrap();
    let b =
        f.k.change_context(&f.bob, "demo", request(repo))
            .await
            .unwrap();
    assert!(b.items.is_empty());
    let (card, _) = confirmed(&f, repo, &m).await;
    sqlx::query("UPDATE cards SET body=jsonb_set(body,'{rationale}',to_jsonb(repeat('Oversized rationale ',5000))) WHERE id=$1").bind(card.id).execute(&f.pool).await.unwrap();
    let mut r = request(repo);
    r.budget_bytes = 1024;
    let b = f.k.change_context(&f.bob, "demo", r).await.unwrap();
    assert!(serde_json::to_vec(&b).unwrap().len() <= 1024);
    assert!(b.truncated);
    assert!(b.gaps.contains(&"dependency_coverage_incomplete".into()));
    let foreign = wk_knowledge::Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: f.bob.user.clone(),
    };
    assert!(matches!(
        f.k.change_context(&foreign, "demo", request(repo)).await,
        Err(Error::NotFound | Error::Denied)
    ));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn correcting_and_republishing_a_card_does_not_reuse_old_author_confirmation() {
    let f = common::setup().await;
    let (repo, m) = repository(&f).await;
    let (card, _) = confirmed(&f, repo, &m).await;
    let mut changed = body("correction", vec!["backend".into()]);
    changed.base_revision = Some(card.revision as u64);
    changed.rationale = "A newly reviewed operation contract".into();
    let changed =
        f.k.correct(&f.alice, "demo", card.id, changed)
            .await
            .unwrap();
    f.k.publish(&f.bob, "demo", changed.id).await.unwrap();
    let b =
        f.k.change_context(&f.reader, "demo", request(repo))
            .await
            .unwrap();
    assert!(!b.items.iter().any(|i| i.role == "rationale_author"));
    assert!(b.reasons.contains(&"reason_not_recorded".into()));
    let (k, pool, role) = common::runtime(&f).await;
    f.k.grant(&f.reader, "demo", &["frontend".into()], false)
        .await
        .unwrap();
    assert!(k
        .change_context(&f.reader, "demo", request(repo))
        .await
        .is_err());
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}
