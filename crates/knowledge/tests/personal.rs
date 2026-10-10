use wk_contracts::CardSubmission;
use wk_knowledge::{
    personal::{PersonalInput, PersonalProvenance},
    Actor, Error, Knowledge,
};
static ROLE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn input(key: &str) -> PersonalInput {
    PersonalInput{body:serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Tenant rate limiter","intent":"Prevent overload","rationale":"Shared middleware","decisions":["tenant limits"],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap(),provenance:PersonalProvenance::Captured}
}
async fn setup() -> (Knowledge, Actor, Actor, sqlx::PgPool) {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let a = Actor {
        org: org.clone(),
        user: "alice".into(),
    };
    let b = Actor {
        org,
        user: "reviewer".into(),
    };
    for who in [&a, &b] {
        k.grant(who, "demo", &["backend".into(), "frontend".into()], true)
            .await
            .unwrap();
    }
    (k, a, b, pool)
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn personal_summaries_remain_owner_only_with_retry_revision_and_deletion() {
    let (k, a, b, pool) = setup().await;
    let note = k.personal_put(&a, "demo", input("one")).await.unwrap();
    assert_eq!(
        k.personal_get(&a, "demo", note.id).await.unwrap().id,
        note.id
    );
    assert_eq!(
        k.personal_get(&b, "demo", note.id).await.unwrap_err(),
        Error::NotFound
    );
    assert!(k.personal_list(&b, "demo", None).await.unwrap().is_empty());
    let other = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: a.user.clone(),
    };
    assert_eq!(
        k.personal_get(&other, "demo", note.id).await.unwrap_err(),
        Error::NotFound
    );
    assert!(k
        .search(&a, "demo", "limiter", 10)
        .await
        .unwrap()
        .is_empty());
    assert!(k.review_queue(&b, "demo").await.unwrap().is_empty());
    let same = k.personal_put(&a, "demo", input("one")).await.unwrap();
    assert_eq!(same.id, note.id);
    assert_eq!(same.expires_at_ms, note.expires_at_ms);
    let mut changed = input("one");
    changed.body.intent = "Different intent".into();
    assert_eq!(
        k.personal_put(&a, "demo", changed).await.unwrap_err(),
        Error::Conflict
    );
    let mut edited: CardSubmission = input("one").body;
    edited.intent = "Keep bounded load".into();
    assert_eq!(
        k.personal_edit(&a, "demo", note.id, 9, edited.clone())
            .await
            .unwrap_err(),
        Error::Conflict
    );
    let next = k
        .personal_edit(&a, "demo", note.id, 1, edited.clone())
        .await
        .unwrap();
    assert_eq!(next.revision, 2);
    assert_eq!(next.expires_at_ms, note.expires_at_ms);
    assert_eq!(
        k.personal_edit(&b, "demo", note.id, 2, edited)
            .await
            .unwrap_err(),
        Error::NotFound
    );
    k.personal_delete(&a, "demo", note.id, 2).await.unwrap();
    assert_eq!(
        k.personal_get(&a, "demo", note.id).await.unwrap_err(),
        Error::NotFound
    );
    assert_eq!(
        k.personal_put(&a, "demo", input("one")).await.unwrap_err(),
        Error::Conflict
    );
    let value: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT body FROM personal_summaries WHERE id=$1")
            .bind(note.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(value.is_none());
    let revoked = k.personal_put(&a, "demo", input("revoked")).await.unwrap();
    k.revoke(&a, "demo").await.unwrap();
    assert_eq!(
        k.personal_get(&a, "demo", revoked.id).await.unwrap_err(),
        Error::NotFound
    );
    assert!(k.personal_list(&a, "demo", None).await.unwrap().is_empty());
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn personal_expiry_screening_and_restricted_role_are_enforced() {
    let _role_setup_guard = ROLE_TEST_LOCK.lock().await;
    let (k, a, b, owner) = setup().await;
    let mut partial = input("partial");
    partial.body.capture_completeness = wk_contracts::Completeness::Partial;
    assert_eq!(
        k.personal_put(&a, "demo", partial).await.unwrap_err(),
        Error::Invalid
    );
    let mut secret = input("secret");
    secret.body.intent = "sk-proj-THIS_IS_A_SYNTHETIC_SECRET_CANARY_012345678901234567890".into();
    assert_eq!(
        k.personal_put(&a, "demo", secret).await.unwrap_err(),
        Error::Invalid
    );
    let mut manual = input("manual");
    manual.provenance = PersonalProvenance::Manual;
    manual.body.lifecycle = wk_contracts::Lifecycle::InProgress;
    assert_eq!(
        k.personal_put(&a, "demo", manual).await.unwrap_err(),
        Error::Invalid
    );
    let expired = k.personal_put(&a, "demo", input("expired")).await.unwrap();
    sqlx::query("UPDATE personal_summaries SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(expired.id)
        .execute(&owner)
        .await
        .unwrap();
    assert_eq!(
        k.personal_get(&a, "demo", expired.id).await.unwrap_err(),
        Error::NotFound
    );
    assert_eq!(
        k.personal_put(&a, "demo", input("expired"))
            .await
            .unwrap_err(),
        Error::Conflict
    );
    let role = format!("wk_personal_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(&owner)
        .await
        .unwrap();
    sqlx::raw_sql(
        &include_str!("../../../deploy/scripts/application-role.sql").replace("wk_app", &role),
    )
    .execute(&owner)
    .await
    .unwrap();
    let selected = role.clone();
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _| {
            let role = selected.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let runtime = Knowledge::from_pool(pool.clone());
    let note = runtime
        .personal_put(&a, "demo", input("runtime"))
        .await
        .unwrap();
    assert_eq!(
        runtime.personal_get(&b, "demo", note.id).await.unwrap_err(),
        Error::NotFound
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM personal_summaries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    runtime
        .personal_delete(&a, "demo", note.id, 1)
        .await
        .unwrap();
    pool.close().await;
    sqlx::raw_sql(&format!("DROP OWNED BY {role}; DROP ROLE {role};"))
        .execute(&owner)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn personal_pagination_and_quota_include_inaccessible_service_records() {
    let _role_setup_guard = ROLE_TEST_LOCK.lock().await;
    let (k, a, _, pool) = setup().await;
    let first = k.personal_put(&a, "demo", input("seed")).await.unwrap();
    sqlx::query("INSERT INTO personal_summaries(id,org,product,owner,idempotency_key,services,body,provenance,created_at) SELECT gen_random_uuid(),org,product,owner,'batch-'||n,ARRAY['frontend'],jsonb_set(jsonb_set(body,'{service_ids}','[\"frontend\"]'::jsonb),'{idempotency_key}',to_jsonb('batch-'||n)),'captured',now()-n*interval '1 second' FROM personal_summaries CROSS JOIN generate_series(1,1999) n WHERE id=$1").bind(first.id).execute(&pool).await.unwrap();
    let page = k.personal_list(&a, "demo", None).await.unwrap();
    assert_eq!(page.len(), 50);
    assert_eq!(page[0].id, first.id);
    let next = k
        .personal_list(&a, "demo", page.last().map(|n| n.id))
        .await
        .unwrap();
    assert_eq!(next.len(), 50);
    assert!(next.iter().all(|n| !page.iter().any(|p| p.id == n.id)));
    assert_eq!(
        k.personal_put(&a, "demo", input("over-quota"))
            .await
            .unwrap_err(),
        Error::Conflict
    );
    k.grant(&a, "demo", &["backend".into()], false)
        .await
        .unwrap();
    let mut body = input("narrowed-quota");
    body.body.policy_revision = 2;
    assert_eq!(
        restricted(&pool)
            .await
            .personal_put(&a, "demo", body)
            .await
            .unwrap_err(),
        Error::Conflict
    );
}

async fn restricted(owner: &sqlx::PgPool) -> Knowledge {
    let role = format!("wk_quota_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(owner)
        .await
        .unwrap();
    sqlx::raw_sql(
        &include_str!("../../../deploy/scripts/application-role.sql").replace("wk_app", &role),
    )
    .execute(owner)
    .await
    .unwrap();
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _| {
            let role = role.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    Knowledge::from_pool(pool)
}
