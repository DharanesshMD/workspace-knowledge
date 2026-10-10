use wk_contracts::CardSubmission;
use wk_knowledge::{
    activity::ActivityUpdate,
    personal::{PersonalInput, PersonalProvenance},
    team::TeamQuery,
    Actor, Error, Knowledge,
};
fn body(key: &str, services: &[&str]) -> CardSubmission {
    serde_json::from_value(serde_json::json!({"schema_version":1,"idempotency_key":key,"policy_revision":1,"title":"Tenant limiter","intent":"Bound work","rationale":"Shared middleware","decisions":[],"constraints":[],"open_questions":[],"service_ids":services,"evidence":[{"kind":"file_observation","reference":"backend/limiter.rs"}],"lifecycle":"proposed","capture_completeness":"complete","base_revision":null})).unwrap()
}
async fn setup() -> (Knowledge, Actor, Actor, Actor, sqlx::PgPool) {
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    assert!(url.ends_with("/wk_test"));
    let k = Knowledge::connect(&url).await.unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let a = Actor {
        org: org.clone(),
        user: "alice".into(),
    };
    let b = Actor {
        org: org.clone(),
        user: "bob".into(),
    };
    let r = Actor {
        org,
        user: "reader".into(),
    };
    for who in [&a, &b] {
        k.grant(who, "demo", &["backend".into(), "frontend".into()], true)
            .await
            .unwrap();
    }
    k.grant(&r, "demo", &["backend".into()], false)
        .await
        .unwrap();
    (k, a, b, r, pool)
}
fn activity(id: &str, services: &[&str], ended: bool) -> ActivityUpdate {
    ActivityUpdate {
        session_id: id.into(),
        services: services.iter().map(|s| s.to_string()).collect(),
        files: vec!["backend/limiter.rs".into()],
        intent: "Review bounded work".into(),
        ended,
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn team_feed_and_activity_reveal_only_published_current_scope() {
    let (k, a, b, r, pool) = setup().await;
    let draft = k
        .submit(&a, "demo", body("draft", &["backend"]))
        .await
        .unwrap();
    let mut private = body("private", &["backend"]);
    private.evidence.clear();
    k.personal_put(
        &a,
        "demo",
        PersonalInput {
            body: private,
            provenance: PersonalProvenance::Manual,
        },
    )
    .await
    .unwrap();
    assert!(k
        .team_context(&b, "demo", TeamQuery::default())
        .await
        .unwrap()
        .is_empty());
    k.publish(&b, "demo", draft.id).await.unwrap();
    let mixed = k
        .submit(&b, "demo", body("mixed", &["backend", "frontend"]))
        .await
        .unwrap();
    k.publish(&b, "demo", mixed.id).await.unwrap();
    let feed = k
        .team_context(&r, "demo", TeamQuery::default())
        .await
        .unwrap();
    assert_eq!(feed.len(), 1);
    assert_eq!(feed[0].author, "alice");
    assert_eq!(feed[0].card.id, draft.id);
    assert!(k
        .team_context(
            &r,
            "demo",
            TeamQuery {
                author: Some("bob".into()),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .is_empty());
    assert!(k
        .team_context(
            &r,
            "demo",
            TeamQuery {
                before: Some(mixed.id),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .is_empty());
    let outsider = Actor {
        org: uuid::Uuid::new_v4().to_string(),
        user: a.user.clone(),
    };
    assert!(k
        .team_context(&outsider, "demo", TeamQuery::default())
        .await
        .unwrap()
        .is_empty());
    k.record_activity(&a, "demo", activity("live", &["backend"], false))
        .await
        .unwrap();
    k.record_activity(&a, "demo", activity("ended", &["backend"], false))
        .await
        .unwrap();
    k.record_activity(&a, "demo", activity("ended", &["backend"], true))
        .await
        .unwrap();
    k.record_activity(
        &b,
        "demo",
        activity("mixed", &["backend", "frontend"], false),
    )
    .await
    .unwrap();
    k.record_activity(&a, "demo", activity("expired", &["backend"], false))
        .await
        .unwrap();
    sqlx::query("UPDATE activity SET expires_at=now()-interval '1 second' WHERE org=$1 AND session_id='expired'").bind(&a.org).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO activity(org,product,user_id,session_id,services,files,intent,status,expires_at) VALUES($1,'demo','alice','unsafe-legacy',ARRAY['backend'],ARRAY['C:/private'],'sk-proj-SYNTHETIC_SECRET_CANARY','active',now()+interval '1 hour')").bind(&a.org).execute(&pool).await.unwrap();
    let signals = k.team_activity(&r, "demo").await.unwrap();
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].developer, "alice");
    let safe = serde_json::to_value(&signals).unwrap();
    assert!(safe[0].get("session_id").is_none());
    assert!(signals[0].expires_at_ms > signals[0].reported_at_ms);
    assert!(k.team_activity(&outsider, "demo").await.unwrap().is_empty());
    k.withdraw(&a, "demo", draft.id, 1).await.unwrap();
    assert!(k
        .team_context(&r, "demo", TeamQuery::default())
        .await
        .unwrap()
        .is_empty());
    k.revoke(&r, "demo").await.unwrap();
    assert!(k.team_activity(&r, "demo").await.unwrap().is_empty());
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn chronological_pages_filters_and_current_revisions() {
    let (k, a, b, _, pool) = setup().await;
    let seed = k
        .submit(&a, "demo", body("seed", &["backend"]))
        .await
        .unwrap();
    k.publish(&b, "demo", seed.id).await.unwrap();
    sqlx::query("INSERT INTO cards(id,org,product,author,idempotency_key,services,body,published,created_at) SELECT gen_random_uuid(),org,product,author,'page-'||n,services,jsonb_set(body,'{idempotency_key}',to_jsonb('page-'||n)),true,created_at FROM cards CROSS JOIN generate_series(1,50) n WHERE id=$1").bind(seed.id).execute(&pool).await.unwrap();
    let first = k
        .team_context(&b, "demo", TeamQuery::default())
        .await
        .unwrap();
    assert_eq!(first.len(), 50);
    let second = k
        .team_context(
            &b,
            "demo",
            TeamQuery {
                before: Some(first.last().unwrap().card.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert!(!first.iter().any(|x| x.card.id == second[0].card.id));
    for query in [
        TeamQuery {
            author: Some("nobody".into()),
            ..Default::default()
        },
        TeamQuery {
            service: Some("frontend".into()),
            ..Default::default()
        },
        TeamQuery {
            since_ms: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as i64
                    + 1000,
            ),
            ..Default::default()
        },
        TeamQuery {
            file: Some("frontend/missing.ts".into()),
            ..Default::default()
        },
    ] {
        assert!(k.team_context(&b, "demo", query).await.unwrap().is_empty());
    }
    assert_eq!(
        k.team_context(
            &b,
            "demo",
            TeamQuery {
                file: Some("backend/limiter.rs".into()),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .len(),
        50
    );
    assert_eq!(
        k.team_context(
            &b,
            "demo",
            TeamQuery {
                file: Some("C:/private".into()),
                ..Default::default()
            }
        )
        .await
        .unwrap_err(),
        Error::Invalid
    );
    let mut revised = body("correction", &["backend"]);
    revised.base_revision = Some(1);
    revised.intent = "Updated bounded work".into();
    let corrected = k.correct(&a, "demo", seed.id, revised).await.unwrap();
    assert_eq!(corrected.revision, 2);
    assert!(!k
        .team_context(&b, "demo", TeamQuery::default())
        .await
        .unwrap()
        .iter()
        .any(|e| e.card.id == seed.id));
    k.publish(&b, "demo", seed.id).await.unwrap();
    let all = k
        .team_context(
            &b,
            "demo",
            TeamQuery {
                file: Some("backend/limiter.rs".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let next = k
        .team_context(
            &b,
            "demo",
            TeamQuery {
                before: all.last().map(|e| e.card.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let found = all
        .iter()
        .chain(next.iter())
        .find(|e| e.card.id == seed.id)
        .unwrap();
    assert_eq!(found.card.revision, 2);
    assert_eq!(found.card.body.intent, "Updated bounded work");
    let current = k.get(&b, "demo", seed.id).await.unwrap();
    assert_eq!(current.revision, 2);
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn restricted_runtime_feed_keeps_multi_service_rows_hidden() {
    let (k, a, b, r, owner) = setup().await;
    let mixed = k
        .submit(&a, "demo", body("mixed", &["backend", "frontend"]))
        .await
        .unwrap();
    k.publish(&b, "demo", mixed.id).await.unwrap();
    k.record_activity(
        &a,
        "demo",
        activity("mixed", &["backend", "frontend"], false),
    )
    .await
    .unwrap();
    let role = format!("wk_team_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(&owner)
        .await
        .unwrap();
    sqlx::raw_sql(
        &include_str!("../../../deploy/scripts/application-role.sql").replace("wk_app", &role),
    )
    .execute(&owner)
    .await
    .unwrap();
    let selected = role.clone();
    let url = std::env::var("WK_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _| {
            let role = selected.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let runtime = Knowledge::from_pool(pool.clone());
    assert!(runtime
        .team_context(&r, "demo", TeamQuery::default())
        .await
        .unwrap()
        .is_empty());
    assert!(runtime.team_activity(&r, "demo").await.unwrap().is_empty());
    assert_eq!(
        runtime
            .team_context(&b, "demo", TeamQuery::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(runtime.team_activity(&b, "demo").await.unwrap().len(), 1);
    pool.close().await;
    sqlx::raw_sql(&format!("DROP OWNED BY {role};DROP ROLE {role};"))
        .execute(&owner)
        .await
        .unwrap();
}
