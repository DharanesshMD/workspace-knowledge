use sqlx::Row;
use wk_knowledge::{personal_tokens::MAX_LIVE_PER_USER, Actor, Error, Knowledge};

fn hash() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
async fn setup() -> (Knowledge, sqlx::PgPool, Actor, Actor) {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let mut people = vec![];
    for (name, product) in [("anika", "demo"), ("ravi", "demo")] {
        let a = Actor {
            org: org.clone(),
            user: name.into(),
        };
        k.provision_identity(&format!("https://{org}.synthetic.example"), name, &a)
            .await
            .unwrap();
        k.grant(&a, product, &["backend".into()], false)
            .await
            .unwrap();
        people.push(a);
    }
    let ravi = people.pop().unwrap();
    let anika = people.pop().unwrap();
    k.grant(&anika, "other", &["frontend".into()], false)
        .await
        .unwrap();
    (k, pool, anika, ravi)
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn a_token_belongs_to_one_user_and_one_product() {
    let (k, _pool, anika, ravi) = setup().await;
    // A new migration must also be accepted by the readiness check.
    k.ready().await.unwrap();
    let h = hash();
    let made = k
        .create_personal_token(&anika, "demo", "  laptop  ", &h, 30)
        .await
        .unwrap();
    assert_eq!(made.label, "laptop");
    assert_eq!(made.product, "demo");
    assert!(made.expires_ms - made.created_ms >= 29 * 86_400_000);

    let owner = k.personal_token_actor(&h, "demo").await.unwrap();
    assert_eq!(
        (owner.org.as_str(), owner.user.as_str()),
        (anika.org.as_str(), "anika")
    );
    // Same token, another product, an unknown token and a malformed hash are all just denied.
    for (candidate, product) in [
        (h.clone(), "other"),
        (hash(), "demo"),
        ("xyz".into(), "demo"),
    ] {
        assert_eq!(
            k.personal_token_actor(&candidate, product)
                .await
                .err()
                .unwrap(),
            Error::Denied
        );
    }
    // Listing is per user, and carries no secret or hash.
    assert!(k.list_personal_tokens(&ravi).await.unwrap().is_empty());
    let listed = serde_json::to_value(k.list_personal_tokens(&anika).await.unwrap()).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(
        listed[0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "created_ms",
            "expires_ms",
            "id",
            "label",
            "last_used_ms",
            "product"
        ]
        .into_iter()
        .map(String::from)
        .collect()
    );
    // Only members can create one for a product.
    assert_eq!(
        k.create_personal_token(&ravi, "other", "x", &hash(), 7)
            .await
            .unwrap_err(),
        Error::Denied
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn creation_is_validated_capped_and_audited() {
    let (k, pool, anika, _ravi) = setup().await;
    let long_label = "x".repeat(65);
    for (label, hash, days) in [
        ("", hash(), 30),
        ("   ", hash(), 30),
        (long_label.as_str(), hash(), 30),
        ("tab\there", hash(), 30),
        ("ok", "not-a-hash".to_string(), 30),
        ("ok", hash(), 0),
        ("ok", hash(), 366),
    ] {
        assert_eq!(
            k.create_personal_token(&anika, "demo", label, &hash, days)
                .await
                .unwrap_err(),
            Error::Invalid,
            "label={label:?} days={days}"
        );
    }
    let mut ids = vec![];
    for n in 0..MAX_LIVE_PER_USER {
        ids.push(
            k.create_personal_token(&anika, "demo", &format!("t{n}"), &hash(), 30)
                .await
                .unwrap()
                .id,
        );
    }
    assert_eq!(
        k.create_personal_token(&anika, "demo", "one too many", &hash(), 30)
            .await
            .unwrap_err(),
        Error::Conflict
    );
    // Revoking frees a slot.
    k.revoke_personal_token(&anika, ids[0]).await.unwrap();
    k.create_personal_token(&anika, "demo", "replacement", &hash(), 30)
        .await
        .unwrap();
    let actions: Vec<String> = sqlx::query("SELECT action FROM audit_events WHERE org=$1 AND action LIKE 'personal_token_%' ORDER BY id")
        .bind(&anika.org)
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|r| r.get("action"))
        .collect();
    assert_eq!(
        actions
            .iter()
            .filter(|a| *a == "personal_token_create")
            .count(),
        6
    );
    assert_eq!(
        actions
            .iter()
            .filter(|a| *a == "personal_token_revoke")
            .count(),
        1
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn revoked_and_expired_tokens_stop_working_and_only_the_owner_can_revoke() {
    let (k, pool, anika, ravi) = setup().await;
    let (revoked, expired) = (hash(), hash());
    let a = k
        .create_personal_token(&anika, "demo", "a", &revoked, 30)
        .await
        .unwrap();
    k.create_personal_token(&anika, "demo", "b", &expired, 30)
        .await
        .unwrap();

    assert_eq!(
        k.revoke_personal_token(&ravi, a.id).await.unwrap_err(),
        Error::NotFound
    );
    assert!(k.personal_token_actor(&revoked, "demo").await.is_ok());
    k.revoke_personal_token(&anika, a.id).await.unwrap();
    assert_eq!(
        k.personal_token_actor(&revoked, "demo")
            .await
            .err()
            .unwrap(),
        Error::Denied
    );
    assert_eq!(
        k.revoke_personal_token(&anika, a.id).await.unwrap_err(),
        Error::NotFound
    );
    assert_eq!(k.list_personal_tokens(&anika).await.unwrap().len(), 1);

    sqlx::query(
        "UPDATE personal_tokens SET expires_at=now()-interval '1 second' WHERE token_hash=$1",
    )
    .bind(&expired)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        k.personal_token_actor(&expired, "demo")
            .await
            .err()
            .unwrap(),
        Error::Denied
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn last_use_is_recorded_but_not_on_every_request() {
    let (k, pool, anika, _ravi) = setup().await;
    let h = hash();
    k.create_personal_token(&anika, "demo", "a", &h, 30)
        .await
        .unwrap();
    let stamp = || async {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_used_at::text FROM personal_tokens WHERE token_hash=$1",
        )
        .bind(&h)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(stamp().await, None);
    k.personal_token_actor(&h, "demo").await.unwrap();
    let first = stamp().await.expect("first use is recorded");
    k.personal_token_actor(&h, "demo").await.unwrap();
    assert_eq!(stamp().await.as_deref(), Some(first.as_str()));
    sqlx::query(
        "UPDATE personal_tokens SET last_used_at=now()-interval '2 minutes' WHERE token_hash=$1",
    )
    .bind(&h)
    .execute(&pool)
    .await
    .unwrap();
    k.personal_token_actor(&h, "demo").await.unwrap();
    assert_ne!(stamp().await.as_deref(), Some(first.as_str()));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn ending_a_membership_revokes_the_token_for_good() {
    let (k, _pool, anika, ravi) = setup().await;
    let (kept, gone) = (hash(), hash());
    k.create_personal_token(&ravi, "demo", "kept", &kept, 30)
        .await
        .unwrap();
    k.create_personal_token(&anika, "demo", "gone", &gone, 30)
        .await
        .unwrap();
    k.create_personal_token(&anika, "other", "elsewhere", &hash(), 30)
        .await
        .unwrap();

    k.revoke(&anika, "demo").await.unwrap();
    // Adding the person back later must not bring the old token back.
    k.grant(&anika, "demo", &["backend".into()], false)
        .await
        .unwrap();
    assert_eq!(
        k.personal_token_actor(&gone, "demo").await.err().unwrap(),
        Error::Denied
    );
    assert!(k.personal_token_actor(&kept, "demo").await.is_ok());
    // Only the demo membership ended: the other product's token and the list still show it.
    let remaining: Vec<_> = k
        .list_personal_tokens(&anika)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.product)
        .collect();
    assert_eq!(remaining, vec!["other".to_string()]);
}

// Observe PostgreSQL's actual lock wait instead of guessing when the request has reached a barrier.
async fn named_knowledge(name: &str) -> Knowledge {
    let name = name.to_owned();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |c, _| {
            let name = name.clone();
            Box::pin(async move {
                sqlx::query("SELECT set_config('application_name',$1,false)")
                    .bind(name)
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("WK_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    Knowledge::from_pool(pool)
}
async fn waiting_on_lock(pool: &sqlx::PgPool, name: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock')")
        .bind(name).fetch_one(pool).await.unwrap()
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn membership_removal_cannot_leave_a_token_issued_after_revocation() {
    for admin_removal in [false, true] {
        let (k, pool, anika, ravi) = setup().await;
        k.admin_assign(&ravi, "demo", &["backend".into()])
            .await
            .unwrap();
        let h = hash();
        let issuance_name = format!("issuance-{}", uuid::Uuid::new_v4());
        let removal_name = format!("removal-{}", uuid::Uuid::new_v4());
        let issuing = named_knowledge(&issuance_name).await;
        let removing = named_knowledge(&removal_name).await;
        // Pause issuance immediately after its membership authorization, at the per-owner cap lock.
        let mut barrier = pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "wk-token:{}:{}:{}",
                anika.org.len(),
                anika.org,
                anika.user
            ))
            .execute(&mut *barrier)
            .await
            .unwrap();
        let (issuer, token_hash) = (anika.clone(), h.clone());
        let creation = tokio::spawn(async move {
            issuing
                .create_personal_token(&issuer, "demo", "racing", &token_hash, 30)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !waiting_on_lock(&pool, &issuance_name).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("issuance must reach the owner lock barrier");
        let (target, admin) = (anika.clone(), ravi.clone());
        let removal = tokio::spawn(async move {
            if admin_removal {
                removing
                    .admin_remove_user(&admin, "demo", &target.user)
                    .await
            } else {
                removing.revoke(&target, "demo").await
            }
        });
        // A correct implementation makes removal wait for issuance's administration gate.
        // The vulnerable implementation commits removal now, then inserts a live token later.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !removal.is_finished() && !waiting_on_lock(&pool, &removal_name).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("removal must finish or reach its lock barrier");
        barrier.commit().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            creation.await.unwrap().unwrap();
            removal.await.unwrap().unwrap();
        })
        .await
        .expect("issuance and removal must finish without a lock-order deadlock");
        k.grant(&anika, "demo", &["backend".into()], false)
            .await
            .unwrap();
        assert_eq!(k.personal_token_actor(&h, "demo").await.err(), Some(Error::Denied), "membership re-add must not revive a token that raced with removal (admin={admin_removal})");
        assert!(k.list_personal_tokens(&anika).await.unwrap().is_empty());
    }
}
