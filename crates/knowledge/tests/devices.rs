use wk_contracts::{device::*, CardSubmission};
use wk_knowledge::{
    devices::*,
    personal::{PersonalInput, PersonalProvenance},
    Actor, Error, Knowledge,
};
fn registration() -> DeviceRegistration {
    DeviceRegistration {
        id: uuid::Uuid::new_v4(),
        product: "demo".into(),
        services: vec!["backend".into()],
        public_key: "11".repeat(32),
        approval_hash: "22".repeat(32),
    }
}
async fn setup() -> (Knowledge, Actor, Actor, sqlx::PgPool) {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let a = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "alice".into(),
    };
    let b = Actor {
        org: a.org.clone(),
        user: "bob".into(),
    };
    for x in [&a, &b] {
        k.grant(x, "demo", &["backend".into(), "frontend".into()], true)
            .await
            .unwrap();
    }
    (k, a, b, pool)
}
fn input(key: &str, services: &[&str]) -> PersonalInput {
    let body:CardSubmission=serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Bound work","intent":"Limit work","rationale":"","decisions":[],"constraints":[],"open_questions":[],"service_ids":services,"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap();
    PersonalInput {
        body,
        provenance: PersonalProvenance::Captured,
    }
}
fn report(rev: i64) -> DeviceReport {
    serde_json::from_value(serde_json::json!({"collector_version":"0.1.0","config_revision":rev,"config_status":"applied","local_pause":false,"local_owner_sync":false,"local_activity":false,"queue":"empty","reason":"none","capabilities":[{"client":"codex","surface":"cli","version":"0.162.0-alpha.17.2","status":"blocked"}]})).unwrap()
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn devices_are_owner_scoped_revocable_and_guard_real_uploads() {
    let (k, a, b, _) = setup().await;
    let reg = registration();
    let d = k.device_register(&a, reg.clone()).await.unwrap();
    assert_eq!(k.device_register(&a, reg.clone()).await.unwrap().id, d.id);
    assert!(k.device_list(&b, "demo").await.unwrap().is_empty());
    assert!(matches!(
        k.device_get(&b, "demo", d.id).await,
        Err(Error::NotFound)
    ));
    let narrow = k.device_bound(&d, DevicePermission::OwnerUpload);
    assert!(matches!(
        narrow
            .personal_put(&a, "demo", input("off", &["backend"]))
            .await,
        Err(Error::Denied)
    ));
    let settings = DeviceSettings {
        owner_sync: true,
        activity: false,
        paused: false,
    };
    let d = k
        .device_change(&a, "demo", d.id, 1, settings)
        .await
        .unwrap();
    assert!(matches!(
        k.device_change(&a, "demo", d.id, 1, DeviceSettings::default())
            .await,
        Err(Error::Conflict)
    ));
    let narrow = k.device_bound(&d, DevicePermission::OwnerUpload);
    assert!(narrow
        .personal_put(&a, "demo", input("yes", &["backend"]))
        .await
        .is_ok());
    assert!(matches!(
        narrow
            .personal_put(&a, "demo", input("wide", &["frontend"]))
            .await,
        Err(Error::Denied)
    ));
    assert!(matches!(
        narrow
            .personal_put(&a, "other", input("other", &["backend"]))
            .await,
        Err(Error::Denied)
    ));
    k.device_revoke(&a, "demo", d.id, d.revision).await.unwrap();
    assert!(matches!(
        narrow
            .personal_put(&a, "demo", input("revoked", &["backend"]))
            .await,
        Err(Error::Denied)
    ));
    assert!(matches!(
        k.device_register(&a, reg).await,
        Err(Error::Denied)
    ));
    let mut new = registration();
    assert!(matches!(
        k.device_register(&a, new.clone()).await,
        Err(Error::Denied)
    ));
    new.public_key = "33".repeat(32);
    assert!(k.device_register(&a, new).await.is_ok());
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn reports_acknowledge_current_settings_and_expire_without_private_fields() {
    let (k, a, b, pool) = setup().await;
    let d = k.device_register(&a, registration()).await.unwrap();
    k.device_report(&a, &d, report(1)).await.unwrap();
    assert!(k
        .device_get(&a, "demo", d.id)
        .await
        .unwrap()
        .report
        .is_some());
    let d = k
        .device_change(
            &a,
            "demo",
            d.id,
            1,
            DeviceSettings {
                paused: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        k.device_report(&a, &d, report(1)).await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        k.device_bound(&d, DevicePermission::SharedUpload)
            .submit(&a, "demo", input("paused", &["backend"]).body)
            .await,
        Err(Error::Denied)
    ));
    k.device_report(&a, &d, report(2)).await.unwrap();
    assert!(k.device_list(&b, "demo").await.unwrap().is_empty());
    let safe = serde_json::to_string(&k.device_get(&a, "demo", d.id).await.unwrap()).unwrap();
    assert!(!safe.contains("approval_hash"));
    assert!(!safe.contains("public_key"));
    sqlx::query(
        "UPDATE device_reports SET expires_at=now()-interval '1 second' WHERE device_id=$1",
    )
    .bind(d.id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(k
        .device_get(&a, "demo", d.id)
        .await
        .unwrap()
        .report
        .is_none());
    k.revoke(&a, "demo").await.unwrap();
    assert!(k.device_list(&a, "demo").await.unwrap().is_empty());
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn revocation_committing_before_a_waiting_guarded_upload_denies_it() {
    let (k, a, _, pool) = setup().await;
    let d = k.device_register(&a, registration()).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM registered_devices WHERE id=$1 FOR UPDATE")
        .bind(d.id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let app_name = format!("device_race_{}", d.id);
    let selected = app_name.clone();
    let guarded_pool = sqlx::postgres::PgPoolOptions::new()
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
    let guarded =
        Knowledge::from_pool(guarded_pool.clone()).device_bound(&d, DevicePermission::SharedUpload);
    let owner = a.clone();
    let upload = tokio::spawn(async move {
        guarded
            .submit(&owner, "demo", input("race", &["backend"]).body)
            .await
    });
    let mut blocked = false;
    for _ in 0..100 {
        blocked=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock')").bind(&app_name).fetch_one(&pool).await.unwrap();
        if blocked {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(blocked, "upload must be waiting on revocation transaction");
    sqlx::query("UPDATE registered_devices SET revoked_at=now() WHERE id=$1")
        .bind(d.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(matches!(upload.await.unwrap(), Err(Error::Denied)));
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn device_nonces_are_single_use_and_registration_scope_cannot_expand() {
    let (k, a, _, _) = setup().await;
    let reg = registration();
    let d = k.device_register(&a, reg.clone()).await.unwrap();
    let nonce = uuid::Uuid::new_v4();
    k.device_nonce(&a, &d, nonce).await.unwrap();
    assert!(matches!(
        k.device_nonce(&a, &d, nonce).await,
        Err(Error::Denied)
    ));
    let mut wide = reg;
    wide.services.push("frontend".into());
    assert!(matches!(
        k.device_register(&a, wide).await,
        Err(Error::Denied)
    ));
}
#[test]
fn health_contract_rejects_arbitrary_paths_secrets_and_versions() {
    for value in [
        serde_json::json!({"raw_messages":"private-canary"}),
        serde_json::json!({"collector_version":"/Users/private"}),
        serde_json::json!({"capabilities":[{"client":"codex","surface":"cli","version":"sk-proj-SYNTHETIC_SECRET_CANARY","status":"supported"}]}),
    ] {
        let mut base = serde_json::to_value(report(1)).unwrap();
        for (k, v) in value.as_object().unwrap() {
            base[k] = v.clone();
        }
        match serde_json::from_value::<DeviceReport>(base) {
            Err(_) => (),
            Ok(r) => assert!(r.validate().is_err()),
        }
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn restricted_runtime_role_cannot_read_peer_devices_or_hidden_reports() {
    let (k, a, b, owner) = setup().await;
    let role = format!("wk_device_{}", uuid::Uuid::new_v4().simple());
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
        .after_connect(move |c, _| {
            let role = selected.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}")).execute(c).await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let runtime = Knowledge::from_pool(pool.clone());
    let d = runtime.device_register(&a, registration()).await.unwrap();
    runtime.device_report(&a, &d, report(1)).await.unwrap();
    assert!(runtime.device_list(&b, "demo").await.unwrap().is_empty());
    assert!(matches!(
        runtime
            .device_change(&b, "demo", d.id, 1, DeviceSettings::default())
            .await,
        Err(Error::NotFound)
    ));
    for table in ["registered_devices", "device_reports", "device_nonces"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    k.grant(&a, "demo", &["frontend".into()], false)
        .await
        .unwrap();
    assert!(runtime.device_list(&a, "demo").await.unwrap().is_empty());
    pool.close().await;
    sqlx::raw_sql(&format!("DROP OWNED BY {role}; DROP ROLE {role};"))
        .execute(&owner)
        .await
        .unwrap();
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn revoking_one_of_ten_installations_allows_replacement_and_audits_metadata() {
    let (k, a, _, pool) = setup().await;
    let mut first = None;
    for n in 1..=10 {
        let mut r = registration();
        r.public_key = format!("{n:064x}");
        let d = k.device_register(&a, r).await.unwrap();
        if first.is_none() {
            first = Some(d)
        }
    }
    let mut next = registration();
    next.public_key = "ff".repeat(32);
    assert!(matches!(
        k.device_register(&a, next.clone()).await,
        Err(Error::Conflict)
    ));
    let d = first.unwrap();
    k.device_revoke(&a, "demo", d.id, 1).await.unwrap();
    assert!(k.device_register(&a, next).await.is_ok());
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_events WHERE org=$1 AND entity_id=$2 ORDER BY id",
    )
    .bind(&a.org)
    .bind(d.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(events, vec!["device_register", "device_revoke"]);
}
