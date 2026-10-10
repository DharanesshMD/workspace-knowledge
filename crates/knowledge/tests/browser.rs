use wk_knowledge::{Actor, Knowledge};

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn session_lookup_enforces_expiry_and_current_identity() {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let owner = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "owner".into(),
    };
    let other = Actor {
        org: owner.org.clone(),
        user: "other".into(),
    };
    let subject = uuid::Uuid::new_v4().to_string();
    k.provision_identity("https://issuer.example", &subject, &owner)
        .await
        .unwrap();
    k.grant(&owner, "demo", &["backend".into()], false)
        .await
        .unwrap();
    k.grant(&other, "other-product", &["frontend".into()], false)
        .await
        .unwrap();
    let products = k.products_for(&owner).await.unwrap();
    assert_eq!(products.len(), 1);
    assert_eq!(products[0].product, "demo");
    let hash = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let csrf = "b".repeat(64);
    k.create_browser_session(&hash, "https://issuer.example", &subject, &csrf)
        .await
        .unwrap();
    let session = k.browser_session(&hash).await.unwrap();
    assert_eq!(session.subject, subject);
    k.provision_identity("https://issuer.example", &subject, &other)
        .await
        .unwrap();
    assert_eq!(
        k.identity(&session.issuer, &session.subject)
            .await
            .unwrap()
            .user,
        "other"
    );
    k.revoke(&owner, "demo").await.unwrap();
    assert!(k.products_for(&owner).await.unwrap().is_empty());
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::query(
        "UPDATE browser_sessions SET last_seen=now()-interval '31 minutes' WHERE session_hash=$1",
    )
    .bind(&hash)
    .execute(&pool)
    .await
    .unwrap();
    assert!(k.browser_session(&hash).await.is_err());
    sqlx::query("UPDATE browser_sessions SET last_seen=now(),expires_at=now()-interval '1 second' WHERE session_hash=$1").bind(&hash).execute(&pool).await.unwrap();
    assert!(k.browser_session(&hash).await.is_err());
    k.delete_browser_session(&hash).await.unwrap();
    assert!(k.browser_session(&hash).await.is_err());
    assert!(k
        .create_browser_session("not-a-hash", "https://issuer.example", &subject, &csrf)
        .await
        .is_err());
}
