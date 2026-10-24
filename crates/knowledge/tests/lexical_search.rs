//! Keyword retrieval takes any word of a natural-language query, ranks cards by how many of
//! those words they contain, and keeps every existing visibility rule.
use wk_contracts::CardSubmission;
use wk_knowledge::{Actor, Error, Knowledge};

const P: &str = "product";

fn card(
    key: &str,
    title: &str,
    intent: &str,
    rationale: &str,
    services: &[&str],
) -> CardSubmission {
    serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":title,"intent":intent,"rationale":rationale,"decisions":[],"constraints":[],"open_questions":[],"service_ids":services,"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()
}

struct World {
    k: Knowledge,
    pool: sqlx::PgPool,
    author: Actor,
    reader: Actor,
}

async fn setup() -> World {
    let url = std::env::var("WK_TEST_DATABASE_URL")
        .expect("set WK_TEST_DATABASE_URL to a disposable /wk_test database");
    assert!(
        url.split('?').next().unwrap().ends_with("/wk_test"),
        "refusing non-test database"
    );
    let k = Knowledge::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let author = Actor {
        org: org.clone(),
        user: "author".into(),
    };
    let reader = Actor {
        org,
        user: "reader".into(),
    };
    k.grant(&author, P, &["backend".into(), "frontend".into()], true)
        .await
        .unwrap();
    k.grant(&reader, P, &["backend".into()], false)
        .await
        .unwrap();
    World {
        k,
        pool,
        author,
        reader,
    }
}

impl World {
    async fn publish(&self, c: CardSubmission) -> uuid::Uuid {
        let card = self.k.submit(&self.author, P, c).await.unwrap();
        self.k.publish(&self.author, P, card.id).await.unwrap();
        card.id
    }
    async fn ids(&self, query: &str, limit: u32) -> Vec<uuid::Uuid> {
        self.k
            .search(&self.reader, P, query, limit)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect()
    }
}

const TITLE: &str = "Rate limiting design: token bucket, 100 req/min per user";

