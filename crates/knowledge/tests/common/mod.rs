#![allow(dead_code)]
use wk_knowledge::{Actor, Knowledge};
pub mod code;

pub struct Fixture {
    pub k: Knowledge,
    pub pool: sqlx::PgPool,
    pub admin: Actor,
    pub alice: Actor,
    pub bob: Actor,
    pub reader: Actor,
}

pub async fn setup() -> Fixture {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let actor = |user: &str| Actor {
        org: org.clone(),
        user: user.into(),
    };
    let (admin, alice, bob, reader) = (
        actor("admin"),
        actor("alice"),
        actor("bob"),
        actor("reader"),
    );
    for a in [&admin, &alice, &bob, &reader] {
        k.provision_identity(&format!("https://{org}.synthetic.example"), &a.user, a)
            .await
            .unwrap();
        let scope = if a.user == "reader" {
            vec!["backend".into()]
        } else {
            vec!["backend".into(), "frontend".into()]
        };
        k.grant(a, "demo", &scope, a.user == "bob").await.unwrap();
    }
    k.admin_assign(&admin, "demo", &["backend".into(), "frontend".into()])
        .await
        .unwrap();
    Fixture {
        k,
        pool,
        admin,
        alice,
        bob,
        reader,
    }
}

pub async fn runtime(f: &Fixture) -> (Knowledge, sqlx::PgPool, String) {
    let role = format!("wk_code_{}", uuid::Uuid::new_v4().simple());
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(733093)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(
        &include_str!("../../../../deploy/scripts/application-role.sql").replace("wk_app", &role),
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let selected = role.clone();
    let scoped = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |c, _| {
            let r = selected.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {r}")).execute(c).await?;
                Ok(())
            })
        })
        .connect(&std::env::var("WK_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    (Knowledge::from_pool(scoped.clone()), scoped, role)
}

pub async fn cleanup_runtime(f: &Fixture, pool: sqlx::PgPool, role: String) {
    pool.close().await;
    let mut tx = f.pool.begin().await.unwrap();
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
