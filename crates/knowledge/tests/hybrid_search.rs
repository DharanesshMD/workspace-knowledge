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
async fn stored(f: &common::Fixture, ns: &Namespace, vectors: &[(uuid::Uuid, [f32; 3])]) {
    for source in
        f.k.semantic_pending(&f.alice.org, "demo", ns)
            .await
            .unwrap()
    {
        let (_, v) = vectors.iter().find(|(id, _)| *id == source.id).unwrap();
        assert!(f
            .k
            .semantic_store(&f.alice.org, "demo", ns, &source, v)
            .await
            .unwrap());
    }
}
async fn published(f: &common::Fixture, key: &str, services: Vec<&str>) -> uuid::Uuid {
    let c =
        f.k.submit(&f.alice, "demo", card(key, services))
            .await
            .unwrap();
    f.k.publish(&f.bob, "demo", c.id).await.unwrap();
    c.id
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn related_by_meaning_returns_other_developers_close_summaries() {
    let f = common::setup().await;
    let ns = namespace();
    f.k.semantic_migrate().await.unwrap();
    let near = published(&f, "near", vec!["backend"]).await;
    let far = published(&f, "far", vec!["backend"]).await;
    // At a 0.6 floor, cosine similarity 0.8 (distance 0.2) is inside and ~0.55 is outside.
    let close = published(&f, "close", vec!["backend"]).await;
    let below = published(&f, "below", vec!["backend"]).await;
    let frontend = published(&f, "frontend", vec!["frontend"]).await;
    stored(
        &f,
        &ns,
        &[
            (near, [1., 0., 0.]),
            (far, [0., 1., 0.]),
            (close, [0.8, 0.6, 0.]),
            (below, [0.55, 0.835, 0.]),
            (frontend, [1., 0., 0.]),
        ],
    )
    .await;
    let related = |a: &wk_knowledge::Actor, exclude: Vec<uuid::Uuid>, limit: u32| {
        let (k, ns, a) = (f.k.clone(), ns.clone(), a.clone());
        async move {
            k.related_by_meaning(&a, "demo", &ns, &[1., 0., 0.], &exclude, limit, 0.6)
                .await
        }
    };
    let ids = |n: Vec<wk_knowledge::activity::Notice>| {
        n.into_iter()
            .map(|n| n.card_id.unwrap())
            .collect::<Vec<_>>()
    };
    let found = related(&f.reader, vec![], 20).await.unwrap();
    assert!(found
        .iter()
        .all(|n| n.kind == "related" && n.matched_by == "meaning" && n.developer == "alice"));
    // The backend-only reader never sees the frontend card, even at distance 0.
    assert_eq!(ids(found), vec![near, close]);
    let mut wide = ids(related(&f.bob, vec![], 20).await.unwrap());
    wide.sort();
    let mut expected = vec![near, close, frontend];
    expected.sort();
    assert_eq!(wide, expected);
    assert!(related(&f.alice, vec![], 20).await.unwrap().is_empty());
    assert_eq!(
        ids(related(&f.reader, vec![near], 20).await.unwrap()),
        vec![close]
    );
    for (exclude, limit) in [(vec![], 0), (vec![], 21), (vec![near; 101], 20)] {
        assert!(matches!(
            related(&f.reader, exclude, limit).await,
            Err(Error::Invalid)
        ));
    }
    // The caller's floor decides: at 0.5 the ~0.55 card is found too, after the closer ones.
    let at = |floor: f32| {
        let (k, ns, a) = (f.k.clone(), ns.clone(), f.reader.clone());
        async move {
            k.related_by_meaning(&a, "demo", &ns, &[1., 0., 0.], &[], 20, floor)
                .await
        }
    };
    assert_eq!(ids(at(0.5).await.unwrap()), vec![near, close, below]);
    assert_eq!(ids(at(0.85).await.unwrap()), vec![near]);
    for floor in [0.29, 0.951, f32::NAN, f32::INFINITY] {
        assert!(matches!(at(floor).await, Err(Error::Invalid)), "{floor}");
    }
    f.k.withdraw(&f.alice, "demo", near, 1).await.unwrap();
    assert_eq!(
        ids(related(&f.reader, vec![], 20).await.unwrap()),
        vec![close]
    );
    assert!(matches!(
        f.k.related_by_meaning(&f.reader, "elsewhere", &ns, &[1., 0., 0.], &[], 20, 0.6)
            .await,
        Err(Error::NotFound)
    ));
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn related_by_meaning_skips_superseded_and_deprecated_cards() {
    let f = common::setup().await;
    let ns = namespace();
    f.k.semantic_migrate().await.unwrap();
    let current = published(&f, "current", vec!["backend"]).await;
    let superseded = published(&f, "superseded", vec!["backend"]).await;
    let deprecated = published(&f, "deprecated", vec!["backend"]).await;
    stored(
        &f,
        &ns,
        &[
            (current, [0.9, 0.1, 0.]),
            (superseded, [1., 0., 0.]),
            (deprecated, [1., 0., 0.]),
        ],
    )
    .await;
    // A body change clears stored vectors; put a close current-revision one back, so the
    // query's own lifecycle filter is what excludes these cards.
    for (id, lifecycle) in [(superseded, "superseded"), (deprecated, "deprecated")] {
        sqlx::query(
            "UPDATE cards SET body=jsonb_set(body,'{lifecycle}',to_jsonb($2::text)) WHERE id=$1",
        )
        .bind(id)
        .bind(lifecycle)
        .execute(&f.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO card_embeddings(org,product,card_id,revision,model,digest,dimensions,format,embedding,status) SELECT org,product,id,revision,$2,$3,$4,$5,'[1,0,0]'::vector,'ready' FROM cards WHERE id=$1")
            .bind(id)
            .bind(&ns.model)
            .bind(&ns.digest)
            .bind(ns.dimensions as i32)
            .bind(wk_knowledge::semantic::FORMAT)
            .execute(&f.pool)
            .await
            .unwrap();
    }
    let found =
        f.k.related_by_meaning(&f.reader, "demo", &ns, &[1., 0., 0.], &[], 20, 0.6)
            .await
            .unwrap();
    assert_eq!(
        found.iter().map(|n| n.card_id.unwrap()).collect::<Vec<_>>(),
        vec![current]
    );
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL"]
async fn semantic_reads_share_the_product_lock_but_wait_for_administration() {
    let f = common::setup().await;
    let ns = namespace();
    f.k.semantic_migrate().await.unwrap();
    let near = published(&f, "near", vec!["backend"]).await;
    stored(&f, &ns, &[(near, [1., 0., 0.])]).await;
    let gate = |mode: &str| {
        format!("SELECT {mode}(hashtextextended(jsonb_build_array($1::text,$2::text,'administration')::text,0))")
    };
    let read = |k: wk_knowledge::Knowledge| {
        let (a, ns) = (f.reader.clone(), ns.clone());
        async move {
            k.related_by_meaning(&a, "demo", &ns, &[1., 0., 0.], &[], 20, 0.6)
                .await
        }
    };
    // Another reader holds the product gate: two more reads proceed without waiting.
    let mut reader = f.pool.begin().await.unwrap();
    sqlx::query(&gate("pg_advisory_xact_lock_shared"))
        .bind(&f.reader.org)
        .bind("demo")
        .execute(&mut *reader)
        .await
        .unwrap();
    let (one, two) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(read(f.k.clone()), read(f.k.clone()))
    })
    .await
    .expect("concurrent semantic reads must not wait on each other");
    assert_eq!(one.unwrap().len(), 1);
    assert_eq!(two.unwrap().len(), 1);
    reader.rollback().await.unwrap();
    // An administration change holds the gate exclusively: a read waits for it.
    let mut admin = f.pool.begin().await.unwrap();
    sqlx::query(&gate("pg_advisory_xact_lock"))
        .bind(&f.reader.org)
        .bind("demo")
        .execute(&mut *admin)
        .await
        .unwrap();
    let app_name = format!("semantic_gate_{}", uuid::Uuid::new_v4().simple());
    let selected = app_name.clone();
    let waiting_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _| {
            let selected = selected.clone();
            Box::pin(async move {
                sqlx::query("SELECT set_config('application_name',$1,false)")
                    .bind(selected)
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("WK_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let pending = tokio::spawn(read(wk_knowledge::Knowledge::from_pool(waiting_pool)));
    // Wait until the read is seen queued on the advisory gate, bounded by a deadline.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let blocked = loop {
        let waiting = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock' AND wait_event='advisory')")
            .bind(&app_name)
            .fetch_one(&f.pool)
            .await
            .unwrap();
        if waiting || std::time::Instant::now() >= deadline {
            break waiting;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert!(
        blocked,
        "a semantic read must wait for an exclusive administration holder"
    );
    assert!(!pending.is_finished());
    admin.rollback().await.unwrap();
    let after = tokio::time::timeout(std::time::Duration::from_secs(5), pending)
        .await
        .expect("the read resumes once administration commits")
        .unwrap()
        .unwrap();
    assert_eq!(after.len(), 1);
}
