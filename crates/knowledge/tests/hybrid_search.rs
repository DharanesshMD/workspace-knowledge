mod common;
use wk_contracts::CardSubmission;
use wk_knowledge::{semantic::Namespace, Error};
fn namespace() -> Namespace {
    Namespace {
        model: "nomic-embed-text:v1.5".into(),
        digest: "a".repeat(64),
        dimensions: 3,
    }
}
fn card(key: &str, services: Vec<&str>) -> CardSubmission {
    serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Idempotency key","intent":"Bound retries","rationale":"Protect payment operations","decisions":[],"constraints":[],"open_questions":[],"service_ids":services,"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()
}
#[test]
fn vectors_that_overflow_distance_math_are_rejected() {
    assert!(namespace().vector(&[f32::MAX, 1., 1.]).is_err());
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn another_tenants_nearest_vector_and_owner_notes_are_not_candidates() {
    let f = common::setup().await;
    let other = common::setup().await;
    let ns = namespace();
    f.k.semantic_migrate().await.unwrap();
    let hidden = other
        .k
        .submit(&other.alice, "demo", card("other-org", vec!["backend"]))
        .await
        .unwrap();
    other
        .k
        .publish(&other.bob, "demo", hidden.id)
        .await
        .unwrap();
    let source = other
        .k
        .semantic_pending(&other.alice.org, "demo", &ns)
        .await
        .unwrap()
        .remove(0);
    other
        .k
        .semantic_store(&other.alice.org, "demo", &ns, &source, &[1., 0., 0.])
        .await
        .unwrap();
    f.k.personal_put(
        &f.alice,
        "demo",
        wk_knowledge::personal::PersonalInput {
            body: card("owner-note", vec!["backend"]),
            provenance: wk_knowledge::personal::PersonalProvenance::Captured,
        },
    )
    .await
    .unwrap();
    assert!(f
        .k
        .semantic_pending(&f.alice.org, "demo", &ns)
        .await
        .unwrap()
        .is_empty());
    assert!(f
        .k
        .semantic_candidates(&f.reader, "demo", &ns, &[1., 0., 0.], 5)
        .await
        .unwrap()
        .is_empty());
    let visible =
        f.k.submit(&f.alice, "demo", card("visible", vec!["backend"]))
            .await
            .unwrap();
    f.k.publish(&f.bob, "demo", visible.id).await.unwrap();
    let source =
        f.k.semantic_pending(&f.alice.org, "demo", &ns)
            .await
            .unwrap()
            .remove(0);
    f.k.semantic_store(&f.alice.org, "demo", &ns, &source, &[0.9, 0.1, 0.])
        .await
        .unwrap();
    let found =
        f.k.search_hybrid(&f.reader, "demo", "idempotency", 5, &ns, &[1., 0., 0.])
            .await
            .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, visible.id);
    sqlx::query("UPDATE cards SET body=jsonb_set(body,'{lifecycle}','\"superseded\"') WHERE id=$1")
        .bind(visible.id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f
        .k
        .semantic_candidates(&f.reader, "demo", &ns, &[1., 0., 0.], 5)
        .await
        .unwrap()
        .is_empty());
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn semantic_matches_only_current_permitted_published_cards_and_runtime_is_read_only() {
    let f = common::setup().await;
    let ns = namespace();
    f.k.semantic_migrate().await.unwrap();
    let backend =
        f.k.submit(&f.alice, "demo", card("backend", vec!["backend"]))
            .await
            .unwrap();
    let frontend =
        f.k.submit(&f.alice, "demo", card("frontend", vec!["frontend"]))
            .await
            .unwrap();
    let mixed =
        f.k.submit(&f.alice, "demo", card("mixed", vec!["backend", "frontend"]))
            .await
            .unwrap();
    let draft =
        f.k.submit(&f.alice, "demo", card("private-draft", vec!["backend"]))
            .await
            .unwrap();
    for c in [&backend, &frontend, &mixed] {
        f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    }
    let sources =
        f.k.semantic_pending(&f.alice.org, "demo", &ns)
            .await
            .unwrap();
    assert!(!sources.iter().any(|s| s.id == draft.id));
    for source in &sources {
        let v = if source.id == backend.id {
            vec![0.9, 0.1, 0.]
        } else {
            vec![1., 0., 0.]
        };
        assert!(f
            .k
            .semantic_store(&f.alice.org, "demo", &ns, source, &v)
            .await
            .unwrap());
    }
    let (runtime, pool, role) = common::runtime(&f).await;
    sqlx::raw_sql(
        &include_str!("../../../deploy/scripts/semantic-role.sql").replace("wk_app", &role),
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(runtime.semantic_migrate().await.is_err());
    assert!(runtime
        .semantic_pending(&f.alice.org, "demo", &ns)
        .await
        .is_err());
    assert!(f
        .k
        .search(&f.reader, "demo", "duplicate money", 5)
        .await
        .unwrap()
        .is_empty());
    let found = runtime
        .search_hybrid(&f.reader, "demo", "duplicate money", 5, &ns, &[1., 0., 0.])
        .await
        .unwrap();
    assert_eq!(
        found.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![backend.id]
    );
    assert!(runtime
        .semantic_candidates(&f.reader, "demo", &ns, &[-1., 0., 0.], 5)
        .await
        .unwrap()
        .is_empty());
    let readable: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT card_id FROM card_embeddings")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(readable.is_empty()); // no transaction actor means no RLS visibility
    let old = sources.iter().find(|s| s.id == backend.id).unwrap();
    let mut correction = card("corrected", vec!["backend"]);
    correction.base_revision = Some(1);
    f.k.correct(&f.alice, "demo", backend.id, correction)
        .await
        .unwrap();
    assert!(!f
        .k
        .semantic_store(&f.alice.org, "demo", &ns, old, &[1., 0., 0.])
        .await
        .unwrap());
    assert!(runtime
        .search_hybrid(&f.reader, "demo", "duplicate money", 5, &ns, &[1., 0., 0.])
        .await
        .unwrap()
        .is_empty());
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM card_embeddings WHERE card_id=$1")
            .bind(backend.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
    assert!(runtime
        .semantic_store(&f.alice.org, "demo", &ns, old, &[1., 0., 0.])
        .await
        .is_err());
    let device =
        f.k.device_register(
            &f.alice,
            wk_contracts::device::DeviceRegistration {
                id: uuid::Uuid::new_v4(),
                public_key: "a".repeat(64),
                approval_hash: "b".repeat(64),
                product: "demo".into(),
                services: vec!["backend".into()],
            },
        )
        .await
        .unwrap();
    let guarded =
        f.k.device_bound(&device, wk_knowledge::devices::DevicePermission::Read);
    assert!(guarded
        .search(&f.alice, "demo", "idempotency", 5)
        .await
        .unwrap()
        .is_empty());
    common::cleanup_runtime(&f, pool, role).await;
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn namespace_failures_retries_withdrawal_and_revocation_do_not_reuse_vectors() {
    let f = common::setup().await;
    let ns = namespace();
    f.k.semantic_migrate().await.unwrap();
    let c =
        f.k.submit(&f.alice, "demo", card("one", vec!["backend"]))
            .await
            .unwrap();
    f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    let source =
        f.k.semantic_pending(&f.alice.org, "demo", &ns)
            .await
            .unwrap()
            .remove(0);
    assert!(f
        .k
        .semantic_store(&f.alice.org, "demo", &ns, &source, &[0., 0., 0.])
        .await
        .is_err());
    assert!(f
        .k
        .semantic_store(&f.alice.org, "demo", &ns, &source, &[1., 0.])
        .await
        .is_err());
    f.k.semantic_failed(&f.alice.org, "demo", &ns, &source)
        .await
        .unwrap();
    assert!(f
        .k
        .semantic_pending(&f.alice.org, "demo", &ns)
        .await
        .unwrap()
        .is_empty());
    assert!(f
        .k
        .semantic_store(&f.alice.org, "demo", &ns, &source, &[1., 0., 0.])
        .await
        .unwrap());
    let mut other = ns.clone();
    other.digest = "b".repeat(64);
    assert!(f
        .k
        .semantic_candidates(&f.reader, "demo", &other, &[1., 0., 0.], 5)
        .await
        .unwrap()
        .is_empty());
    f.k.withdraw(&f.alice, "demo", c.id, 1).await.unwrap();
    assert!(!f
        .k
        .semantic_store(&f.alice.org, "demo", &ns, &source, &[1., 0., 0.])
        .await
        .unwrap());
    assert!(f
        .k
        .semantic_candidates(&f.reader, "demo", &ns, &[1., 0., 0.], 5)
        .await
        .unwrap()
        .is_empty());
    f.k.revoke(&f.reader, "demo").await.unwrap();
    assert!(matches!(
        f.k.search_hybrid(&f.reader, "demo", "duplicate money", 5, &ns, &[1., 0., 0.])
            .await,
        Err(Error::NotFound)
    ));
}