async fn world_with_rate_limit_card() -> (World, uuid::Uuid) {
    let w = setup().await;
    let id = w
        .publish(card(
            "rate",
            TITLE,
            "Protect shared capacity",
            "Each caller draws from its own allowance",
            &["backend"],
        ))
        .await;
    (w, id)
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn natural_phrasings_find_the_card_that_matches_only_some_words() {
    let (w, id) = world_with_rate_limit_card().await;
    w.publish(card(
        "other",
        "Kubernetes ingress certificates",
        "Rotate edge certificates",
        "Automation renews them",
        &["backend"],
    ))
    .await;
    for query in [
        "rate limiting",
        "token bucket",
        "rate limiting backend API",
        "I'm about to add rate limiting to the backend API",
        "RATE-LIMITING, backend/API?",
        "how should we throttle requests per user",
    ] {
        assert_eq!(w.ids(query, 10).await, vec![id], "{query}");
    }
    for query in [
        "database migration rollback",
        "the and of",
        "!!! & | ::",
        "'quoted' \"also\"",
    ] {
        assert!(w.ids(query, 10).await.is_empty(), "{query}");
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn cards_matching_more_of_the_terms_rank_first() {
    let w = setup().await;
    // Created in the order least to most relevant, so rank (not insertion or id) decides.
    let one = w
        .publish(card(
            "one",
            "Zephyr cache notes",
            "Warm entries early",
            "Nothing else applies",
            &["backend"],
        ))
        .await;
    let repeated = w
        .publish(card(
            "repeated",
            "Zephyr zephyr zephyr",
            "Zephyr zephyr",
            "Zephyr appears often but never alongside the other words",
            &["backend"],
        ))
        .await;
    let two = w
        .publish(card(
            "two",
            "Zephyr cache eviction",
            "Bound memory",
            "Least recently used",
            &["backend"],
        ))
        .await;
    let three = w
        .publish(card(
            "three",
            "Zephyr cache eviction policy",
            "Bound memory",
            "Least recently used",
            &["backend"],
        ))
        .await;
    // Four words, three, two, then a single word repeated: more distinct words beat repetition.
    assert_eq!(
        w.ids("zephyr cache eviction policy", 10).await,
        vec![three, two, one, repeated]
    );
    // The limit applies after ranking, so the best cards survive it.
    assert_eq!(
        w.ids("zephyr cache eviction policy", 2).await,
        vec![three, two]
    );
    assert_eq!(w.ids("zephyr cache eviction policy", 1).await, vec![three]);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn stopword_only_queries_keep_the_original_behaviour() {
    let w = setup().await;
    let id = w
        .publish(card(
            "the",
            "Guard the gateway",
            "Keep the edge simple",
            "The gateway owns the policy",
            &["backend"],
        ))
        .await;
    // No usable word remains, so the original text goes to plainto_tsquery as before.
    assert_eq!(w.ids("the", 10).await, vec![id]);
    assert_eq!(w.ids("The gateway", 10).await, vec![id]);
    assert!(w.ids("the nonexistentword", 10).await.is_empty());
    assert!(w.ids("of or", 10).await.is_empty());
    for blank in ["", "   ", "\n\t"] {
        assert_eq!(
            w.k.search(&w.reader, P, blank, 10).await.unwrap_err(),
            Error::Invalid
        );
    }
    assert_eq!(
        w.k.search(&w.reader, P, "gateway", 0).await.unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        w.k.search(&w.reader, P, "gateway", 21).await.unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        w.k.search(&w.reader, P, &"gateway ".repeat(200), 10)
            .await
            .unwrap_err(),
        Error::Invalid
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn operators_and_punctuation_in_a_query_are_harmless() {
    let (w, id) = world_with_rate_limit_card().await;
    // The words still match, whatever surrounds them.
    for query in [
        "bucket & zzzunknown",
        "bucket | zzzunknown",
        "!bucket",
        "!(bucket)",
        "bucket <-> zzzunknown",
        "bucket:A",
        "'bucket'",
        "\"bucket\" \\ ;",
        "x'); DROP TABLE cards;-- bucket",
        "bucket\u{0301} rocket \u{1f680} \u{65e5}\u{672c}\u{8a9e}",
        "caf\u{e9} na\u{ef}ve bucket",
    ] {
        assert_eq!(w.ids(query, 10).await, vec![id], "{query:?}");
    }
    // Operators do not act: no prefix match, no negation, no phrase, no wildcard.
    for query in ["buck:*", "buck*", "bucke:B", "b_cket", "tok:*"] {
        assert!(w.ids(query, 10).await.is_empty(), "{query:?}");
    }
    // Queries made only of syntax or hostile text never error and never match.
    for query in [
        "&",
        "|",
        "!",
        ":",
        "()",
        "'",
        "''",
        "\\",
        "&&&|||!!!",
        "<->",
        "<2>",
        "*:*",
        "\u{0301}",
        "\u{1f680}",
        "'; DROP TABLE cards; --",
        "a & !b | (c <-> d)",
        "\u{bd} \u{2460}",
    ] {
        let found = w.k.search(&w.reader, P, query, 10).await;
        assert!(found.is_ok(), "{query:?}: {found:?}");
        assert!(found.unwrap().is_empty(), "{query:?}");
    }
    // The table is still there and still searchable.
    assert_eq!(w.ids("rate limiting", 10).await, vec![id]);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn long_queries_use_the_first_twelve_words_and_never_error() {
    let (w, id) = world_with_rate_limit_card().await;
    let filler = (1..=12)
        .map(|i| format!("filler{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    // Words past the twelfth are ignored, so the match below is cut off.
    assert!(w.ids(&format!("{filler} bucket"), 10).await.is_empty());
    assert_eq!(w.ids(&format!("bucket {filler}"), 10).await, vec![id]);
    let huge = format!("bucket {}", "x".repeat(900));
    assert_eq!(w.ids(&huge, 10).await, vec![id]);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn broader_matching_keeps_every_visibility_rule() {
    let w = setup().await;
    let query = "token bucket throttling";
    let visible = w
        .publish(card(
            "visible",
            "Token bucket throttling",
            "Cap bursts",
            "Backend only",
            &["backend"],
        ))
        .await;
    // Not published.
    w.k.submit(
        &w.author,
        P,
        card(
            "draft",
            "Token bucket throttling draft",
            "Cap bursts",
            "Unreviewed",
            &["backend"],
        ),
    )
    .await
    .unwrap();
    // Outside the reader's services.
    w.publish(card(
        "frontend",
        "Token bucket throttling in the browser",
        "Cap bursts",
        "Frontend only",
        &["frontend"],
    ))
    .await;
    w.publish(card(
        "both",
        "Token bucket throttling everywhere",
        "Cap bursts",
        "Needs both services",
        &["backend", "frontend"],
    ))
    .await;
    // Withdrawn, superseded and deprecated.
    let withdrawn = w
        .publish(card(
            "withdrawn",
            "Token bucket throttling withdrawn",
            "Cap bursts",
            "Gone",
            &["backend"],
        ))
        .await;
    w.k.withdraw(&w.author, P, withdrawn, 1).await.unwrap();
    for (key, lifecycle) in [("superseded", "superseded"), ("deprecated", "deprecated")] {
        let id = w
            .publish(card(
                key,
                "Token bucket throttling retired",
                "Cap bursts",
                "Retired",
                &["backend"],
            ))
            .await;
        sqlx::query(
            "UPDATE cards SET body=jsonb_set(body,'{lifecycle}',to_jsonb($2::text)) WHERE id=$1",
        )
        .bind(id)
        .bind(lifecycle)
        .execute(&w.pool)
        .await
        .unwrap();
    }
    // Another organization with a stronger match.
    let other = setup().await;
    other
        .publish(card(
            "other-org",
            "Token bucket throttling elsewhere",
            "Cap bursts",
            "Not yours",
            &["backend"],
        ))
        .await;
    assert_eq!(w.ids(query, 10).await, vec![visible]);
    assert_eq!(w.ids("bucket", 10).await, vec![visible]);
    // The author sees both of their own published backend cards plus the shared one.
    let author_view = w.k.search(&w.author, P, query, 20).await.unwrap().len();
    assert_eq!(author_view, 3);
    // A revoked member finds nothing.
    w.k.revoke(&w.reader, P).await.unwrap();
    assert!(w.ids(query, 10).await.is_empty());
}
