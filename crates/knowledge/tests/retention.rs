use wk_contracts::CardSubmission;
use wk_knowledge::{
    personal::{PersonalInput, PersonalProvenance},
    Actor, Error, Knowledge,
};
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
async fn setup() -> (Knowledge, Actor, sqlx::PgPool) {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let a = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "alice".into(),
    };
    k.grant(&a, "demo", &["backend".into()], false)
        .await
        .unwrap();
    (k, a, pool)
}
fn input(key: &str) -> PersonalInput {
    let body:CardSubmission=serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Synthetic retention note","intent":"Bound retained data","rationale":"","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap();
    PersonalInput {
        body,
        provenance: PersonalProvenance::Captured,
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn deleting_then_restoring_a_missing_note_cannot_recreate_its_key() {
    let _guard = LOCK.lock().await;
    let (k, a, pool) = setup().await;
    let note = k
        .personal_put(&a, "demo", input("deleted-after-snapshot"))
        .await
        .unwrap();
    k.personal_delete(&a, "demo", note.id, 1).await.unwrap();
    let journal: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(d) FROM personal_deletions d WHERE note_id=$1")
            .bind(note.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(journal.get("body").is_none() && journal.get("idempotency_key").is_none());
    assert_eq!(journal["key_hash"].as_str().unwrap().len(), 64);
    sqlx::query("DELETE FROM personal_summaries WHERE id=$1")
        .bind(note.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        k.personal_put(&a, "demo", input("deleted-after-snapshot"))
            .await
            .unwrap_err(),
        Error::Conflict
    );
    let other = Actor {
        org: a.org.clone(),
        user: "bob".into(),
    };
    k.grant(&other, "demo", &["backend".into()], false)
        .await
        .unwrap();
    k.personal_put(&other, "demo", input("deleted-after-snapshot"))
        .await
        .unwrap();
    // Simulate an older restored row using an operator INSERT with its old body.
    sqlx::query("INSERT INTO personal_summaries(id,org,product,owner,idempotency_key,services,body,provenance) VALUES($1,$2,'demo',$3,'deleted-after-snapshot',ARRAY['backend'],$4,'captured')").bind(note.id).bind(&a.org).bind(&a.user).bind(serde_json::to_value(&note.body).unwrap()).execute(&pool).await.unwrap();
    assert!(k.replay_personal_deletions().await.unwrap() >= 1);
    assert_eq!(
        k.personal_get(&a, "demo", note.id).await.unwrap_err(),
        Error::NotFound
    );
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM personal_summaries WHERE id=$1")
        .bind(note.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(revision, 2);
    k.replay_personal_deletions().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT revision FROM personal_summaries WHERE id=$1")
            .bind(note.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        revision
    );
    assert_eq!(
        k.personal_list(&other, "demo", None).await.unwrap().len(),
        1
    );
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn cleanup_is_bounded_clears_expired_text_and_preserves_unexpired_data() {
    let _guard = LOCK.lock().await;
    let (k, a, pool) = setup().await;
    let first = k
        .personal_put(&a, "demo", input("expired-first"))
        .await
        .unwrap();
    let second = k
        .personal_put(&a, "demo", input("expired-second"))
        .await
        .unwrap();
    let live = k.personal_put(&a, "demo", input("live")).await.unwrap();
    sqlx::query("UPDATE personal_summaries SET expires_at=to_timestamp(1) WHERE id=$1")
        .bind(first.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE personal_summaries SET expires_at=to_timestamp(2) WHERE id=$1")
        .bind(second.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(k.retention_batch(0).await.unwrap_err(), Error::Invalid);
    assert_eq!(k.retention_batch(1001).await.unwrap_err(), Error::Invalid);
    let result = k.retention_batch(1).await.unwrap();
    assert_eq!(result.personal, 1);
    let gone: bool =
        sqlx::query_scalar("SELECT deleted AND body IS NULL FROM personal_summaries WHERE id=$1")
            .bind(first.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(gone);
    assert!(sqlx::query_scalar::<_, bool>(
        "SELECT body IS NOT NULL FROM personal_summaries WHERE id=$1"
    )
    .bind(second.id)
    .fetch_one(&pool)
    .await
    .unwrap());
    k.retention_batch(1).await.unwrap();
    assert!(sqlx::query_scalar::<_, bool>(
        "SELECT deleted AND body IS NULL FROM personal_summaries WHERE id=$1"
    )
    .bind(second.id)
    .fetch_one(&pool)
    .await
    .unwrap());
    assert!(k.personal_get(&a, "demo", live.id).await.is_ok());
    assert_eq!(
        k.personal_put(&a, "demo", input("expired-first"))
            .await
            .unwrap_err(),
        Error::Conflict
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM personal_deletions WHERE org=$1")
        .bind(&a.org)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn retention_removes_only_preflights_past_their_cleanup_deadline() {
    let _guard = LOCK.lock().await;
    let (k, a, pool) = setup().await;
    let repository = uuid::Uuid::new_v4();
    let device = uuid::Uuid::new_v4();
    let binding = uuid::Uuid::new_v4();
    sqlx::query("DELETE FROM code_preflights WHERE org=$1")
        .bind(&a.org)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO code_repositories(org,product,id,services) VALUES($1,'demo',$2,ARRAY['backend'])").bind(&a.org).bind(repository).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO registered_devices(id,org,product,owner,public_key,approval_hash,services) VALUES($1,$2,'demo',$3,$4,$5,ARRAY['backend'])").bind(device).bind(&a.org).bind(&a.user).bind("11".repeat(32)).bind("22".repeat(32)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspace_bindings(id,org,product,user_id,device_id,approval_hash,services,policy_revision) VALUES($1,$2,'demo',$3,$4,$5,ARRAY['backend'],1)").bind(binding).bind(&a.org).bind(&a.user).bind(device.to_string()).bind("22".repeat(32)).execute(&pool).await.unwrap();
    let expired_id = uuid::Uuid::new_v4();
    let live_id = uuid::Uuid::new_v4();
    for (id, handle, cleanup_offset) in [
        (expired_id, uuid::Uuid::new_v4(), -1i64),
        (live_id, uuid::Uuid::new_v4(), 43_200i64),
    ] {
        sqlx::query("INSERT INTO code_preflights(id,org,product,owner,device_id,binding_id,security_epoch,services,policy_revision,repository_id,repository_revision,operation_handle,request,source_versions,brief,expires_at,cleanup_at) VALUES($1,$2,'demo',$3,$4,$5,1,ARRAY['backend'],1,$6,1,$7,'{}','{}','{}',now()+interval '5 minutes',now()+($8 * interval '1 second'))").bind(id).bind(&a.org).bind(&a.user).bind(device).bind(binding).bind(repository).bind(handle).bind(cleanup_offset as f64).execute(&pool).await.unwrap();
    }
    let result = k.retention_batch(1000).await.unwrap();
    assert!(result.preflights >= 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM code_preflights WHERE id=$1")
            .bind(expired_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM code_preflights WHERE id=$1")
            .bind(live_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn runtime_cannot_clean_or_replay_or_read_deletion_journal() {
    let _guard = LOCK.lock().await;
    let (k, a, pool) = setup().await;
    let note = k
        .personal_put(&a, "demo", input("runtime-delete"))
        .await
        .unwrap();
    let role = format!("wk_retention_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(
        &include_str!("../../../deploy/scripts/application-role.sql").replace("wk_app", &role),
    )
    .execute(&pool)
    .await
    .unwrap();
    let selected = role.clone();
    let scoped = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |c, _| {
            let role = selected.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}")).execute(c).await?;
                Ok(())
            })
        })
        .connect(&std::env::var("WK_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let runtime = Knowledge::from_pool(scoped.clone());
    runtime
        .personal_delete(&a, "demo", note.id, 1)
        .await
        .unwrap();
    assert_eq!(
        runtime.retention_batch(100).await.unwrap_err(),
        Error::Denied
    );
    assert_eq!(
        runtime.replay_personal_deletions().await.unwrap_err(),
        Error::Denied
    );
    assert!(sqlx::query("SELECT * FROM personal_deletions")
        .fetch_all(&scoped)
        .await
        .is_err());
    assert!(sqlx::query("DELETE FROM personal_deletions")
        .execute(&scoped)
        .await
        .is_err());
    assert_eq!(
        runtime
            .personal_put(&a, "demo", input("runtime-delete"))
            .await
            .unwrap_err(),
        Error::Conflict
    );
    scoped.close().await;
    sqlx::raw_sql(&format!("DROP OWNED BY {role}; DROP ROLE {role}"))
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn expired_operational_metadata_is_removed_without_retiring_live_credentials_or_key_tombstones(
) {
    let _guard = LOCK.lock().await;
    let (k, a, pool) = setup().await;
    let d = k
        .device_register(
            &a,
            wk_contracts::device::DeviceRegistration {
                id: uuid::Uuid::new_v4(),
                product: "demo".into(),
                services: vec!["backend".into()],
                public_key: "11".repeat(32),
                approval_hash: "22".repeat(32),
            },
        )
        .await
        .unwrap();
    k.device_revoke(&a, "demo", d.id, 1).await.unwrap();
    for (hash, expiry) in [("aa".repeat(32), 1i64), ("bb".repeat(32), 32503680000)] {
        sqlx::query("INSERT INTO collector_credentials(org,owner,token_hash,device_id,expires_at) VALUES($1,$2,$3,$4,to_timestamp($5))").bind(&a.org).bind(&a.user).bind(hash).bind(d.id).bind(expiry as f64).execute(&pool).await.unwrap();
    }
    let expired = uuid::Uuid::new_v4();
    let live = uuid::Uuid::new_v4();
    for (nonce, expiry) in [(expired, 1i64), (live, 32503680000)] {
        sqlx::query("INSERT INTO device_nonces(device_id,org,owner,nonce,expires_at) VALUES($1,$2,$3,$4,to_timestamp($5))").bind(d.id).bind(&a.org).bind(&a.user).bind(nonce).bind(expiry as f64).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO device_reports(device_id,org,owner,body,expires_at) VALUES($1,$2,$3,'{}',to_timestamp(1))").bind(d.id).bind(&a.org).bind(&a.user).execute(&pool).await.unwrap();
    for (session, expiry) in [("expired", 1i64), ("live", 32503680000)] {
        sqlx::query("INSERT INTO activity(org,product,user_id,session_id,services,files,intent,status,expires_at) VALUES($1,'demo',$2,$3,ARRAY['backend'],ARRAY[]::text[],'Bound work','active',to_timestamp($4))").bind(&a.org).bind(&a.user).bind(session).bind(expiry as f64).execute(&pool).await.unwrap();
    }
    let issuer = format!("https://{}.synthetic.example", a.org);
    k.provision_identity(&issuer, "alice", &a).await.unwrap();
    let idle = uuid::Uuid::new_v4().simple().to_string().repeat(2);
    let active = uuid::Uuid::new_v4().simple().to_string().repeat(2);
    for (hash, seen) in [(&idle, 1i64), (&active, 32503680000)] {
        sqlx::query("INSERT INTO browser_sessions(session_hash,issuer,subject,csrf_hash,last_seen) VALUES($1,$2,'alice',$3,to_timestamp($4))").bind(hash).bind(&issuer).bind("cc".repeat(32)).bind(seen as f64).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO audit_events(org,product,user_id,action,entity_id,created_at) VALUES($1,'demo',$2,'synthetic-expired',$3,to_timestamp(1))").bind(&a.org).bind(&a.user).bind(d.id).execute(&pool).await.unwrap();
    let counts = k.retention_batch(1000).await.unwrap();
    assert!(
        counts.reports >= 1
            && counts.nonces >= 1
            && counts.credentials >= 1
            && counts.activity >= 1
            && counts.sessions >= 1
            && counts.audit >= 1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM collector_credentials WHERE org=$1")
            .bind(&a.org)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM device_nonces WHERE device_id=$1")
            .bind(d.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM device_reports WHERE device_id=$1")
            .bind(d.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM activity WHERE org=$1")
            .bind(&a.org)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM browser_sessions WHERE issuer=$1")
            .bind(&issuer)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_events WHERE org=$1 AND action='synthetic-expired'"
        )
        .bind(&a.org)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    assert!(sqlx::query_scalar::<_, bool>(
        "SELECT revoked_at IS NOT NULL FROM registered_devices WHERE id=$1"
    )
    .bind(d.id)
    .fetch_one(&pool)
    .await
    .unwrap());
}
