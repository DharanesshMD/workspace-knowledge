use sqlx::Row;
use wk_contracts::{device::*, CardSubmission};
use wk_knowledge::{
    admin::{MemberChange, ProductPolicy},
    devices::DevicePermission,
    personal::{PersonalInput, PersonalProvenance},
    Actor, Error, Knowledge,
};
async fn setup() -> (Knowledge, Vec<Actor>, sqlx::PgPool) {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let mut users = vec![];
    for (name, services, review) in [
        ("admin", vec!["backend", "frontend"], false),
        ("reviewer", vec!["backend", "frontend"], true),
        ("alice", vec!["backend", "frontend"], false),
        ("partial", vec!["backend"], false),
    ] {
        let a = Actor {
            org: org.clone(),
            user: name.into(),
        };
        k.provision_identity(&format!("https://{org}.synthetic.example"), name, &a)
            .await
            .unwrap();
        k.grant(
            &a,
            "demo",
            &services.into_iter().map(String::from).collect::<Vec<_>>(),
            review,
        )
        .await
        .unwrap();
        users.push(a);
    }
    k.admin_assign(&users[0], "demo", &["backend".into(), "frontend".into()])
        .await
        .unwrap();
    k.admin_assign(&users[3], "demo", &["backend".into()])
        .await
        .unwrap();
    (k, users, pool)
}
async fn restricted(pool: &sqlx::PgPool) -> (Knowledge, sqlx::PgPool, String) {
    let role = format!("wk_admin_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql(
        &include_str!("../../../deploy/scripts/application-role.sql").replace("wk_app", &role),
    )
    .execute(pool)
    .await
    .unwrap();
    let selected = role.clone();
    let scoped = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
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
    (Knowledge::from_pool(scoped.clone()), scoped, role)
}
fn body(key: &str) -> CardSubmission {
    serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Bound work","intent":"Reduce overload","rationale":"Scoped middleware","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn scoped_admin_is_distinct_from_review_and_cannot_read_private_context() {
    let (k, u, pool) = setup().await;
    let (runtime, scoped, role) = restricted(&pool).await;
    assert_eq!(
        runtime.admin_members(&u[1], "demo").await.unwrap_err(),
        Error::Denied
    );
    assert_eq!(
        runtime.admin_members(&u[0], "other").await.unwrap_err(),
        Error::Denied
    );
    let outsider = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "admin".into(),
    };
    assert_eq!(
        runtime.admin_members(&outsider, "demo").await.unwrap_err(),
        Error::Denied
    );
    assert_eq!(
        runtime
            .admin_assign(&u[1], "demo", &["backend".into()])
            .await
            .unwrap_err(),
        Error::Denied
    );
    let listed = runtime.admin_members(&u[0], "demo").await.unwrap();
    assert_eq!(listed.len(), 4);
    assert_eq!(runtime.admin_members(&u[3], "demo").await.unwrap().len(), 1);
    assert_eq!(
        runtime
            .admin_member_put(
                &u[3],
                "demo",
                MemberChange {
                    user: "alice".into(),
                    services: vec!["backend".into()],
                    can_review: false
                }
            )
            .await
            .unwrap_err(),
        Error::Denied
    );
    assert_eq!(
        runtime
            .admin_member_put(
                &u[0],
                "demo",
                MemberChange {
                    user: "unmapped".into(),
                    services: vec!["backend".into()],
                    can_review: false
                }
            )
            .await
            .unwrap_err(),
        Error::Denied
    );
    assert_eq!(
        runtime
            .admin_member_put(
                &u[0],
                "demo",
                MemberChange {
                    user: "alice".into(),
                    services: vec!["secret".into()],
                    can_review: false
                }
            )
            .await
            .unwrap_err(),
        Error::Denied
    );
    let note = k
        .personal_put(
            &u[2],
            "demo",
            PersonalInput {
                body: body("private"),
                provenance: PersonalProvenance::Captured,
            },
        )
        .await
        .unwrap();
    let card = k
        .submit(&u[2], "demo", body("shared-candidate"))
        .await
        .unwrap();
    assert!(runtime
        .personal_list(&u[0], "demo", None)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        runtime
            .personal_get(&u[0], "demo", note.id)
            .await
            .unwrap_err(),
        Error::NotFound
    );
    assert_eq!(
        runtime.get(&u[0], "demo", card.id).await.unwrap_err(),
        Error::NotFound
    );
    let mut tx = scoped.begin().await.unwrap();
    sqlx::query("SELECT set_config('wk.org',$1,true),set_config('wk.user',$2,true)")
        .bind(&u[0].org)
        .bind(&u[0].user)
        .execute(&mut *tx)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM memberships")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 1);
    tx.rollback().await.unwrap();
    k.revoke(&u[0], "demo").await.unwrap();
    assert_eq!(
        runtime.admin_members(&u[0], "demo").await.unwrap_err(),
        Error::Denied
    );
    scoped.close().await;
    sqlx::raw_sql(&format!("DROP OWNED BY {role}; DROP ROLE {role}"))
        .execute(&pool)
        .await
        .unwrap();
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn admin_scope_change_and_membership_regrant_do_not_resurrect_devices() {
    let (k, u, pool) = setup().await;
    let d = k
        .device_register(
            &u[2],
            DeviceRegistration {
                id: uuid::Uuid::new_v4(),
                product: "demo".into(),
                services: vec!["backend".into(), "frontend".into()],
                public_key: "11".repeat(32),
                approval_hash: "22".repeat(32),
            },
        )
        .await
        .unwrap();
    k.admin_member_put(
        &u[0],
        "demo",
        MemberChange {
            user: "alice".into(),
            services: vec!["backend".into()],
            can_review: false,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        k.device_bound(&d, DevicePermission::Read)
            .policy(&u[2], "demo")
            .await,
        Err(Error::Denied)
    ));
    k.admin_member_put(
        &u[0],
        "demo",
        MemberChange {
            user: "alice".into(),
            services: vec!["backend".into(), "frontend".into()],
            can_review: false,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        k.device_bound(&d, DevicePermission::Read)
            .policy(&u[2], "demo")
            .await,
        Err(Error::Denied)
    ));
    let replacement = k
        .device_register(
            &u[2],
            DeviceRegistration {
                id: uuid::Uuid::new_v4(),
                product: "demo".into(),
                services: vec!["backend".into()],
                public_key: "33".repeat(32),
                approval_hash: "22".repeat(32),
            },
        )
        .await
        .unwrap();
    k.admin_member_remove(&u[0], "demo", "alice").await.unwrap();
    k.grant(&u[2], "demo", &["backend".into()], false)
        .await
        .unwrap();
    assert!(matches!(
        k.device_bound(&replacement, DevicePermission::Read)
            .policy(&u[2], "demo")
            .await,
        Err(Error::Denied)
    ));
    let rows = sqlx::query(
        "SELECT action,target_user FROM audit_events WHERE org=$1 AND action LIKE 'admin_%'",
    )
    .bind(&u[0].org)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(rows
        .iter()
        .any(|r| r.get::<String, _>("action") == "admin_member_remove"
            && r.get::<Option<String>, _>("target_user").as_deref() == Some("alice")));
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn admin_policy_can_restrict_but_not_widen_privacy_or_destinations() {
    let (k, u, pool) = setup().await;
    let note = k
        .personal_put(
            &u[2],
            "demo",
            PersonalInput {
                body: body("old"),
                provenance: PersonalProvenance::Captured,
            },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE personal_summaries SET created_at=now()-interval '3 days' WHERE id=$1")
        .bind(note.id)
        .execute(&pool)
        .await
        .unwrap();
    let policy = ProductPolicy {
        private_days: 2,
        report_days: 2,
        allow_loopback: false,
        https_origins: vec![],
    };
    assert_eq!(
        k.admin_policy_put(&u[3], "demo", policy.clone())
            .await
            .unwrap_err(),
        Error::Denied
    );
    k.admin_policy_put(&u[0], "demo", policy.clone())
        .await
        .unwrap();
    assert_eq!(k.policy(&u[2], "demo").await.unwrap().restrictions, policy);
    assert!(k.policy(&u[2], "demo").await.unwrap().policy_revision > 1);
    assert_eq!(
        k.personal_get(&u[2], "demo", note.id).await.unwrap_err(),
        Error::NotFound
    );
    let mut fresh = body("new-after-restriction");
    fresh.policy_revision = k.policy(&u[2], "demo").await.unwrap().policy_revision as u64;
    let new_note = k
        .personal_put(
            &u[2],
            "demo",
            PersonalInput {
                body: fresh,
                provenance: PersonalProvenance::Captured,
            },
        )
        .await
        .unwrap();
    assert!((new_note.expires_at_ms - new_note.created_at_ms - 2 * 86_400_000).abs() < 1000);
    assert_eq!(
        k.admin_policy_put(&u[0], "demo", ProductPolicy::default())
            .await
            .unwrap_err(),
        Error::Denied
    );
    let bad = ProductPolicy {
        private_days: 31,
        ..ProductPolicy::default()
    };
    assert_eq!(
        k.admin_policy_put(&u[0], "demo", bad).await.unwrap_err(),
        Error::Invalid
    );
}
