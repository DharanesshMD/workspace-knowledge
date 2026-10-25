use sqlx::Row;
use wk_knowledge::{Actor, Error, Knowledge};

fn hash() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn svc(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}
struct World {
    k: Knowledge,
    pool: sqlx::PgPool,
    /// Admin with a ceiling of backend and frontend.
    admin: Actor,
    /// Admin whose ceiling is backend only.
    narrow: Actor,
    /// An ordinary developer who signs in (a real identity), not an admin.
    ravi: Actor,
}
async fn world() -> World {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let mut people = vec![];
    for (name, services) in [
        ("admin", svc(&["backend", "frontend"])),
        ("narrow", svc(&["backend"])),
        ("ravi", svc(&["backend", "frontend"])),
    ] {
        let a = Actor {
            org: org.clone(),
            user: name.into(),
        };
        k.provision_identity(&format!("https://{org}.synthetic.example"), name, &a)
            .await
            .unwrap();
        k.grant(&a, "demo", &services, false).await.unwrap();
        people.push(a);
    }
    let ravi = people.pop().unwrap();
    let narrow = people.pop().unwrap();
    let admin = people.pop().unwrap();
    k.admin_assign(&admin, "demo", &svc(&["backend", "frontend"]))
        .await
        .unwrap();
    k.admin_assign(&narrow, "demo", &svc(&["backend"]))
        .await
        .unwrap();
    World {
        k,
        pool,
        admin,
        narrow,
        ravi,
    }
}
fn actor(w: &World, user: &str) -> Actor {
    Actor {
        org: w.admin.org.clone(),
        user: user.into(),
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn an_admin_creates_an_account_and_its_token_in_one_step() {
    let w = world().await;
    assert!(w.k.is_product_admin(&w.admin, "demo").await.unwrap());
    assert!(!w.k.is_product_admin(&w.ravi, "demo").await.unwrap());
    assert!(!w.k.is_product_admin(&w.admin, "elsewhere").await.unwrap());

    let h = hash();
    let made =
        w.k.admin_create_user(
            &w.admin,
            "demo",
            "newdev",
            &svc(&["backend"]),
            false,
            "first",
            &h,
            30,
        )
        .await
        .unwrap();
    assert_eq!(
        (made.product.as_str(), made.label.as_str()),
        ("demo", "first")
    );

    // The token opens the product as exactly that developer, with exactly those services.
    let who = w.k.personal_token_actor(&h, "demo").await.unwrap();
    assert_eq!(
        (who.org.as_str(), who.user.as_str()),
        (w.admin.org.as_str(), "newdev")
    );
    let products = w.k.products_for(&who).await.unwrap();
    assert_eq!(products.len(), 1);
    assert_eq!(products[0].services, svc(&["backend"]));
    assert!(!products[0].can_review);
    assert!(!w.k.is_product_admin(&who, "demo").await.unwrap());

    // The admin sees them, marked as token-only with token metadata, next to the signed-in developer.
    let listing = w.k.admin_users(&w.admin, "demo").await.unwrap();
    assert_eq!(listing.services, svc(&["backend", "frontend"]));
    let newdev = listing.users.iter().find(|u| u.user == "newdev").unwrap();
    assert!(newdev.token_only && newdev.tokens.len() == 1 && newdev.tokens[0].id == made.id);
    let ravi = listing.users.iter().find(|u| u.user == "ravi").unwrap();
    assert!(!ravi.token_only && ravi.tokens.is_empty());
    // No secret or hash anywhere in what the admin receives.
    assert!(!serde_json::to_string(&listing).unwrap().contains(&h));

    let actions: Vec<String> = sqlx::query(
        "SELECT action FROM audit_events WHERE org=$1 AND action LIKE 'admin_%' ORDER BY id",
    )
    .bind(&w.admin.org)
    .fetch_all(&w.pool)
    .await
    .unwrap()
    .iter()
    .map(|r| r.get("action"))
    .collect();
    assert!(actions.contains(&"admin_user_create".to_string()));
    assert!(actions.contains(&"admin_token_issue".to_string()));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn names_scopes_and_callers_it_must_refuse() {
    let w = world().await;
    let create = |who: &Actor, name: &str, services: &[&str]| {
        let (k, who, name, services) = (w.k.clone(), who.clone(), name.to_string(), svc(services));
        async move {
            k.admin_create_user(&who, "demo", &name, &services, false, "t", &hash(), 30)
                .await
                .map(|t| t.id)
        }
    };
    create(&w.admin, "taken", &["backend"]).await.unwrap();
    // A name already used by anyone is refused, including a signed-in developer's: no takeover.
    assert_eq!(
        create(&w.admin, "taken", &["backend"]).await.unwrap_err(),
        Error::Conflict
    );
    for existing in ["ravi", "admin", "narrow"] {
        assert_eq!(
            create(&w.admin, existing, &["backend"]).await.unwrap_err(),
            Error::Conflict,
            "{existing}"
        );
    }
    // A real developer who belongs only to another product still owns the name: no takeover there either.
    let ghost = actor(&w, "ghost");
    w.k.provision_identity(
        "https://elsewhere.synthetic.example",
        "ghost-subject",
        &ghost,
    )
    .await
    .unwrap();
    w.k.grant(&ghost, "other", &svc(&["backend"]), false)
        .await
        .unwrap();
    assert_eq!(
        create(&w.admin, "ghost", &["backend"]).await.unwrap_err(),
        Error::Conflict
    );
    // Malformed names never reach the table.
    let too_long = "a".repeat(33);
    for bad in [
        "",
        "Bad",
        "1x",
        "has space",
        "a/b",
        "über",
        too_long.as_str(),
    ] {
        assert_eq!(
            create(&w.admin, bad, &["backend"]).await.unwrap_err(),
            Error::Invalid,
            "{bad:?}"
        );
    }
    // Services must be present, valid and inside the admin's ceiling.
    assert_eq!(
        create(&w.admin, "nosvc", &[]).await.unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        create(&w.admin, "badsvc", &["has space"])
            .await
            .unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        create(&w.admin, "wide", &["backend", "unknown"])
            .await
            .unwrap_err(),
        Error::Denied
    );
    assert_eq!(
        create(&w.narrow, "wide", &["frontend"]).await.unwrap_err(),
        Error::Denied
    );
    create(&w.narrow, "within", &["backend"]).await.unwrap();
    // A developer who is not an admin cannot create anyone.
    assert_eq!(
        create(&w.ravi, "byravi", &["backend"]).await.unwrap_err(),
        Error::Denied
    );
    // Nothing was left behind by any refusal.
    let names: Vec<String> =
        w.k.admin_users(&w.admin, "demo")
            .await
            .unwrap()
            .users
            .into_iter()
            .map(|u| u.user)
            .collect();
    for absent in ["wide", "badsvc", "nosvc", "byravi", "Bad"] {
        assert!(!names.contains(&absent.to_string()), "{absent}");
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn the_account_and_token_succeed_or_fail_together() {
    let w = world().await;
    let shared = hash();
    w.k.admin_create_user(
        &w.admin,
        "demo",
        "first",
        &svc(&["backend"]),
        false,
        "a",
        &shared,
        30,
    )
    .await
    .unwrap();
    // The token step fails (the hash already exists), after the account step already ran.
    assert!(w
        .k
        .admin_create_user(
            &w.admin,
            "demo",
            "second",
            &svc(&["backend"]),
            false,
            "b",
            &shared,
            30
        )
        .await
        .is_err());
    let users = w.k.admin_users(&w.admin, "demo").await.unwrap().users;
    assert!(
        users.iter().all(|u| u.user != "second"),
        "the half-made account was rolled back"
    );
    let leftover: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identities WHERE org=$1 AND user_id='second'")
            .bind(&w.admin.org)
            .fetch_one(&w.pool)
            .await
            .unwrap();
    assert_eq!(leftover, 0);
    // And the name is still free.
    w.k.admin_create_user(
        &w.admin,
        "demo",
        "second",
        &svc(&["backend"]),
        false,
        "b",
        &hash(),
        30,
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn reissuing_replaces_the_tokens_and_only_for_accounts_an_admin_created() {
    let w = world().await;
    let (old, new) = (hash(), hash());
    w.k.admin_create_user(
        &w.admin,
        "demo",
        "dev",
        &svc(&["backend", "frontend"]),
        false,
        "v1",
        &old,
        30,
    )
    .await
    .unwrap();
    w.k.admin_reissue_token(&w.admin, "demo", "dev", "v2", &new, 7)
        .await
        .unwrap();
    assert_eq!(
        w.k.personal_token_actor(&old, "demo").await.err().unwrap(),
        Error::Denied
    );
    assert_eq!(
        w.k.personal_token_actor(&new, "demo").await.unwrap().user,
        "dev"
    );
    let listing = w.k.admin_users(&w.admin, "demo").await.unwrap();
    let dev = listing.users.iter().find(|u| u.user == "dev").unwrap();
    assert_eq!(dev.tokens.len(), 1);
    assert_eq!(dev.tokens[0].label, "v2");
    // A signed-in developer, an unknown name, a non-admin and an admin whose ceiling does not
    // cover the account are all refused, and nothing changes.
    for (who, target) in [
        (&w.admin, "ravi"),
        (&w.admin, "nobody"),
        (&w.ravi, "dev"),
        (&w.narrow, "dev"),
    ] {
        assert_eq!(
            w.k.admin_reissue_token(who, "demo", target, "x", &hash(), 7)
                .await
                .unwrap_err(),
            Error::Denied,
            "{} -> {target}",
            who.user
        );
    }
    assert!(w.k.personal_token_actor(&new, "demo").await.is_ok());
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn removing_a_developer_revokes_the_token_and_frees_the_name() {
    let w = world().await;
    let (first, second) = (hash(), hash());
    w.k.admin_create_user(
        &w.admin,
        "demo",
        "leaver",
        &svc(&["backend"]),
        false,
        "a",
        &first,
        30,
    )
    .await
    .unwrap();
    w.k.admin_remove_user(&w.admin, "demo", "leaver")
        .await
        .unwrap();
    assert_eq!(
        w.k.personal_token_actor(&first, "demo")
            .await
            .err()
            .unwrap(),
        Error::Denied
    );
    let gone: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identities WHERE org=$1 AND user_id='leaver'")
            .bind(&w.admin.org)
            .fetch_one(&w.pool)
            .await
            .unwrap();
    assert_eq!(gone, 0);
    // The name can be used again, by a new account that inherits nothing.
    w.k.admin_create_user(
        &w.admin,
        "demo",
        "leaver",
        &svc(&["backend"]),
        false,
        "b",
        &second,
        30,
    )
    .await
    .unwrap();
    assert_eq!(
        w.k.personal_token_actor(&first, "demo")
            .await
            .err()
            .unwrap(),
        Error::Denied
    );
    assert!(w.k.personal_token_actor(&second, "demo").await.is_ok());

    // An account that still belongs to another product is kept (only its demo access ends).
    let (dual, other) = (hash(), hash());
    w.k.admin_create_user(
        &w.admin,
        "demo",
        "dual",
        &svc(&["backend"]),
        false,
        "a",
        &dual,
        30,
    )
    .await
    .unwrap();
    let dual_actor = actor(&w, "dual");
    w.k.grant(&dual_actor, "other", &svc(&["backend"]), false)
        .await
        .unwrap();
    w.k.create_personal_token(&dual_actor, "other", "o", &other, 30)
        .await
        .unwrap();
    w.k.admin_remove_user(&w.admin, "demo", "dual")
        .await
        .unwrap();
    assert_eq!(
        w.k.personal_token_actor(&dual, "demo").await.err().unwrap(),
        Error::Denied
    );
    assert!(w.k.personal_token_actor(&other, "other").await.is_ok());
    assert_eq!(
        w.k.admin_create_user(
            &w.admin,
            "demo",
            "dual",
            &svc(&["backend"]),
            false,
            "c",
            &hash(),
            30
        )
        .await
        .unwrap_err(),
        Error::Conflict
    );
    // A non-admin and an admin without authority over the member cannot remove anyone.
    assert_eq!(
        w.k.admin_remove_user(&w.ravi, "demo", "second")
            .await
            .unwrap_err(),
        Error::Denied
    );
    w.k.admin_create_user(
        &w.admin,
        "demo",
        "wide",
        &svc(&["backend", "frontend"]),
        false,
        "w",
        &hash(),
        30,
    )
    .await
    .unwrap();
    assert_eq!(
        w.k.admin_remove_user(&w.narrow, "demo", "wide")
            .await
            .unwrap_err(),
        Error::Denied
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn a_signed_in_developer_can_still_be_removed_but_keeps_their_identity() {
    let w = world().await;
    w.k.admin_remove_user(&w.admin, "demo", "ravi")
        .await
        .unwrap();
    let identity: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identities WHERE org=$1 AND user_id='ravi'")
            .bind(&w.admin.org)
            .fetch_one(&w.pool)
            .await
            .unwrap();
    assert_eq!(
        identity, 1,
        "a real identity-provider identity is never deleted by an admin"
    );
}
