use std::time::{Duration, Instant};
#[tokio::test]
async fn cleanup_worker_survives_storage_failure_and_stops_on_shutdown() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://127.0.0.1:1/wk_test")
        .unwrap();
    pool.close().await;
    let k = wk_knowledge::Knowledge::from_pool(pool);
    let began = Instant::now();
    tokio::time::timeout(
        Duration::from_millis(500),
        wk_server::retention_worker::run(
            &k,
            async { tokio::time::sleep(Duration::from_millis(60)).await },
            Duration::from_millis(10),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(began.elapsed() >= Duration::from_millis(55));
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn operator_cleanup_cli_returns_only_bounded_counts() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    wk_knowledge::Knowledge::connect(&url).await.unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_wk-server"))
        .arg("retention-once")
        .env("WK_DATABASE_URL", &url)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert!(value["personal"].as_u64().unwrap() <= 1000);
    for field in ["org", "owner", "body", "token", "key_hash"] {
        assert!(value.get(field).is_none());
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn worker_refuses_permanently_unauthorized_application_connection() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    wk_knowledge::Knowledge::connect(&url).await.unwrap();
    let owner = sqlx::PgPool::connect(&url).await.unwrap();
    let role = format!("wk_worker_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(&owner)
        .await
        .unwrap();
    let selected = role.clone();
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
    let k = wk_knowledge::Knowledge::from_pool(pool.clone());
    let result = wk_server::retention_worker::run(
        &k,
        async { tokio::time::sleep(Duration::from_millis(100)).await },
        Duration::from_millis(10),
    )
    .await;
    pool.close().await;
    sqlx::query(&format!("DROP ROLE {role}"))
        .execute(&owner)
        .await
        .unwrap();
    assert!(result.unwrap_err().contains("operator connection"));
}
