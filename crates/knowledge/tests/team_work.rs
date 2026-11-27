use serde_json::Value;
use sqlx::Row;
use wk_contracts::CardSubmission;
use wk_knowledge::{
    activity::{ActivityUpdate, Notice, OverlapQuery},
    share::ShareRequest,
    team_work::TeamWorkQuery,
    Actor, AutoPublish, Error, Knowledge,
};

struct World {
    k: Knowledge,
    pool: sqlx::PgPool,
    anika: Actor,
    sam: Actor,
    samira: Actor,
    jo: Actor,
    /// Backend only: must never see frontend work.
    viewer: Actor,
    outsider: Actor,
}
async fn world() -> World {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url)
        .await
        .unwrap()
        .with_auto_publish_products(AutoPublish::Only(vec!["demo".into()]))
        .unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let actor = |user: &str| Actor {
        org: org.clone(),
        user: user.into(),
    };
    let (anika, sam, samira, jo, viewer, outsider) = (
        actor("anika"),
        actor("sam"),
        actor("samira"),
        actor("jo"),
        actor("viewer"),
        actor("outsider"),
    );
    for (who, services, review) in [
        (&anika, vec!["backend", "frontend"], true),
        (&sam, vec!["backend"], false),
        (&samira, vec!["backend"], false),
        (&jo, vec!["frontend"], false),
        (&viewer, vec!["backend"], false),
    ] {
        let services: Vec<String> = services.into_iter().map(String::from).collect();
        k.grant(who, "demo", &services, review).await.unwrap();
    }
    World {
        k,
        pool,
        anika,
        sam,
        samira,
        jo,
        viewer,
        outsider,
    }
}
fn request(title: &str, service: Option<&str>, files: &[&str]) -> ShareRequest {
    ShareRequest {
        service: service.map(String::from),
        title: title.into(),
        intent: format!("Goal of {title}"),
        rationale: format!("Because of {title}"),
        decisions: vec![format!("decision for {title}")],
        constraints: vec![],
        open_questions: vec![],
        files: files.iter().map(|f| f.to_string()).collect(),
    }
}
async fn share(
    w: &World,
    who: &Actor,
    service: Option<&str>,
    title: &str,
    files: &[&str],
) -> uuid::Uuid {
    w.k.share_session(who, "demo", request(title, service, files))
        .await
        .unwrap()
        .card_id
}
/// The standard cast of published work, oldest first.
async fn seed(w: &World) {
    share(
        w,
        &w.sam,
        None,
        "Retry with backoff",
        &["backend/payments/retry.rs", "backend/payments/gateway.rs"],
    )
    .await;
    share(
        w,
        &w.sam,
        None,
        "Rate limit middleware",
        &["backend/api/limiter.rs"],
    )
    .await;
    share(
        w,
        &w.anika,
        Some("frontend"),
        "Checkout form validation",
        &["frontend/checkout/form.tsx"],
    )
    .await;
    share(w, &w.jo, None, "Button styles", &["frontend/ui/button.css"]).await;
    share(
        w,
        &w.samira,
        None,
        "Audit logging",
        &["backend/audit/log.rs"],
    )
    .await;
    // A card whose files were observed by the collector, not reported.
    let observed: CardSubmission = serde_json::from_value(serde_json::json!({
        "schema_version":1,"idempotency_key":"observed-refund","policy_revision":1,"title":"Refund endpoint",
        "intent":"Let support refund","rationale":"Support needed it","decisions":[],"constraints":[],"open_questions":[],
        "service_ids":["backend"],"evidence":[{"kind":"file_observation","reference":"backend/payments/refund.rs"}],
        "lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap();
    let card = w.k.submit(&w.sam, "demo", observed).await.unwrap();
    w.k.publish(&w.anika, "demo", card.id).await.unwrap();
}
async fn ask(w: &World, who: &Actor, build: impl FnOnce(&mut TeamWorkQuery)) -> Value {
    let mut q = TeamWorkQuery::default();
    build(&mut q);
    w.k.team_work(who, "demo", q).await.unwrap()
}
fn titles(v: &Value) -> Vec<String> {
    v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap().to_string())
        .collect()
}
fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

async fn live(w: &World, who: &Actor, product: &str, services: &[&str], ended: bool) {
    w.k.record_activity(
        who,
        product,
        ActivityUpdate {
            session_id: "live".into(),
            services: strings(services),
            files: strings(&["backend/payments/retry.rs"]),
            intent: "Investigating payment retries".into(),
            ended,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn named_activity_only_work_resolves_and_keeps_useful_text_without_cards() {
    let w = world().await;
    live(&w, &w.sam, "demo", &["backend"], false).await;
    for name in [Some("SAM"), Some("sa"), None] {
        let found = ask(&w, &w.viewer, |q| {
            q.developer = name.map(String::from);
            q.paths = strings(&["backend/payments"]);
            q.service = Some("backend".into());
        })
        .await;
        assert_eq!(found["status"], "ok", "{found}");
        if name.is_some() {
            assert_eq!(found["resolved"]["developer"], "sam");
        }
        assert!(found["entries"].as_array().unwrap().is_empty());
        assert_eq!(found["active_now"].as_array().unwrap().len(), 1);
        assert_eq!(found["active_now"][0]["developer"], "sam");
        let text = found["text"].as_str().unwrap();
        assert!(text.contains("Active now: sam"), "{text}");
        assert!(text.contains("Investigating payment retries"));
        assert!(text.contains("backend/payments/retry.rs"));
        assert!(found.to_string().len() <= 2500 * 3);
    }
    for (path, service) in [("backend/pay", "backend"), ("backend", "frontend")] {
        let found = ask(&w, &w.viewer, |q| {
            q.developer = Some("sam".into());
            q.paths = strings(&[path]);
            q.service = Some(service.into());
        })
        .await;
        assert_eq!(found["status"], "no_matches");
        assert!(found["active_now"].as_array().unwrap().is_empty());
    }
    let cards: i64 = sqlx::query_scalar("SELECT count(*) FROM cards WHERE org=$1")
        .bind(&w.viewer.org)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(cards, 0);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn activity_names_merge_with_card_authors_without_duplicates_and_keep_ambiguity() {
    let w = world().await;
    share(&w, &w.sam, None, "Published retries", &[]).await;
    live(&w, &w.sam, "demo", &["backend"], false).await;
    live(&w, &w.samira, "demo", &["backend"], false).await;
    let exact = ask(&w, &w.viewer, |q| q.developer = Some("sam".into())).await;
    assert_eq!(exact["status"], "ok");
    assert_eq!(exact["entries"].as_array().unwrap().len(), 1);
    assert_eq!(exact["active_now"].as_array().unwrap().len(), 1);
    let ambiguous = ask(&w, &w.viewer, |q| q.developer = Some("sa".into())).await;
    assert_eq!(ambiguous["status"], "developer_ambiguous");
    assert_eq!(
        ambiguous["known_developers"],
        serde_json::json!(["sam", "samira"])
    );
    let unknown = ask(&w, &w.viewer, |q| q.developer = Some("missing".into())).await;
    assert_eq!(
        unknown["known_developers"],
        serde_json::json!(["sam", "samira"])
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn hidden_or_inactive_activity_never_reveals_names_or_results() {
    let w = world().await;
    live(&w, &w.jo, "demo", &["frontend"], false).await;
    live(&w, &w.sam, "demo", &["backend"], false).await;
    sqlx::query(
        "UPDATE activity SET expires_at=now()-interval '1 second' WHERE org=$1 AND user_id='sam'",
    )
    .bind(&w.viewer.org)
    .execute(&w.pool)
    .await
    .unwrap();
    live(&w, &w.samira, "demo", &["backend"], false).await;
    live(&w, &w.samira, "demo", &["backend"], true).await;
    live(&w, &w.anika, "demo", &["backend"], false).await;
    w.k.clear_activity(&w.anika, "demo").await.unwrap();
    w.k.grant(&w.outsider, "other", &strings(&["backend"]), false)
        .await
        .unwrap();
    live(&w, &w.outsider, "other", &["backend"], false).await;
    let other_org = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: "cross-org".into(),
    };
    w.k.grant(&other_org, "demo", &strings(&["backend"]), false)
        .await
        .unwrap();
    live(&w, &other_org, "demo", &["backend"], false).await;
    // Stored activity from an older collector is screened before names are discovered too.
    live(&w, &w.viewer, "demo", &["backend"], false).await;
    sqlx::query(
        "UPDATE activity SET files=ARRAY['/private/unsafe'] WHERE org=$1 AND user_id='viewer'",
    )
    .bind(&w.viewer.org)
    .execute(&w.pool)
    .await
    .unwrap();
    for name in [
        "missing",
        "jo",
        "sam",
        "samira",
        "anika",
        "outsider",
        "cross-org",
        "viewer",
    ] {
        let found = ask(&w, &w.viewer, |q| q.developer = Some(name.into())).await;
        assert_eq!(found["status"], "developer_not_found", "{name}: {found}");
        assert_eq!(found["known_developers"], serde_json::json!([]), "{found}");
        assert!(found["active_now"].as_array().unwrap().is_empty());
    }
    let found = ask(&w, &w.viewer, |_| {}).await;
    assert_eq!(found["status"], "no_matches");
    assert!(found["active_now"].as_array().unwrap().is_empty());
    // A device's narrower grant applies even when its owner can read frontend work.
    let device =
        w.k.device_register(
            &w.anika,
            wk_contracts::device::DeviceRegistration {
                id: uuid::Uuid::new_v4(),
                product: "demo".into(),
                services: strings(&["backend"]),
                public_key: "11".repeat(32),
                approval_hash: "22".repeat(32),
            },
        )
        .await
        .unwrap();
    let narrow =
        w.k.device_bound(&device, wk_knowledge::devices::DevicePermission::Read);
    let found = narrow
        .team_work(
            &w.anika,
            "demo",
            TeamWorkQuery {
                developer: Some("jo".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(found["status"], "developer_not_found");
    assert_eq!(found["known_developers"], serde_json::json!([]));
    assert!(found["active_now"].as_array().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn publication_is_separate_from_implementation_lifecycle_even_in_small_replies() {
    let w = world().await;
    let mut card = request("Published retry", None, &["backend/retry.rs"]);
    card.rationale = "Synthetic rationale. ".repeat(100);
    let shared = w.k.share_session(&w.sam, "demo", card).await.unwrap();
    assert!(shared.published);
    for budget in [256, 2500] {
        let found = ask(&w, &w.anika, |q| {
            q.developer = Some("sam".into());
            q.budget_tokens = Some(budget);
        })
        .await;
        let entry = &found["entries"][0];
        assert_eq!(entry["published"], true, "budget {budget}: {entry}");
        assert_eq!(entry["lifecycle"], "proposed");
        if budget == 256 {
            assert_eq!(entry["trimmed"], true);
        }
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn a_name_resolves_by_case_and_unique_prefix_and_failures_say_who_exists() {
    let w = world().await;
    seed(&w).await;
    let sam = ask(&w, &w.anika, |q| q.developer = Some("sam".into())).await;
    assert_eq!(
        (
            sam["status"].as_str(),
            sam["resolved"]["developer"].as_str()
        ),
        (Some("ok"), Some("sam"))
    );
    // Newest first, and only sam's: the retry and limiter shares and the observed refund card.
    assert_eq!(
        titles(&sam),
        strings(&[
            "Refund endpoint",
            "Rate limit middleware",
            "Retry with backoff"
        ])
    );
    assert!(sam["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["developer"] == "sam"));
    // Exact beats prefix (sam and samira both start with "sam"), case does not matter.
    for spelled in ["SAM", "Sam", " sam "] {
        let again = ask(&w, &w.anika, |q| q.developer = Some(spelled.into())).await;
        assert_eq!(again["resolved"]["developer"], "sam", "{spelled:?}");
    }
    // A unique prefix is enough; an ambiguous one lists the candidates.
    assert_eq!(
        ask(&w, &w.anika, |q| q.developer = Some("an".into())).await["resolved"]["developer"],
        "anika"
    );
    let ambiguous = ask(&w, &w.anika, |q| q.developer = Some("sa".into())).await;
    assert_eq!(ambiguous["status"], "developer_ambiguous");
    assert_eq!(
        ambiguous["known_developers"],
        serde_json::json!(["sam", "samira"])
    );
    assert!(ambiguous["entries"].as_array().unwrap().is_empty());
    // An unknown name returns a normal result naming everyone who has shared work, so an agent can retry.
    let unknown = ask(&w, &w.anika, |q| q.developer = Some("bob".into())).await;
    assert_eq!(unknown["status"], "developer_not_found");
    assert_eq!(
        unknown["known_developers"],
        serde_json::json!(["anika", "jo", "sam", "samira"])
    );
    assert!(unknown["text"]
        .as_str()
        .unwrap()
        .contains("Known developers"));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn files_and_modules_match_on_directory_boundaries_from_both_sources() {
    let w = world().await;
    seed(&w).await;
    let by = |paths: &[&str]| {
        let paths = strings(paths);
        let w = &w;
        async move { ask(w, &w.anika, |q| q.paths = paths).await }
    };
    // A module matches everything under it, reported or observed, newest first.
    let module = by(&["backend/payments"]).await;
    assert_eq!(
        titles(&module),
        strings(&["Refund endpoint", "Retry with backoff"])
    );
    // Normalization: leading ./ and a trailing slash change nothing.
    assert_eq!(titles(&by(&["./backend/payments/"]).await), titles(&module));
    // A name that is only a prefix of a directory name does not match it.
    assert_eq!(by(&["backend/pay"]).await["status"], "no_matches");
    assert_eq!(
        by(&["backend/payments/retry"]).await["status"],
        "no_matches"
    );
    // One exact file.
    let one = by(&["backend/payments/retry.rs"]).await;
    assert_eq!(titles(&one), strings(&["Retry with backoff"]));
    assert_eq!(
        one["entries"][0]["matched_files"],
        serde_json::json!(["backend/payments/retry.rs"])
    );
    assert_eq!(one["entries"][0]["matched_by"], serde_json::json!(["path"]));
    // Reported and observed files are labelled apart.
    let refund = &module["entries"][0];
    assert_eq!(
        refund["files_observed"],
        serde_json::json!(["backend/payments/refund.rs"])
    );
    assert_eq!(refund["files_reported"], serde_json::json!([]));
    let retry = &module["entries"][1];
    assert_eq!(
        retry["files_reported"],
        serde_json::json!(["backend/payments/gateway.rs", "backend/payments/retry.rs"])
    );
    // Several paths are a union.
    assert_eq!(
        titles(&by(&["backend/api", "frontend/checkout"]).await),
        strings(&["Checkout form validation", "Rate limit middleware"])
    );
    // A developer and a module together intersect.
    let both = ask(&w, &w.anika, |q| {
        q.developer = Some("sam".into());
        q.paths = strings(&["backend/api"]);
    })
    .await;
    assert_eq!(titles(&both), strings(&["Rate limit middleware"]));
    assert_eq!(
        both["entries"][0]["matched_by"],
        serde_json::json!(["developer", "path"])
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn time_service_and_keywords_narrow_and_rank() {
    let w = world().await;
    seed(&w).await;
    sqlx::query("UPDATE cards SET created_at=now()-interval '10 days' WHERE org=$1 AND body->>'title'='Retry with backoff'")
        .bind(&w.anika.org)
        .execute(&w.pool)
        .await
        .unwrap();
    let recent = ask(&w, &w.anika, |q| {
        q.developer = Some("sam".into());
        q.since_days = Some(7);
    })
    .await;
    assert_eq!(
        titles(&recent),
        strings(&["Refund endpoint", "Rate limit middleware"])
    );
    let month = ask(&w, &w.anika, |q| {
        q.developer = Some("sam".into());
        q.since_days = Some(30);
    })
    .await;
    assert_eq!(
        titles(&month),
        strings(&[
            "Refund endpoint",
            "Rate limit middleware",
            "Retry with backoff"
        ])
    );
    assert_eq!(
        titles(&ask(&w, &w.anika, |q| q.service = Some("frontend".into())).await),
        strings(&["Button styles", "Checkout form validation"])
    );
    // Keywords rank inside the filtered set, in natural phrasing.
    let ranked = ask(&w, &w.anika, |q| {
        q.developer = Some("sam".into());
        q.query = Some("what about rate limiting".into());
    })
    .await;
    assert_eq!(titles(&ranked)[0], "Rate limit middleware");
    assert_eq!(
        ranked["entries"][0]["matched_by"],
        serde_json::json!(["developer", "text"])
    );
    let none = ask(&w, &w.anika, |q| q.query = Some("zzzunmatchedzzz".into())).await;
    assert_eq!(none["status"], "no_matches");
    assert!(none["text"].as_str().unwrap().starts_with("No shared work"));
    // With nothing asked, it is simply the recent work of everyone, newest first.
    assert_eq!(
        titles(&ask(&w, &w.anika, |_| {}).await)[0],
        "Refund endpoint"
    );
}

/// Edit-time overlap notices for `files`, as the before-edit hook asks for them.
async fn notices(w: &World, who: &Actor, files: &[&str]) -> Vec<Notice> {
    let q = OverlapQuery {
        files: strings(files),
        query: None,
        limit: 8,
    };
    w.k.overlaps(who, "demo", q).await.unwrap()
}
fn order(notices: &[Notice]) -> Vec<(&'static str, &str)> {
    notices
        .iter()
        .map(|n| (n.matched_by, n.summary.as_str()))
        .collect()
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn files_a_share_reports_warn_teammates_before_they_edit() {
    let w = world().await;
    seed(&w).await;
    // The same file: a changed notice worded as reported, not observed. It comes first, ahead of
    // the observed refund card in the same folder.
    let exact = notices(&w, &w.anika, &["backend/payments/retry.rs"]).await;
    assert_eq!(
        order(&exact),
        [("file", "Retry with backoff"), ("folder", "Refund endpoint")]
    );
    assert_eq!((exact[0].kind, exact[0].developer.as_str()), ("changed", "sam"));
    assert_eq!(exact[0].matched_files, strings(&["backend/payments/retry.rs"]));
    assert!(exact[0]
        .text
        .starts_with("sam shared work on `backend/payments/retry.rs` just now"));
    // Another file in the same folder: weaker folder notices, reported and observed alike.
    let near = notices(&w, &w.anika, &["backend/payments/ledger.rs"]).await;
    assert_eq!(
        order(&near),
        [("folder", "Refund endpoint"), ("folder", "Retry with backoff")]
    );
    assert!(near[1].text.starts_with(
        "sam shared work on files in the same folder \
         (`backend/payments/gateway.rs`, `backend/payments/retry.rs`)"
    ));
    // Collector-observed files keep the stronger wording.
    assert!(near[0]
        .text
        .starts_with("sam changed files in the same folder (`backend/payments/refund.rs`)"));
    let observed = notices(&w, &w.anika, &["backend/payments/refund.rs"]).await;
    assert_eq!(order(&observed)[0], ("file", "Refund endpoint"));
    assert!(observed[0]
        .text
        .starts_with("sam changed `backend/payments/refund.rs`"));
    // Never your own work, never outside your services, never withdrawn work.
    assert!(notices(&w, &w.sam, &["backend/api/limiter.rs"]).await.is_empty());
    assert!(notices(&w, &w.viewer, &["frontend/checkout/form.tsx"])
        .await
        .is_empty());
    assert_eq!(
        order(&notices(&w, &w.viewer, &["backend/api/limiter.rs"]).await),
        [("file", "Rate limit middleware")]
    );
    let gone = share(&w, &w.samira, None, "Old audit", &["backend/audit/export.rs"]).await;
    w.k.withdraw(&w.samira, "demo", gone, 1).await.unwrap();
    assert_eq!(
        order(&notices(&w, &w.anika, &["backend/audit/export.rs"]).await),
        [("folder", "Audit logging")]
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn only_visible_published_work_is_ever_returned() {
    let w = world().await;
    seed(&w).await;
    // Withdrawn work disappears; a draft was never visible.
    let old = share(
        &w,
        &w.sam,
        None,
        "Old retry approach",
        &["backend/payments/retry.rs"],
    )
    .await;
    w.k.withdraw(&w.sam, "demo", old, 1).await.unwrap();
    let draft: CardSubmission = serde_json::from_value(serde_json::json!({
        "schema_version":1,"idempotency_key":"draft-only","policy_revision":1,"title":"Unpublished draft",
        "intent":"x","rationale":"y","decisions":[],"constraints":[],"open_questions":[],"service_ids":["backend"],
        "evidence":[{"kind":"file_observation","reference":"backend/payments/retry.rs"}],
        "lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap();
    w.k.submit(&w.sam, "demo", draft).await.unwrap();
    let seen = ask(&w, &w.anika, |q| {
        q.paths = strings(&["backend/payments/retry.rs"])
    })
    .await;
    assert_eq!(titles(&seen), strings(&["Retry with backoff"]));
    // A backend-only developer sees no frontend work, and cannot even find the frontend developer.
    let viewer = ask(&w, &w.viewer, |_| {}).await;
    assert!(titles(&viewer)
        .iter()
        .all(|t| t != "Checkout form validation" && t != "Button styles"));
    assert_eq!(
        ask(&w, &w.viewer, |q| q.developer = Some("jo".into())).await["status"],
        "developer_not_found"
    );
    assert_eq!(
        ask(&w, &w.viewer, |q| q.paths = strings(&["frontend"])).await["status"],
        "no_matches"
    );
    // Someone outside the product is refused outright.
    let q = TeamWorkQuery {
        developer: Some("sam".into()),
        ..Default::default()
    };
    assert_eq!(
        w.k.team_work(&w.outsider, "demo", q).await.unwrap_err(),
        Error::Denied
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn bad_input_is_refused_and_the_reply_stays_inside_its_size_cap() {
    let w = world().await;
    seed(&w).await;
    let eleven: Vec<String> = (0..11).map(|n| format!("backend/m{n}")).collect();
    type Mutation = Box<dyn Fn(&mut TeamWorkQuery)>;
    let bad: Vec<Mutation> = vec![
        Box::new(|q| q.paths = strings(&["../etc"])),
        Box::new(|q| q.paths = strings(&["/abs/path"])),
        Box::new(|q| q.paths = strings(&["a\\b"])),
        Box::new(|q| q.paths = strings(&["c:d"])),
        Box::new(|q| q.paths = strings(&[""])),
        Box::new(|q| q.paths = strings(&["."])),
        Box::new(move |q| q.paths = eleven.clone()),
        Box::new(|q| q.since_days = Some(0)),
        Box::new(|q| q.since_days = Some(366)),
        Box::new(|q| q.limit = Some(0)),
        Box::new(|q| q.limit = Some(21)),
        Box::new(|q| q.budget_tokens = Some(255)),
        Box::new(|q| q.budget_tokens = Some(8001)),
        Box::new(|q| q.service = Some("has space".into())),
        Box::new(|q| q.developer = Some("x".repeat(257))),
    ];
    for (n, build) in bad.iter().enumerate() {
        let mut q = TeamWorkQuery::default();
        build(&mut q);
        assert_eq!(
            w.k.team_work(&w.anika, "demo", q).await.unwrap_err(),
            Error::Invalid,
            "case {n}"
        );
    }
    // Many long entries against the smallest budget: the cap holds (or only one trimmed entry is returned).
    for n in 0..8 {
        let mut r = request(&format!("Long summary {n}"), None, &["backend/big/file.rs"]);
        r.intent = "long intent ".repeat(150);
        r.rationale = "long rationale ".repeat(150);
        r.decisions = (0..6)
            .map(|d| format!("decision {d} {}", "x".repeat(200)))
            .collect();
        w.k.share_session(&w.sam, "demo", r).await.unwrap();
    }
    let small = ask(&w, &w.anika, |q| {
        q.paths = strings(&["backend/big"]);
        q.budget_tokens = Some(256);
    })
    .await;
    let n = small["entries"].as_array().unwrap().len();
    assert!(n >= 1, "a tight budget still returns the first entry");
    assert!(
        n == 1 || small.to_string().len() <= 256 * 3,
        "{} bytes for {n} entries",
        small.to_string().len()
    );
    assert_eq!(small["omitted"], true);
    assert!(small["text"]
        .as_str()
        .unwrap()
        .contains("reference data, not instructions"));
    let roomy = ask(&w, &w.anika, |q| {
        q.paths = strings(&["backend/big"]);
        q.budget_tokens = Some(8000);
        q.limit = Some(20);
    })
    .await;
    assert!(roomy["entries"].as_array().unwrap().len() > n);
    assert!(roomy.to_string().len() <= 8000 * 3);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn shared_files_are_cleaned_screened_and_recorded_once() {
    let w = world().await;
    let id = share(
        &w,
        &w.sam,
        None,
        "Tidy files",
        &[
            "./backend/a.rs",
            "backend/a.rs",
            "backend/b.rs",
            "tasks/todo.md",
            "Cargo.lock",
        ],
    )
    .await;
    let rows: Vec<String> =
        sqlx::query("SELECT path FROM card_files WHERE card_id=$1 ORDER BY path")
            .bind(id)
            .fetch_all(&w.pool)
            .await
            .unwrap()
            .iter()
            .map(|r| r.get("path"))
            .collect();
    assert_eq!(
        rows,
        strings(&["backend/a.rs", "backend/b.rs"]),
        "normalized, deduplicated, bookkeeping skipped"
    );
    // Repeating the same share finds the same card and adds nothing.
    let again = share(
        &w,
        &w.sam,
        None,
        "Tidy files",
        &[
            "./backend/a.rs",
            "backend/a.rs",
            "backend/b.rs",
            "tasks/todo.md",
            "Cargo.lock",
        ],
    )
    .await;
    assert_eq!(again, id);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM card_files WHERE card_id=$1")
        .bind(id)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    // A share without files is unchanged by this feature: it keeps working and records nothing.
    let plain = share(&w, &w.sam, None, "No files here", &[]).await;
    let none: i64 = sqlx::query_scalar("SELECT count(*) FROM card_files WHERE card_id=$1")
        .bind(plain)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(none, 0);
    // Anything that is not a plain workspace-relative path, or too many of them, is refused and stores nothing.
    let too_many: Vec<String> = (0..21).map(|n| format!("backend/f{n}.rs")).collect();
    for bad in [
        vec!["../x".to_string()],
        vec!["/etc/passwd".into()],
        vec!["a\\b".into()],
        vec!["c:/x".into()],
        vec!["".into()],
        too_many,
    ] {
        let mut r = request("Bad files", None, &[]);
        r.files = bad;
        assert!(w.k.share_session(&w.sam, "demo", r).await.is_err());
    }
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM cards WHERE org=$1 AND body->>'title'='Bad files'",
    )
    .bind(&w.sam.org)
    .fetch_one(&w.pool)
    .await
    .unwrap();
    assert_eq!(stored, 0);
}
