use wk_contracts::CardSubmission;
use wk_knowledge::{Actor, Error, Knowledge};
fn card(key: &str, services: Vec<&str>) -> CardSubmission {
    serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Tenant rate limiter","intent":"Prevent overload","rationale":"Shared middleware","decisions":["tenant limits"],"constraints":[],"open_questions":[],"service_ids":services,"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()
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
    k.grant(&a, &p, &["backend".into(), "frontend".into()], true)
        .await
        .unwrap();
    k.grant(&b, &p, &["backend".into()], false).await.unwrap();
    (k, a, b, p)
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn private_drafts_publish_only_by_reviewer_then_revocation_applies() {
    let (k, a, b, p) = setup().await;
    let c = k.submit(&a, &p, card("1", vec!["backend"])).await.unwrap();
    assert!(k.search(&b, &p, "limiter", 10).await.unwrap().is_empty());
    assert_eq!(k.publish(&b, &p, c.id).await.unwrap_err(), Error::Denied);
    k.publish(&a, &p, c.id).await.unwrap();
    assert_eq!(k.search(&b, &p, "limiter", 10).await.unwrap().len(), 1);
    k.revoke(&b, &p).await.unwrap();
    assert!(k.search(&b, &p, "limiter", 10).await.unwrap().is_empty());
    assert_eq!(k.get(&b, &p, c.id).await.unwrap_err(), Error::NotFound);
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn requires_every_service_and_same_organization() {
    let (k, a, b, p) = setup().await;
    let c = k
        .submit(&a, &p, card("1", vec!["backend", "frontend"]))
        .await
        .unwrap();
    k.publish(&a, &p, c.id).await.unwrap();
    assert!(k.search(&b, &p, "limiter", 10).await.unwrap().is_empty());
    let other = Actor {
        org: "different".into(),
        user: "a".into(),
    };
    assert_eq!(k.get(&other, &p, c.id).await.unwrap_err(), Error::NotFound);
    assert_eq!(
        k.submit(&b, &p, card("bad", vec!["frontend"]))
            .await
            .unwrap_err(),
        Error::Denied
    );
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn submissions_are_idempotent_and_stale_policy_denied() {
    let (k, a, _, p) = setup().await;
    let c = k
        .submit(&a, &p, card("repeat", vec!["backend"]))
        .await
        .unwrap();
    assert_eq!(
        k.submit(&a, &p, card("repeat", vec!["backend"]))
            .await
            .unwrap()
            .id,
        c.id
    );
    let mut changed = card("repeat", vec!["backend"]);
    changed.title = "Different".into();
    assert_eq!(
        k.submit(&a, &p, changed).await.unwrap_err(),
        Error::Conflict
    );
    let mut stale = card("stale", vec!["backend"]);
    stale.policy_revision = 2;
    assert_eq!(k.submit(&a, &p, stale).await.unwrap_err(), Error::Denied);
}
