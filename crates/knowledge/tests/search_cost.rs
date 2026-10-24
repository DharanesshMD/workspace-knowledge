//! Lexical search must not re-tokenize every candidate card on each request.
//!
//! Row-level security stops PostgreSQL using a GIN index for the (non-leakproof)
//! `@@` operator under the restricted application role, so the search vector has
//! to be stored with the card. The synthetic corpus below makes the old
//! per-request `to_tsvector` cost obvious without depending on absolute speed.
use std::time::{Duration, Instant};
use wk_contracts::CardSubmission;
use wk_knowledge::{Actor, Knowledge};

const CARDS: i64 = 600;
const PRODUCT: &str = "product";
// Role grants and the timing budgets both assume the tests do not overlap.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn card(key: &str) -> CardSubmission {
    serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Tenant rate limiter","intent":"Prevent overload","rationale":"Shared middleware","decisions":["tenant limits"],"constraints":["burst budget"],"open_questions":["window sizing"],"service_ids":["backend"],"evidence":[],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()
}

async fn restricted_runtime(url: &str) -> (sqlx::PgPool, sqlx::PgPool, Knowledge) {
    let owner = sqlx::PgPool::connect(url).await.unwrap();
    let role = format!("wk_search_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(&owner)
        .await
        .unwrap();
    let grants =
        include_str!("../../../deploy/scripts/application-role.sql").replace("wk_app", &role);
    sqlx::raw_sql(&grants).execute(&owner).await.unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |conn, _| {
            let role = role.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await
        .unwrap();
    (owner, pool.clone(), Knowledge::from_pool(pool))
}

async fn corpus() -> (
    sqlx::PgPool,
    sqlx::PgPool,
    Knowledge,
    Actor,
    Actor,
    uuid::Uuid,
) {
    let url = std::env::var("WK_TEST_DATABASE_URL")
        .expect("set WK_TEST_DATABASE_URL to a disposable /wk_test database");
    assert!(url.split('?').next().unwrap().ends_with("/wk_test"));
    let admin = Knowledge::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let author = Actor {
        org: org.clone(),
        user: "author".into(),
    };
    let reader = Actor {
        org: org.clone(),
        user: "reader".into(),
    };
    admin
        .grant(&author, PRODUCT, &["backend".into()], true)
        .await
        .unwrap();
    admin
        .grant(&reader, PRODUCT, &["backend".into()], false)
        .await
        .unwrap();
    let template = admin
        .submit(&author, PRODUCT, card("template"))
        .await
        .unwrap();
    admin.publish(&author, PRODUCT, template.id).await.unwrap();
    let (owner, pool, runtime) = restricted_runtime(&url).await;
    // About 10 KiB of varied prose per card, as the capacity pilot uses.
    sqlx::query("INSERT INTO cards(id,org,product,author,idempotency_key,services,body,published) SELECT gen_random_uuid(),org,product,author,'bulk-'||g,services,jsonb_set(body,'{rationale}',to_jsonb((SELECT string_agg('w'||(g*7919+i*i*31+i)%700,' ') FROM generate_series(1,1600) i))),TRUE FROM cards, generate_series(1,$2::int) g WHERE id=$1")
        .bind(template.id).bind(CARDS as i32).execute(&owner).await.unwrap();
    (owner, pool, runtime, author, reader, template.id)
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn rare_term_search_over_large_cards_is_not_proportional_to_tokenizing_them() {
    let _serial = SERIAL.lock().await;
    let (owner, _pool, runtime, _author, reader, template) = corpus().await;
    // Warm the connection and plan, then time a term that matches nothing so
    // every candidate must be examined.
    runtime
        .search(&reader, PRODUCT, "middleware", 8)
        .await
        .unwrap();
    let began = Instant::now();
    assert!(runtime
        .search(&reader, PRODUCT, "absentlexeme", 8)
        .await
        .unwrap()
        .is_empty());
    let elapsed = began.elapsed();
    eprintln!("absent-term search over {CARDS} cards: {elapsed:?}");
    assert!(
        elapsed < Duration::from_millis(250),
        "searching {CARDS} large cards for an absent term took {elapsed:?}"
    );
    // A natural-language query ORs its words but still only matches the stored vector.
    let began = Instant::now();
    assert!(runtime
        .search(&reader, PRODUCT, "absentlexeme and absentother words", 8)
        .await
        .unwrap()
        .is_empty());
    let elapsed = began.elapsed();
    eprintln!("absent OR search over {CARDS} cards: {elapsed:?}");
    assert!(
        elapsed < Duration::from_millis(250),
        "an OR query over {CARDS} large cards took {elapsed:?}"
    );
    // The stored vector must follow later edits to every searchable field.
    for (field, term) in [
        ("title", "zebratitle"),
        ("intent", "zebraintent"),
        ("rationale", "zebrarationale"),
        ("decisions", "zebradecision"),
        ("constraints", "zebraconstraint"),
        ("open_questions", "zebraquestion"),
    ] {
        let value = if matches!(field, "title" | "intent" | "rationale") {
            serde_json::json!(term)
        } else {
            serde_json::json!([term])
        };
        sqlx::query("UPDATE cards SET body=jsonb_set(body,ARRAY[$2],$3) WHERE id=$1")
            .bind(template)
            .bind(field)
            .bind(value)
            .execute(&owner)
            .await
            .unwrap();
        let found = runtime.search(&reader, PRODUCT, term, 8).await.unwrap();
        assert_eq!(found.len(), 1, "{field} edits must be searchable");
        assert_eq!(found[0].id, template);
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn similar_work_lookup_over_large_cards_is_not_proportional_to_tokenizing_them() {
    let _serial = SERIAL.lock().await;
    let (_owner, _pool, runtime, author, reader, _template) = corpus().await;
    let _ = ask(&runtime, &author).await.unwrap();
    let began = Instant::now();
    assert!(ask(&runtime, &reader).await.unwrap().is_empty());
    let elapsed = began.elapsed();
    assert!(
        elapsed < Duration::from_millis(250),
        "similar-work lookup over {CARDS} large cards took {elapsed:?}"
    );
}

async fn ask(
    runtime: &Knowledge,
    user: &Actor,
) -> Result<Vec<wk_knowledge::activity::Notice>, wk_knowledge::Error> {
    runtime
        .overlaps(
            user,
            PRODUCT,
            wk_knowledge::activity::OverlapQuery {
                files: vec![],
                query: Some("absentlexeme".into()),
                limit: 8,
            },
        )
        .await
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn publishing_looks_up_a_cards_graph_nodes_by_index() {
    let _serial = SERIAL.lock().await;
    let (owner, pool, _runtime, author, _reader, template) = corpus().await;
    // Three nodes per card, as publication writes; without a supporting index
    // every publication scanned the whole table (87 ms at 24,000 nodes).
    sqlx::query("INSERT INTO graph_nodes(id,org,product,card_id,kind,label,provenance) SELECT gen_random_uuid(),org,product,id,k,'n','{}'::jsonb FROM cards, unnest(ARRAY['product','service','feature']) k WHERE org=$1")
        .bind(&author.org).execute(&owner).await.unwrap();
    sqlx::query("ANALYZE graph_nodes")
        .execute(&owner)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('wk.org',$1,true),set_config('wk.user',$2,true)")
        .bind(&author.org)
        .bind(&author.user)
        .execute(&mut *tx)
        .await
        .unwrap();
    let plan: serde_json::Value = sqlx::query_scalar("EXPLAIN (FORMAT JSON) SELECT count(*) FROM graph_nodes WHERE org=$1 AND product=$2 AND card_id=$3")
        .bind(&author.org).bind(PRODUCT).bind(template).fetch_one(&mut *tx).await.unwrap();
    assert!(
        !scans(&plan, "graph_nodes"),
        "graph node lookup by card must use an index: {plan}"
    );
}

fn scans(plan: &serde_json::Value, relation: &str) -> bool {
    match plan {
        serde_json::Value::Object(node) => {
            (node.get("Node Type").and_then(|t| t.as_str()) == Some("Seq Scan")
                && node.get("Relation Name").and_then(|r| r.as_str()) == Some(relation))
                || node.values().any(|v| scans(v, relation))
        }
        serde_json::Value::Array(items) => items.iter().any(|v| scans(v, relation)),
        _ => false,
    }
}
