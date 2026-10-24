use wk_contracts::CardSubmission;
use wk_knowledge::{Actor, AutoPublish, Error, Knowledge};

fn card(key: &str, completeness: &str, provenance: Option<&str>) -> CardSubmission {
    let mut v = serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Cap retries","intent":"Avoid repeat charges","rationale":"The gateway charges twice on duplicate calls.","decisions":["cap at three"],"constraints":[],"open_questions":[],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":completeness,"base_revision":null});
    if let Some(p) = provenance {
        v["provenance"] = serde_json::json!(p);
    }
    serde_json::from_value(v).unwrap()
}
struct World {
    k: Knowledge,
    pool: sqlx::PgPool,
    author: Actor,
    member: Actor,
    outsider: Actor,
}
/// `author` cannot review; `demo` is enabled for publish on submit, `closed` is not.
async fn setup() -> World {
    setup_with(AutoPublish::Only(vec!["demo".into()])).await
}
async fn setup_with(setting: AutoPublish) -> World {
    let url = std::env::var("WK_TEST_DATABASE_URL")
        .expect("set WK_TEST_DATABASE_URL to a disposable /wk_test database");
    assert!(
        url.split('?').next().unwrap().ends_with("/wk_test"),
        "refusing non-test database"
    );
    let k = Knowledge::connect(&url)
        .await
        .unwrap()
        .with_auto_publish_products(setting)
        .unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let actor = |user: &str| Actor {
        org: org.clone(),
        user: user.into(),
    };
    let (author, member, outsider) = (actor("author"), actor("member"), actor("outsider"));
    for product in ["demo", "closed"] {
        k.grant(&author, product, &["backend".into()], false)
            .await
            .unwrap();
        k.grant(&member, product, &["backend".into()], false)
            .await
            .unwrap();
    }
    World {
        k,
        pool,
        author,
        member,
        outsider,
    }
}
async fn audit(w: &World, action: &str, id: uuid::Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_events WHERE org=$1 AND action=$2 AND entity_id=$3",
    )
    .bind(&w.author.org)
    .bind(action)
    .bind(id)
    .fetch_one(&w.pool)
    .await
    .unwrap()
}
async fn graph_nodes(w: &World, id: uuid::Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM graph_nodes WHERE card_id=$1")
        .bind(id)
        .fetch_one(&w.pool)
        .await
        .unwrap()
}
async fn stored(w: &World) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM cards WHERE org=$1")
        .bind(&w.author.org)
        .fetch_one(&w.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn auto_publish_products_must_be_valid_ids() {
    // Lazy pool: nothing connects, so this needs no database.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1/wk_test")
        .unwrap();
    let k = Knowledge::from_pool(pool);
    for bad in ["", "a b", "a,b", "é"] {
        assert!(k
            .clone()
            .with_auto_publish_products(AutoPublish::Only(vec![bad.into()]))
            .is_err());
    }
    assert!(k
        .clone()
        .with_auto_publish_products(AutoPublish::Only(vec!["demo".into()]))
        .is_ok());
    assert!(k
        .clone()
        .with_auto_publish_products(AutoPublish::All)
        .is_ok());
    assert!(k.with_auto_publish_products(AutoPublish::Off).is_ok());
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn enabled_product_publishes_the_authors_assistant_card_for_others_to_find() {
    let w = setup().await;
    for (key, completeness) in [("p1", "partial"), ("p2", "complete")] {
        let c =
            w.k.submit_publishing(
                &w.author,
                "demo",
                card(key, completeness, Some("assistant_written")),
                true,
            )
            .await
            .unwrap();
        assert!(c.published, "{key}");
        assert!(w.k.get(&w.member, "demo", c.id).await.unwrap().published);
        assert_eq!(audit(&w, "publish_on_submit", c.id).await, 1);
        let nodes = graph_nodes(&w, c.id).await;
        assert!(nodes > 0, "published cards enter the graph");
    }
    assert_eq!(
        w.k.search(&w.member, "demo", "retries", 10)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn resubmitting_a_published_card_is_idempotent() {
    let w = setup().await;
    let body = || card("again", "partial", Some("assistant_written"));
    let first =
        w.k.submit_publishing(&w.author, "demo", body(), true)
            .await
            .unwrap();
    let before = graph_nodes(&w, first.id).await;
    let second =
        w.k.submit_publishing(&w.author, "demo", body(), true)
            .await
            .unwrap();
    assert_eq!(second.id, first.id);
    assert!(second.published);
    assert_eq!(graph_nodes(&w, first.id).await, before);
    assert_eq!(audit(&w, "publish_on_submit", first.id).await, 1);
    // An earlier plain submit of the same card is published by a later opted-in retry.
    let plain =
        w.k.submit(
            &w.author,
            "demo",
            card("late", "partial", Some("assistant_written")),
        )
        .await
        .unwrap();
    assert!(!plain.published);
    let later =
        w.k.submit_publishing(
            &w.author,
            "demo",
            card("late", "partial", Some("assistant_written")),
            true,
        )
        .await
        .unwrap();
    assert_eq!((later.id, later.published), (plain.id, true));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn disabled_product_or_missing_flag_stores_an_ordinary_review_copy() {
    let w = setup().await;
    let assistant = || card("q1", "partial", Some("assistant_written"));
    let closed =
        w.k.submit_publishing(&w.author, "closed", assistant(), true)
            .await
            .unwrap();
    let unflagged =
        w.k.submit_publishing(
            &w.author,
            "demo",
            card("q2", "partial", Some("assistant_written")),
            false,
        )
        .await
        .unwrap();
    for c in [&closed, &unflagged] {
        assert!(!c.published);
        assert_eq!(audit(&w, "publish_on_submit", c.id).await, 0);
    }
    assert!(w
        .k
        .search(&w.member, "closed", "retries", 10)
        .await
        .unwrap()
        .is_empty());
    assert!(w
        .k
        .search(&w.member, "demo", "retries", 10)
        .await
        .unwrap()
        .is_empty());
    // The owner of an unreviewed copy still cannot publish it without can_review.
    assert_eq!(
        w.k.publish(&w.author, "demo", unflagged.id)
            .await
            .unwrap_err(),
        Error::Denied
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn only_assistant_written_cards_publish_on_submit() {
    let w = setup().await;
    // Rule drafts, hook captures and imports carry no provenance, whatever the completeness.
    for (key, completeness) in [("r1", "complete"), ("r2", "partial")] {
        let c =
            w.k.submit_publishing(&w.author, "demo", card(key, completeness, None), true)
                .await
                .unwrap();
        assert!(!c.published, "{key}");
        assert_eq!(audit(&w, "publish_on_submit", c.id).await, 0);
    }
    assert!(w
        .k
        .search(&w.member, "demo", "retries", 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn sensitive_text_is_rejected_and_nothing_is_stored() {
    let w = setup().await;
    let mut body = card("s1", "partial", Some("assistant_written"));
    body.rationale = "password = CANARY".into();
    assert_eq!(
        w.k.submit_publishing(&w.author, "demo", body, true)
            .await
            .unwrap_err(),
        Error::Invalid
    );
    assert_eq!(stored(&w).await, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn non_members_and_service_or_policy_mismatches_are_denied_without_storing() {
    let w = setup().await;
    let assistant = |key: &str| card(key, "partial", Some("assistant_written"));
    assert_eq!(
        w.k.submit_publishing(&w.outsider, "demo", assistant("d1"), true)
            .await
            .unwrap_err(),
        Error::Denied
    );
    let mut wider = assistant("d2");
    wider.service_ids = vec!["backend".into(), "frontend".into()];
    assert_eq!(
        w.k.submit_publishing(&w.author, "demo", wider, true)
            .await
            .unwrap_err(),
        Error::Denied
    );
    let mut stale = assistant("d3");
    stale.policy_revision = 99;
    assert_eq!(
        w.k.submit_publishing(&w.author, "demo", stale, true)
            .await
            .unwrap_err(),
        Error::Denied
    );
    assert_eq!(stored(&w).await, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn revoked_author_cannot_publish_on_submit() {
    let w = setup().await;
    w.k.revoke(&w.author, "demo").await.unwrap();
    assert_eq!(
        w.k.submit_publishing(
            &w.author,
            "demo",
            card("v1", "partial", Some("assistant_written")),
            true
        )
        .await
        .unwrap_err(),
        Error::Denied
    );
    assert_eq!(stored(&w).await, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn author_withdraws_an_auto_published_card_and_it_leaves_search() {
    let w = setup().await;
    let c =
        w.k.submit_publishing(
            &w.author,
            "demo",
            card("w1", "partial", Some("assistant_written")),
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        w.k.search(&w.member, "demo", "retries", 10)
            .await
            .unwrap()
            .len(),
        1
    );
    // Another non-reviewer cannot withdraw it; its author can.
    assert_eq!(
        w.k.withdraw(&w.member, "demo", c.id, c.revision)
            .await
            .unwrap_err(),
        Error::NotFound
    );
    w.k.withdraw(&w.author, "demo", c.id, c.revision)
        .await
        .unwrap();
    assert!(w
        .k
        .search(&w.member, "demo", "retries", 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        w.k.get(&w.member, "demo", c.id).await.unwrap_err(),
        Error::NotFound
    );
    let nodes = graph_nodes(&w, c.id).await;
    assert_eq!(nodes, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn policy_advertises_publish_on_submit_only_for_enabled_products() {
    let w = setup().await;
    let features = |p: wk_knowledge::lifecycle::Policy| p.features;
    assert_eq!(
        features(w.k.policy(&w.author, "demo").await.unwrap()),
        ["card_provenance", "publish_on_submit"]
    );
    assert_eq!(
        features(w.k.policy(&w.author, "closed").await.unwrap()),
        ["card_provenance"]
    );
    let by_product = w.k.products_for(&w.author).await.unwrap();
    let listed = |name: &str| {
        by_product
            .iter()
            .find(|p| p.product == name)
            .unwrap()
            .features
            .clone()
    };
    assert!(listed("demo").contains(&"publish_on_submit".into()));
    assert!(!listed("closed").contains(&"publish_on_submit".into()));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn default_enables_every_product_and_none_disables_all() {
    for (setting, demo, closed) in [
        (AutoPublish::default(), true, true),
        (AutoPublish::All, true, true),
        (AutoPublish::Off, false, false),
        (AutoPublish::Only(vec!["closed".into()]), false, true),
    ] {
        let w = setup_with(setting.clone()).await;
        for (product, enabled) in [("demo", demo), ("closed", closed)] {
            let features = w.k.policy(&w.author, product).await.unwrap().features;
            assert_eq!(
                features.contains(&"publish_on_submit".into()),
                enabled,
                "{setting:?} {product}"
            );
            let c =
                w.k.submit_publishing(
                    &w.author,
                    product,
                    card(
                        &format!("d-{product}"),
                        "partial",
                        Some("assistant_written"),
                    ),
                    true,
                )
                .await
                .unwrap();
            assert_eq!(c.published, enabled, "{setting:?} {product}");
        }
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn default_still_needs_the_flag_to_publish() {
    let w = setup_with(AutoPublish::default()).await;
    let c =
        w.k.submit_publishing(
            &w.author,
            "demo",
            card("noflag", "partial", Some("assistant_written")),
            false,
        )
        .await
        .unwrap();
    assert!(!c.published);
    assert_eq!(audit(&w, "publish_on_submit", c.id).await, 0);
    assert_eq!(
        w.k.get(&w.member, "demo", c.id).await.unwrap_err(),
        Error::NotFound
    );
}
