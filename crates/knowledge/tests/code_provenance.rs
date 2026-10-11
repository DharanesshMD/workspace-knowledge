mod common;
use wk_knowledge::{code_provenance::*, Actor, Error};

fn repo(id: uuid::Uuid) -> RepositoryInput {
    RepositoryInput {
        id,
        services: vec!["backend".into()],
        high_risk_paths: vec!["api/".into()],
        contact_hosts: vec!["chat.synthetic.example".into()],
        base_revision: None,
    }
}
fn profile() -> ProfileInput {
    ProfileInput {
        display_name: Some("Alice Synthetic".into()),
        contact_url: Some("https://chat.synthetic.example/alice".into()),
        services: vec!["backend".into()],
        base_revision: None,
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn repository_registration_requires_operator_and_current_scope() {
    let f = common::setup().await;
    let id = uuid::Uuid::new_v4();
    assert_eq!(
        f.k.configure_repository(&f.bob, "demo", repo(id))
            .await
            .unwrap_err(),
        Error::Denied
    );
    let r =
        f.k.configure_repository(&f.admin, "demo", repo(id))
            .await
            .unwrap();
    assert_eq!(r.revision, 1);
    assert_eq!(
        f.k.configure_repository(&f.admin, "demo", repo(id))
            .await
            .unwrap_err(),
        Error::Conflict
    );
    let mut update = repo(id);
    update.base_revision = Some(1);
    update.services.push("frontend".into());
    assert_eq!(
        f.k.configure_repository(&f.admin, "demo", update)
            .await
            .unwrap()
            .revision,
        2
    );
    let outsider = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: f.admin.user.clone(),
    };
    assert!(f
        .k
        .configure_repository(&outsider, "demo", repo(id))
        .await
        .is_err());
    let (k, pool, role) = common::runtime(&f).await;
    assert_eq!(
        k.configure_repository(&f.admin, "demo", repo(uuid::Uuid::new_v4()))
            .await
            .unwrap_err(),
        Error::Denied
    );
    common::cleanup_runtime(&f, pool, role).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn profile_contact_is_scoped_opt_in_and_revision_checked() {
    let f = common::setup().await;
    f.k.configure_repository(&f.admin, "demo", repo(uuid::Uuid::new_v4()))
        .await
        .unwrap();
    let (k, pool, role) = common::runtime(&f).await;
    let saved = k
        .put_code_profile(&f.alice, "demo", profile())
        .await
        .unwrap();
    assert_eq!(saved.user_id, "alice");
    assert_eq!(saved.revision, 1);
    assert_eq!(
        k.put_code_profile(&f.alice, "demo", profile())
            .await
            .unwrap_err(),
        Error::Conflict
    );
    for contact in [
        "http://chat.synthetic.example/alice",
        "https://evil.example/alice",
        "https://user:password@chat.synthetic.example/alice",
        "https://chat.synthetic.example/alice?token=secret",
        "javascript:alert(1)",
        "https://chat.synthetic.example/alice\n",
    ] {
        let mut bad = profile();
        bad.contact_url = Some(contact.into());
        bad.base_revision = Some(1);
        assert_eq!(
            k.put_code_profile(&f.alice, "demo", bad).await.unwrap_err(),
            Error::Invalid
        );
    }
    let mut withdrawal = profile();
    withdrawal.display_name = None;
    withdrawal.contact_url = None;
    withdrawal.base_revision = Some(1);
    assert!(
        k.put_code_profile(&f.alice, "demo", withdrawal)
            .await
            .unwrap()
            .revoked
    );
    let mut wider = profile();
    wider.services.push("frontend".into());
    assert_eq!(
        k.put_code_profile(&f.reader, "demo", wider)
            .await
            .unwrap_err(),
        Error::Denied
    );
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('wk.org',$1,true),set_config('wk.user',$2,true)")
        .bind(&f.alice.org)
        .bind(&f.bob.user)
        .execute(&mut *tx)
        .await
        .unwrap();
    let changed=sqlx::query("UPDATE code_profiles SET display_name='Forged name' WHERE org=$1 AND product='demo' AND user_id='alice'").bind(&f.alice.org).execute(&mut *tx).await.unwrap();
    assert_eq!(changed.rows_affected(), 0);
    tx.rollback().await.unwrap();
    common::cleanup_runtime(&f, pool, role).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn repository_scope_narrowing_hides_existing_mapping_from_runtime() {
    let f = common::setup().await;
    let id = uuid::Uuid::new_v4();
    f.k.configure_repository(&f.admin, "demo", repo(id))
        .await
        .unwrap();
    f.k.configure_code_mapping(
        &f.admin,
        "demo",
        MappingInput {
            repository_id: id,
            user_id: "alice".into(),
            identity_reference: "team:alice".into(),
            proof_reference: "Developer confirmed identity".into(),
            services: vec!["backend".into()],
            base_revision: None,
        },
    )
    .await
    .unwrap();
    let mut wider = repo(id);
    wider.base_revision = Some(1);
    wider.services.push("frontend".into());
    f.k.configure_repository(&f.admin, "demo", wider)
        .await
        .unwrap();
    let (k, pool, role) = common::runtime(&f).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('wk.org',$1,true),set_config('wk.user',$2,true)")
        .bind(&f.reader.org)
        .bind(&f.reader.user)
        .execute(&mut *tx)
        .await
        .unwrap();
    let leaked: i64 =
        sqlx::query_scalar("SELECT count(*) FROM code_mappings WHERE org=$1 AND product='demo'")
            .bind(&f.reader.org)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(leaked, 0);
    tx.rollback().await.unwrap();
    drop(k);
    common::cleanup_runtime(&f, pool, role).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn identity_mapping_cannot_grant_access_or_accept_email_matching() {
    let f = common::setup().await;
    let id = uuid::Uuid::new_v4();
    f.k.configure_repository(&f.admin, "demo", repo(id))
        .await
        .unwrap();
    let input = MappingInput {
        repository_id: id,
        user_id: "alice".into(),
        identity_reference: "team:alice".into(),
        proof_reference: "Developer confirmed identity in team review".into(),
        services: vec!["backend".into()],
        base_revision: None,
    };
    assert_eq!(
        f.k.configure_code_mapping(&f.bob, "demo", input.clone())
            .await
            .unwrap_err(),
        Error::Denied
    );
    let saved =
        f.k.configure_code_mapping(&f.admin, "demo", input.clone())
            .await
            .unwrap();
    assert_eq!(saved.assurance, "team_attested");
    let mut bad = input.clone();
    bad.identity_reference = "alice@example.com".into();
    assert_eq!(
        f.k.configure_code_mapping(&f.admin, "demo", bad)
            .await
            .unwrap_err(),
        Error::Invalid
    );
    let mut bad = input;
    bad.user_id = "not-a-member".into();
    assert_eq!(
        f.k.configure_code_mapping(&f.admin, "demo", bad)
            .await
            .unwrap_err(),
        Error::Denied
    );
}
