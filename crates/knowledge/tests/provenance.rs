use wk_contracts::CardSubmission;
use wk_knowledge::{Actor, Error, Knowledge};

fn card(key: &str, completeness: &str, provenance: Option<&str>) -> CardSubmission {
    let mut v = serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Cap retries","intent":"Avoid repeat charges","rationale":"The gateway charges twice on duplicate calls.","decisions":["cap at three"],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":completeness,"base_revision":null});
    if let Some(p) = provenance {
        v["provenance"] = serde_json::json!(p);
    }
    serde_json::from_value(v).unwrap()
}
async fn setup() -> (Knowledge, Actor, Actor, String) {
    let url = std::env::var("WK_TEST_DATABASE_URL")
        .expect("set WK_TEST_DATABASE_URL to a disposable /wk_test database");
    assert!(
        url.split('?').next().unwrap().ends_with("/wk_test"),
        "refusing non-test database"
    );
    let k = Knowledge::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let a = Actor {
        org: org.clone(),
        user: "a".into(),
    };
    let b = Actor {
        org,
        user: "b".into(),
    };
    let p = "product".to_string();
    k.grant(&a, &p, &["backend".into()], true).await.unwrap();
    k.grant(&b, &p, &["backend".into()], false).await.unwrap();
    (k, a, b, p)
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn reviewer_publishes_an_assistant_written_partial_card() {
    let (k, a, b, p) = setup().await;
    let c = k
        .submit(&a, &p, card("1", "partial", Some("assistant_written")))
        .await
        .unwrap();
    assert_eq!(
        k.publish(&b, &p, c.id).await.unwrap_err(),
        Error::Denied,
        "non-reviewer"
    );
    k.publish(&a, &p, c.id).await.unwrap();
    assert_eq!(k.search(&b, &p, "retries", 10).await.unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn other_partial_cards_stay_unpublishable() {
    let (k, a, _b, p) = setup().await;
    let plain = k.submit(&a, &p, card("2", "partial", None)).await.unwrap();
    assert_eq!(
        k.publish(&a, &p, plain.id).await.unwrap_err(),
        Error::Denied
    );
    // `validate` already refuses truncated + provenance, so submit is the guard.
    assert_eq!(
        k.submit(&a, &p, card("3", "truncated", Some("assistant_written")))
            .await
            .unwrap_err(),
        Error::Invalid
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn correct_keeps_provenance_and_cannot_change_it() {
    let (k, a, _b, p) = setup().await;
    let c = k
        .submit(&a, &p, card("4", "partial", Some("assistant_written")))
        .await
        .unwrap();
    k.publish(&a, &p, c.id).await.unwrap();
    let mut next = card("4", "partial", Some("assistant_written"));
    next.base_revision = Some(1);
    next.intent = "Revised scope".into();
    let updated = k.correct(&a, &p, c.id, next).await.unwrap();
    assert!(!updated.published);
    assert_eq!(
        updated.body.provenance,
        Some(wk_contracts::Provenance::AssistantWritten)
    );
    // The gate holds on the corrected row.
    k.publish(&a, &p, c.id).await.unwrap();
    // Adding provenance to a plain partial card, or removing it, is refused.
    let plain = k.submit(&a, &p, card("5", "partial", None)).await.unwrap();
    let mut add = card("5", "partial", Some("assistant_written"));
    add.base_revision = Some(1);
    assert_eq!(
        k.correct(&a, &p, plain.id, add).await.unwrap_err(),
        Error::Invalid
    );
    let mut drop = card("4", "partial", None);
    drop.base_revision = Some(2);
    assert_eq!(
        k.correct(&a, &p, c.id, drop).await.unwrap_err(),
        Error::Invalid
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn publish_rescreens_the_stored_text() {
    let (k, a, _b, p) = setup().await;
    let c = k
        .submit(&a, &p, card("6", "partial", Some("assistant_written")))
        .await
        .unwrap();
    // Simulate a row edited after submit: the screen at publish must still refuse it.
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query(
        "UPDATE cards SET body=jsonb_set(body,'{rationale}','\"password = CANARY\"') WHERE id=$1",
    )
    .bind(c.id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(k.publish(&a, &p, c.id).await.unwrap_err(), Error::Invalid);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn publish_gate_refuses_truncated_even_with_provenance() {
    let (k, a, _b, p) = setup().await;
    let c = k
        .submit(&a, &p, card("7", "partial", Some("assistant_written")))
        .await
        .unwrap();
    // Simulate a row edited after submit: the SQL gate itself must refuse truncated.
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query("UPDATE cards SET body=jsonb_set(body,'{capture_completeness}','\"truncated\"') WHERE id=$1")
        .bind(c.id).execute(&pool).await.unwrap();
    assert_eq!(k.publish(&a, &p, c.id).await.unwrap_err(), Error::Denied);
}
