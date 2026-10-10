use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use serde_json::{json, Value};
use tower::ServiceExt;
use wk_knowledge::Knowledge;
fn closed() -> Knowledge {
    Knowledge::from_pool(
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://127.0.0.1:1/wk_test")
            .unwrap(),
    )
}
#[test]
fn deployment_configuration_is_validated_and_defaults_are_bounded() {
    let defaults = wk_server::runtime::Limits::parse(|_| None).unwrap();
    assert_eq!(defaults.inflight, 64);
    assert_eq!(defaults.http_timeout, std::time::Duration::from_secs(30));
    assert_eq!(defaults.pool.max_connections, 10);
    assert_eq!(
        defaults.pool.acquire_timeout,
        std::time::Duration::from_secs(5)
    );
    assert_eq!(
        defaults.pool.statement_timeout,
        std::time::Duration::from_secs(5)
    );
    assert_eq!(
        defaults.pool.lock_timeout,
        std::time::Duration::from_secs(3)
    );
    for (key, value) in [
        ("WK_MAX_INFLIGHT", "65"),
        ("WK_MAX_INFLIGHT", "0"),
        ("WK_HTTP_TIMEOUT_SECONDS", "31"),
        ("WK_POOL_MAX", "0"),
        ("WK_POOL_MAX", "33"),
        ("WK_DB_LOCK_MS", "6000"),
        ("WK_DB_STATEMENT_MS", "bad"),
    ] {
        assert!(
            wk_server::runtime::Limits::parse(|name| (name == key).then(|| value.to_string()))
                .is_err(),
            "{key}"
        );
    }
}
#[tokio::test]
async fn overload_is_shed_and_timeout_releases_its_slot() {
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal = started.clone();
    let app = Router::new()
        .route(
            "/slow",
            get(move || {
                let signal = signal.clone();
                async move {
                    signal.notify_one();
                    std::future::pending::<()>().await;
                    "unreachable"
                }
            }),
        )
        .route("/fast", get(|| async { "ok" }));
    let app = wk_server::runtime::bound(
        app,
        wk_server::runtime::Limits {
            inflight: 1,
            http_timeout: std::time::Duration::from_millis(100),
            ..Default::default()
        },
    );
    let slow = app.clone();
    let task = tokio::spawn(async move {
        slow.oneshot(Request::builder().uri("/slow").body(Body::empty()).unwrap())
            .await
            .unwrap()
    });
    started.notified().await;
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/fast").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        task.await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        app.oneshot(Request::builder().uri("/fast").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn readiness_requires_exact_schema_and_runtime_queries_have_timeouts() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let owner = sqlx::PgPool::connect(&url).await.unwrap();
    let name = format!("wk_health_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&owner)
        .await
        .unwrap();
    let mut testurl = reqwest::Url::parse(&url).unwrap();
    testurl.set_path(&format!("/{name}"));
    let pool = sqlx::PgPool::connect(testurl.as_str()).await.unwrap();
    let k = Knowledge::from_pool(pool.clone());
    k.migrate().await.unwrap();
    drop(k);
    let role = format!("wk_health_role_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE ROLE {role} NOLOGIN; GRANT USAGE ON SCHEMA public TO {role}; GRANT SELECT ON wk_schema_migrations TO {role};")).execute(&pool).await.unwrap();
    let role_name = role.clone();
    let runtime = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |c, _| {
            let role = role_name.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}")).execute(c).await?;
                Ok(())
            })
        })
        .connect(testurl.as_str())
        .await
        .unwrap();
    let app = wk_server::router(Knowledge::from_pool(runtime.clone()), vec![]);
    let request = || {
        Request::builder()
            .uri("/health/ready")
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        app.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::OK
    );
    sqlx::query("INSERT INTO wk_schema_migrations VALUES(13)")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let config = wk_knowledge::PoolConfig {
        max_connections: 1,
        acquire_timeout: std::time::Duration::from_millis(1000),
        statement_timeout: std::time::Duration::from_millis(500),
        lock_timeout: std::time::Duration::from_millis(100),
    };
    let bounded = config.connect(testurl.as_str()).await.unwrap();
    let error = sqlx::query("SELECT pg_sleep(0.8)")
        .execute(&bounded)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()),
        Some("57014".into())
    );
    let mut held = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE wk_schema_migrations IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *held)
        .await
        .unwrap();
    let error = sqlx::query("SELECT version FROM wk_schema_migrations")
        .execute(&bounded)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()),
        Some("55P03".into())
    );
    held.rollback().await.unwrap();
    let _connection = bounded.acquire().await.unwrap();
    assert!(matches!(
        bounded.acquire().await,
        Err(sqlx::Error::PoolTimedOut)
    ));
    drop(_connection);
    bounded.close().await;
    drop(app);
    runtime.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query(&format!("DROP ROLE {role}"))
        .execute(&owner)
        .await
        .unwrap();
}
#[tokio::test]
async fn liveness_does_not_need_database_and_readiness_failure_is_opaque() {
    let app = wk_server::router(closed(), vec![]);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&to_bytes(response.into_body(), 1024).await.unwrap())
            .unwrap(),
        json!({"live":true})
    );
    let began = std::time::Instant::now();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(began.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&to_bytes(response.into_body(), 1024).await.unwrap())
            .unwrap(),
        json!({"error":"unavailable"})
    );
}
