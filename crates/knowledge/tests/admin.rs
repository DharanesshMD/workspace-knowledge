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
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(733093)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(
        &include_str!("../../../deploy/scripts/application-role.sql").replace("wk_app", &role),
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
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

// Barrier-controlled tests use real transactions and restricted application roles.
// A PostgreSQL lock wait proves each operation reached the intended barrier.
async fn named_pool(role: Option<String>, name: &str) -> sqlx::PgPool {
    let name = name.to_owned();
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |c, _| {
            let role = role.clone();
            let name = name.clone();
            Box::pin(async move {
                if let Some(role) = role {
                    sqlx::query(&format!("SET ROLE {role}"))
                        .execute(&mut *c)
                        .await?;
                }
                sqlx::query("SELECT set_config('application_name',$1,false)")
                    .bind(name)
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("WK_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap()
}
async fn wait_for_lock(pool: &sqlx::PgPool, name: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock')")
                .bind(name).fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("operation did not reach its database lock barrier");
}
fn registration(services: &[&str]) -> DeviceRegistration {
    DeviceRegistration {
        id: uuid::Uuid::new_v4(),
        product: "demo".into(),
        services: services.iter().map(|s| (*s).into()).collect(),
        public_key: "55".repeat(32),
        approval_hash: "66".repeat(32),
    }
}
async fn drop_role(pool: &sqlx::PgPool, role: &str) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(733093)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(&format!("DROP OWNED BY {role}; DROP ROLE {role}"))
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn bootstrap_changes_revoke_installations_registered_across_the_boundary() {
    let mut failures = Vec::new();
    for remove in [true, false] {
        let (k, u, pool) = setup().await;
        let (_, scoped, role) = restricted(&pool).await;
        scoped.close().await;
        let label = uuid::Uuid::new_v4().simple().to_string();
        let reg_name = format!("reg_{label}");
        let op_name = format!("op_{label}");
        let reg_pool = named_pool(Some(role.clone()), &reg_name).await;
        let op_pool = named_pool(None, &op_name).await;
        let function = format!("wk_barrier_{label}");
        let latch = (uuid::Uuid::new_v4().as_u128() >> 64) as i64;
        // Pause real registration after membership authorization but before insert.
        sqlx::raw_sql(&format!("CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.org='{}' THEN PERFORM pg_advisory_xact_lock({latch}); END IF; RETURN NEW; END $$; CREATE TRIGGER {function} BEFORE INSERT ON registered_devices FOR EACH ROW EXECUTE FUNCTION {function}()", u[2].org)).execute(&pool).await.unwrap();
        let mut barrier = pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(latch)
            .execute(&mut *barrier)
            .await
            .unwrap();
        let request = registration(&["backend", "frontend"]);
        let old_request = request.clone();
        let reg = Knowledge::from_pool(reg_pool.clone());
        let actor = u[2].clone();
        let registering = tokio::spawn(async move { reg.device_register(&actor, request).await });
        wait_for_lock(&pool, &reg_name).await;
        let op = Knowledge::from_pool(op_pool.clone());
        let actor = u[2].clone();
        let changing = tokio::spawn(async move {
            if remove {
                op.revoke(&actor, "demo").await
            } else {
                op.grant(&actor, "demo", &["backend".into()], false).await
            }
        });
        wait_for_lock(&pool, &op_name).await;
        barrier.commit().await.unwrap();
        let (_, changed) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            (registering.await.unwrap(), changing.await.unwrap())
        })
        .await
        .unwrap();
        k.grant(&u[2], "demo", &["backend".into(), "frontend".into()], false)
            .await
            .unwrap();
        let revoked: bool =
            sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM registered_devices WHERE id=$1")
                .bind(old_request.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let retry = Knowledge::from_pool(reg_pool.clone())
            .device_register(&u[2], old_request)
            .await;
        sqlx::raw_sql(&format!(
            "DROP TRIGGER {function} ON registered_devices; DROP FUNCTION {function}()"
        ))
        .execute(&pool)
        .await
        .unwrap();
        reg_pool.close().await;
        op_pool.close().await;
        drop_role(&pool, &role).await;
        assert!(changed.is_ok(), "bootstrap failed: {changed:?}");
        if !revoked || !matches!(retry, Err(Error::Denied)) {
            failures.push(format!("remove={remove}: installation survived removal/narrowing and regrant; retry={retry:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn admin_revocation_serializes_with_bootstrap_narrowing() {
    let (k, u, pool) = setup().await;
    let d = k
        .device_register(&u[0], registration(&["backend", "frontend"]))
        .await
        .unwrap();
    let (_, scoped, role) = restricted(&pool).await;
    scoped.close().await;
    let label = uuid::Uuid::new_v4().simple().to_string();
    let op_name = format!("op_{label}");
    let admin_name = format!("admin_{label}");
    let op_pool = named_pool(None, &op_name).await;
    let admin_pool = named_pool(Some(role.clone()), &admin_name).await;
    let mut barrier = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT 1 FROM memberships WHERE org=$1 AND product='demo' AND user_id='admin' FOR SHARE",
    )
    .bind(&u[0].org)
    .execute(&mut *barrier)
    .await
    .unwrap();
    let op = Knowledge::from_pool(op_pool.clone());
    let actor = u[0].clone();
    let narrowing =
        tokio::spawn(async move { op.grant(&actor, "demo", &["backend".into()], false).await });
    wait_for_lock(&pool, &op_name).await;
    let admin = Knowledge::from_pool(admin_pool.clone());
    let actor = u[0].clone();
    let revoking =
        tokio::spawn(async move { admin.admin_device_revoke(&actor, "demo", d.id).await });
    wait_for_lock(&pool, &admin_name).await;
    barrier.commit().await.unwrap();
    let (narrowed, revoked) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        (narrowing.await.unwrap(), revoking.await.unwrap())
    })
    .await
    .unwrap();
    op_pool.close().await;
    admin_pool.close().await;
    drop_role(&pool, &role).await;
    assert!(
        narrowed.is_ok(),
        "bootstrap narrowing aborted: {narrowed:?}"
    );
    assert_eq!(
        revoked.unwrap_err(),
        Error::Denied,
        "removed admin must be denied, not deadlocked"
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn admin_assignment_serializes_with_self_membership_change() {
    let (k, u, pool) = setup().await;
    let (_, scoped, role) = restricted(&pool).await;
    scoped.close().await;
    let label = uuid::Uuid::new_v4().simple().to_string();
    let op_name = format!("op_{label}");
    let admin_name = format!("admin_{label}");
    let op_pool = named_pool(None, &op_name).await;
    let admin_pool = named_pool(Some(role.clone()), &admin_name).await;
    let mut barrier = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM product_admins WHERE org=$1 AND product='demo' AND user_id='admin' FOR SHARE").bind(&u[0].org).execute(&mut *barrier).await.unwrap();
    let op = Knowledge::from_pool(op_pool.clone());
    let actor = u[0].clone();
    let assigning = tokio::spawn(async move {
        op.admin_assign(&actor, "demo", &["backend".into(), "frontend".into()])
            .await
    });
    wait_for_lock(&pool, &op_name).await;
    let admin = Knowledge::from_pool(admin_pool.clone());
    let actor = u[0].clone();
    let changing = tokio::spawn(async move {
        admin
            .admin_member_put(
                &actor,
                "demo",
                MemberChange {
                    user: actor.user.clone(),
                    services: vec!["backend".into(), "frontend".into()],
                    can_review: false,
                },
            )
            .await
    });
    wait_for_lock(&pool, &admin_name).await;
    barrier.commit().await.unwrap();
    let (assigned, changed) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        (assigning.await.unwrap(), changing.await.unwrap())
    })
    .await
    .unwrap();
    op_pool.close().await;
    admin_pool.close().await;
    drop_role(&pool, &role).await;
    assert!(assigned.is_ok(), "assignment deadlocked: {assigned:?}");
    assert!(
        changed.is_ok(),
        "self membership change deadlocked: {changed:?}"
    );
    // Keep setup's operator alive through the test, independent of runtime roles.
    assert!(!k.admin_members(&u[0], "demo").await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn owner_mutations_serialize_with_retention_policy_changes() {
    let mut failures = Vec::new();
    for action in ["edit", "delete", "submit"] {
        let (k, u, pool) = setup().await;
        let note = k
            .personal_put(
                &u[2],
                "demo",
                PersonalInput {
                    body: body("policy-race"),
                    provenance: PersonalProvenance::Captured,
                },
            )
            .await
            .unwrap();
        let (_, scoped, role) = restricted(&pool).await;
        scoped.close().await;
        let label = uuid::Uuid::new_v4().simple().to_string();
        let owner_name = format!("owner_{label}");
        let admin_name = format!("admin_{label}");
        let owner_pool = named_pool(Some(role.clone()), &owner_name).await;
        let admin_pool = named_pool(Some(role.clone()), &admin_name).await;
        let mut barrier = pool.begin().await.unwrap();
        sqlx::query("SELECT 1 FROM personal_summaries WHERE id=$1 FOR UPDATE")
            .bind(note.id)
            .execute(&mut *barrier)
            .await
            .unwrap();
        let owner = Knowledge::from_pool(owner_pool.clone());
        let actor = u[2].clone();
        let editing = tokio::spawn(async move {
            match action {
                "edit" => owner
                    .personal_edit(&actor, "demo", note.id, 1, body("policy-race"))
                    .await
                    .map(|_| ()),
                "delete" => owner.personal_delete(&actor, "demo", note.id, 1).await,
                _ => owner
                    .personal_submit(&actor, "demo", note.id, 1)
                    .await
                    .map(|_| ()),
            }
        });
        wait_for_lock(&pool, &owner_name).await;
        let admin = Knowledge::from_pool(admin_pool.clone());
        let actor = u[0].clone();
        let restricting = tokio::spawn(async move {
            admin
                .admin_policy_put(
                    &actor,
                    "demo",
                    ProductPolicy {
                        private_days: 2,
                        ..ProductPolicy::default()
                    },
                )
                .await
        });
        wait_for_lock(&pool, &admin_name).await;
        barrier.commit().await.unwrap();
        let (edited, restricted) =
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                (editing.await.unwrap(), restricting.await.unwrap())
            })
            .await
            .unwrap();
        owner_pool.close().await;
        admin_pool.close().await;
        drop_role(&pool, &role).await;
        if edited.is_err() || restricted.is_err() {
            failures.push(format!(
                "owner {action}={edited:?}, privacy policy={restricted:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
    drop_role(&pool, &role).await;
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
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn removing_a_member_revokes_their_personal_tokens_under_the_restricted_role() {
    let (k, u, pool) = setup().await;
    let (runtime, scoped, role) = restricted(&pool).await;
    let hash = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    runtime
        .create_personal_token(&u[2], "demo", "laptop", &hash, 30)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .personal_token_actor(&hash, "demo")
            .await
            .unwrap()
            .user,
        "alice"
    );
    runtime
        .admin_member_remove(&u[0], "demo", "alice")
        .await
        .unwrap();
    k.grant(&u[2], "demo", &["backend".into()], false)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .personal_token_actor(&hash, "demo")
            .await
            .err()
            .unwrap(),
        Error::Denied
    );
    scoped.close().await;
    drop_role(&pool, &role).await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn admin_created_accounts_work_under_the_restricted_application_role() {
    let (_k, u, pool) = setup().await;
    let (runtime, scoped, role) = restricted(&pool).await;
    let hash = || {
        format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        )
    };
    let (first, second) = (hash(), hash());
    let services = vec!["backend".to_string()];
    assert!(runtime.is_product_admin(&u[0], "demo").await.unwrap());
    assert!(!runtime.is_product_admin(&u[2], "demo").await.unwrap());
    runtime
        .admin_create_user(
            &u[0], "demo", "newdev", &services, false, "first", &first, 30,
        )
        .await
        .unwrap();
    assert_eq!(
        runtime
            .personal_token_actor(&first, "demo")
            .await
            .unwrap()
            .user,
        "newdev"
    );
    let listing = runtime.admin_users(&u[0], "demo").await.unwrap();
    assert!(listing
        .users
        .iter()
        .any(|m| m.user == "newdev" && m.token_only && m.tokens.len() == 1));
    runtime
        .admin_reissue_token(&u[0], "demo", "newdev", "second", &second, 7)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .personal_token_actor(&first, "demo")
            .await
            .err()
            .unwrap(),
        Error::Denied
    );
    assert_eq!(
        runtime
            .admin_reissue_token(&u[0], "demo", "alice", "x", &hash(), 7)
            .await
            .unwrap_err(),
        Error::Denied
    );
    assert_eq!(
        runtime
            .admin_create_user(
                &u[2],
                "demo",
                "bysomeone",
                &services,
                false,
                "x",
                &hash(),
                7
            )
            .await
            .unwrap_err(),
        Error::Denied
    );
    runtime
        .admin_remove_user(&u[0], "demo", "newdev")
        .await
        .unwrap();
    assert_eq!(
        runtime
            .personal_token_actor(&second, "demo")
            .await
            .err()
            .unwrap(),
        Error::Denied
    );
    scoped.close().await;
    drop_role(&pool, &role).await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn shared_files_and_team_work_run_under_the_restricted_application_role() {
    let (_k, u, pool) = setup().await;
    let (runtime, scoped, role) = restricted(&pool).await;
    let share = |title: &str, files: &[&str]| wk_knowledge::share::ShareRequest {
        service: Some("backend".into()),
        title: title.into(),
        intent: format!("Goal of {title}"),
        rationale: "Because".into(),
        files: files.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    let shared = runtime
        .share_session(
            &u[2],
            "demo",
            share("Retry with backoff", &["backend/payments/retry.rs"]),
        )
        .await
        .unwrap();
    assert!(shared.published);
    let found = runtime
        .team_work(
            &u[3],
            "demo",
            wk_knowledge::team_work::TeamWorkQuery {
                developer: Some("ali".into()),
                paths: vec!["backend/payments".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(found["resolved"]["developer"], "alice");
    assert_eq!(
        found["entries"][0]["files_reported"],
        serde_json::json!(["backend/payments/retry.rs"])
    );
    // Row-level security: another organization's member sees nothing of it.
    let stranger = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "alice".into(),
    };
    assert_eq!(
        runtime
            .team_work(&stranger, "demo", Default::default())
            .await
            .unwrap_err(),
        Error::Denied
    );
    let leaked: i64 = {
        let mut tx = scoped.begin().await.unwrap();
        sqlx::query("SELECT set_config('wk.org',$1,true),set_config('wk.user',$2,true)")
            .bind(&stranger.org)
            .bind(&stranger.user)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query_scalar("SELECT count(*) FROM card_files")
            .fetch_one(&mut *tx)
            .await
            .unwrap()
    };
    assert_eq!(
        leaked, 0,
        "card_files rows are invisible outside the organization"
    );
    scoped.close().await;
    drop_role(&pool, &role).await;
}
