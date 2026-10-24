use wk_knowledge::share::{ShareError, ShareRequest, ShareStatus};
use wk_knowledge::{Actor, AutoPublish, Error, Knowledge};

fn share(title: &str) -> ShareRequest {
    ShareRequest {
        service: None,
        title: title.into(),
        intent: "Avoid repeat charges".into(),
        rationale: "The gateway charges twice on duplicate calls.".into(),
        decisions: vec!["cap at three".into()],
        constraints: vec![],
        open_questions: vec!["Should the cap be configurable?".into()],
    }
}
struct World {
    k: Knowledge,
    pool: sqlx::PgPool,
    /// Holds only `backend`.
    author: Actor,
    member: Actor,
    /// Holds `backend` and `frontend`.
    multi: Actor,
    outsider: Actor,
}
/// `demo` is enabled for publish on submit, `closed` is not.
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
    let (author, member, multi, outsider) = (
        actor("author"),
        actor("member"),
        actor("multi"),
        actor("outsider"),
    );
    for product in ["demo", "closed"] {
        for a in [&author, &member] {
            k.grant(a, product, &["backend".into()], false)
                .await
                .unwrap();
        }
        k.grant(
            &multi,
            product,
            &["backend".into(), "frontend".into()],
            false,
        )
        .await
        .unwrap();
    }
    World {
        k,
        pool,
        author,
        member,
        multi,
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
async fn stored(w: &World) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM cards WHERE org=$1")
        .bind(&w.author.org)
        .fetch_one(&w.pool)
        .await
        .unwrap()
}
async fn found(w: &World, product: &str) -> usize {
    w.k.search(&w.member, product, "retries", 10)
        .await
        .unwrap()
        .len()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn enabled_product_publishes_the_summary_for_teammates_to_find() {
    let w = setup().await;
    let out =
        w.k.share_session(&w.author, "demo", share("Cap retries"))
            .await
            .unwrap();
    assert!(out.published);
    assert_eq!(out.status, ShareStatus::Published);
    assert_eq!(audit(&w, "publish_on_submit", out.card_id).await, 1);
    // A teammate finds it by search and reads the server-set fields.
    let hits = w.k.search(&w.member, "demo", "retries", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, out.card_id);
    let card = w.k.get(&w.member, "demo", out.card_id).await.unwrap();
    let body = serde_json::to_value(&card.body).unwrap();
    assert_eq!(body["provenance"], "assistant_written");
    assert_eq!(body["capture_completeness"], "partial");
    assert_eq!(body["lifecycle"], "proposed");
    assert_eq!(body["service_ids"], serde_json::json!(["backend"]));
    assert_eq!(body["evidence"], serde_json::json!([]));
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["policy_revision"], 1);
    assert_eq!(body["decisions"], serde_json::json!(["cap at three"]));
    assert_eq!(
        body["idempotency_key"].as_str().unwrap().len(),
        64,
        "sha256 hex"
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn product_not_enabled_stores_a_review_copy_only() {
    for (setting, product) in [
        (AutoPublish::Only(vec!["demo".into()]), "closed"),
        (AutoPublish::Off, "demo"),
    ] {
        let w = setup_with(setting).await;
        let out =
            w.k.share_session(&w.author, product, share("Cap retries"))
                .await
                .unwrap();
        assert!(!out.published, "{product}");
        assert_eq!(out.status, ShareStatus::SubmittedForReview);
        assert_eq!(audit(&w, "publish_on_submit", out.card_id).await, 0);
        assert_eq!(found(&w, product).await, 0);
        // The author still sees their own review copy, and it is not shared.
        assert!(
            !w.k.get(&w.author, product, out.card_id)
                .await
                .unwrap()
                .published
        );
        assert_eq!(
            w.k.get(&w.member, product, out.card_id).await.unwrap_err(),
            Error::NotFound
        );
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn several_services_need_a_choice_and_the_service_must_be_held() {
    let w = setup().await;
    // Ambiguous: the error lists what the member holds, and nothing is stored.
    assert_eq!(
        w.k.share_session(&w.multi, "demo", share("Cap retries"))
            .await
            .unwrap_err(),
        ShareError::ServiceRequired(vec!["backend".into(), "frontend".into()])
    );
    assert_eq!(stored(&w).await, 0);
    // A held service works.
    let mut pick = share("Cap retries");
    pick.service = Some("frontend".into());
    let out = w.k.share_session(&w.multi, "demo", pick).await.unwrap();
    let card = w.k.get(&w.multi, "demo", out.card_id).await.unwrap();
    assert_eq!(card.body.service_ids, vec!["frontend".to_string()]);
    // A service the member does not hold is denied, with or without other services.
    for who in [&w.author, &w.multi] {
        let mut other = share("Cap retries");
        other.service = Some("payments".into());
        assert_eq!(
            w.k.share_session(who, "demo", other).await.unwrap_err(),
            ShareError::Failed(Error::Denied)
        );
    }
    let mut frontend = share("Cap retries");
    frontend.service = Some("frontend".into());
    assert_eq!(
        w.k.share_session(&w.author, "demo", frontend)
            .await
            .unwrap_err(),
        ShareError::Failed(Error::Denied)
    );
    assert_eq!(stored(&w).await, 1);
    // A single service is used without being named, and naming it is also fine.
    let mut named = share("Cap retries again");
    named.service = Some("backend".into());
    w.k.share_session(&w.author, "demo", named).await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn sensitive_text_is_rejected_and_nothing_is_stored() {
    let w = setup().await;
    for field in [
        "title",
        "intent",
        "rationale",
        "decision",
        "constraint",
        "question",
    ] {
        let mut r = share("Cap retries");
        let leak = "password = CANARY".to_string();
        match field {
            "title" => r.title = leak,
            "intent" => r.intent = leak,
            "rationale" => r.rationale = leak,
            "decision" => r.decisions.push(leak),
            "constraint" => r.constraints.push(leak),
            _ => r.open_questions.push(leak),
        }
        assert_eq!(
            w.k.share_session(&w.author, "demo", r).await.unwrap_err(),
            ShareError::Sensitive,
            "{field}"
        );
    }
    assert_eq!(stored(&w).await, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn the_same_share_twice_is_one_card() {
    let w = setup().await;
    let first =
        w.k.share_session(&w.author, "demo", share("Cap retries"))
            .await
            .unwrap();
    let again =
        w.k.share_session(&w.author, "demo", share("Cap retries"))
            .await
            .unwrap();
    assert_eq!(again.card_id, first.card_id);
    assert!(again.published);
    assert_eq!(stored(&w).await, 1);
    assert_eq!(audit(&w, "publish_on_submit", first.card_id).await, 1);
    assert_eq!(found(&w, "demo").await, 1);
    // Any change to the content is a new card; so is another author sharing the same text.
    let other =
        w.k.share_session(&w.author, "demo", share("Cap retries v2"))
            .await
            .unwrap();
    assert_ne!(other.card_id, first.card_id);
    let theirs =
        w.k.share_session(&w.member, "demo", share("Cap retries"))
            .await
            .unwrap();
    assert_ne!(theirs.card_id, first.card_id);
    assert_eq!(stored(&w).await, 3);
    // A review copy is idempotent too.
    let closed =
        w.k.share_session(&w.author, "closed", share("Cap retries"))
            .await
            .unwrap();
    let closed_again =
        w.k.share_session(&w.author, "closed", share("Cap retries"))
            .await
            .unwrap();
    assert_eq!(closed.card_id, closed_again.card_id);
    assert!(!closed_again.published);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn sharing_the_same_text_after_withdrawing_it_is_a_conflict_not_a_republish() {
    let w = setup().await;
    let out =
        w.k.share_session(&w.author, "demo", share("Cap retries"))
            .await
            .unwrap();
    w.k.withdraw(&w.author, "demo", out.card_id, out.revision)
        .await
        .unwrap();
    assert_eq!(found(&w, "demo").await, 0);
    assert_eq!(
        w.k.share_session(&w.author, "demo", share("Cap retries"))
            .await
            .unwrap_err(),
        ShareError::Failed(Error::Conflict)
    );
    assert_eq!(found(&w, "demo").await, 0);
    // Changed text shares normally.
    assert!(
        w.k.share_session(&w.author, "demo", share("Cap retries, revised"))
            .await
            .unwrap()
            .published
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn non_members_and_revoked_members_are_denied() {
    let w = setup().await;
    assert_eq!(
        w.k.share_session(&w.outsider, "demo", share("Cap retries"))
            .await
            .unwrap_err(),
        ShareError::Failed(Error::Denied)
    );
    assert_eq!(
        w.k.share_session(&w.author, "no-such-product", share("Cap retries"))
            .await
            .unwrap_err(),
        ShareError::Failed(Error::Denied)
    );
    w.k.revoke(&w.author, "demo").await.unwrap();
    assert_eq!(
        w.k.share_session(&w.author, "demo", share("Cap retries"))
            .await
            .unwrap_err(),
        ShareError::Failed(Error::Denied)
    );
    assert_eq!(stored(&w).await, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn the_card_field_limits_still_apply() {
    let w = setup().await;
    let mut cases = vec![];
    let mut long_title = share("x");
    long_title.title = "t".repeat(257);
    cases.push(long_title);
    let mut empty_title = share("x");
    empty_title.title = "  ".into();
    cases.push(empty_title);
    let mut empty_intent = share("x");
    empty_intent.intent = String::new();
    cases.push(empty_intent);
    let mut huge = share("x");
    huge.decisions = vec!["d".repeat(70_000)];
    cases.push(huge);
    for r in cases {
        assert_eq!(
            w.k.share_session(&w.author, "demo", r).await.unwrap_err(),
            ShareError::Failed(Error::Invalid)
        );
    }
    assert_eq!(stored(&w).await, 0);
    let mut longest = share("x");
    longest.title = "t".repeat(256);
    assert!(
        w.k.share_session(&w.author, "demo", longest)
            .await
            .unwrap()
            .published
    );
}

#[test]
fn a_caller_cannot_name_provenance_or_any_server_set_field() {
    for field in [
        "provenance",
        "policy_revision",
        "lifecycle",
        "capture_completeness",
        "evidence",
        "idempotency_key",
        "service_ids",
    ] {
        let mut v = serde_json::json!({"title":"t","intent":"i","rationale":"r"});
        v[field] = serde_json::json!("assistant_written");
        assert!(
            serde_json::from_value::<ShareRequest>(v).is_err(),
            "{field}"
        );
    }
    let ok: ShareRequest =
        serde_json::from_value(serde_json::json!({"title":"t","intent":"i","rationale":"r"}))
            .unwrap();
    assert!(ok.decisions.is_empty() && ok.service.is_none());
}
