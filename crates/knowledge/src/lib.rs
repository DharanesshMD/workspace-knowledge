//! PostgreSQL knowledge core. Actor identities must come from trusted authentication.
//! Grant/revoke are administrative provisioning APIs, never exposed as client tools.
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use wk_contracts::CardSubmission;
pub mod activity;
pub mod browser;
pub mod devices;
pub mod evidence;
pub mod graph;
pub mod lifecycle;
pub mod personal;
pub mod signed_evidence;
pub mod team;
#[derive(Clone)]
pub struct Actor {
    pub org: String,
    pub user: String,
}
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("access denied")]
    Denied,
    #[error("not found")]
    NotFound,
    #[error("invalid submission")]
    Invalid,
    #[error("storage unavailable")]
    Storage,
    #[error("conflict")]
    Conflict,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Card {
    pub id: uuid::Uuid,
    pub body: CardSubmission,
    pub revision: i64,
    pub published: bool,
}
#[derive(Clone)]
pub struct Knowledge {
    pool: PgPool,
    device_guard: Option<devices::DeviceGuard>,
}
fn validate_shared_body(body: &serde_json::Value) -> Result<(), Error> {
    if wk_summarize::sensitive(&body.to_string()) {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
fn db(_: sqlx::Error) -> Error {
    Error::Storage
}
fn decode(row: sqlx::postgres::PgRow) -> Result<Card, Error> {
    Ok(Card {
        id: row.try_get("id").map_err(db)?,
        body: serde_json::from_value(row.try_get("body").map_err(db)?)
            .map_err(|_| Error::Storage)?,
        revision: row.try_get("revision").map_err(db)?,
        published: row.try_get("published").map_err(db)?,
    })
}
impl Knowledge {
    async fn actor_tx(
        &self,
        actor: &Actor,
    ) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, Error> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("SELECT set_config('wk.org',$1,true),set_config('wk.user',$2,true)")
            .bind(&actor.org)
            .bind(&actor.user)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        self.lock_device(&mut tx, actor).await?;
        Ok(tx)
    }
    pub fn from_pool(pool: PgPool) -> Self {
        Self {
            pool,
            device_guard: None,
        }
    }
    pub async fn connect(url: &str) -> Result<Self, Error> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(url)
            .await
            .map_err(db)?;
        let result = Self {
            pool,
            device_guard: None,
        };
        result.migrate().await?;
        Ok(result)
    }
    /// Production application connection: never performs DDL.
    pub async fn open(url: &str) -> Result<Self, Error> {
        Ok(Self {
            device_guard: None,
            pool: sqlx::postgres::PgPoolOptions::new()
                .max_connections(10)
                .acquire_timeout(std::time::Duration::from_secs(5))
                .connect(url)
                .await
                .map_err(db)?,
        })
    }
    /// Run only with the deployment migration role, before starting app processes.
    pub async fn migrate(&self) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("SELECT pg_advisory_xact_lock(733091)")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS wk_schema_migrations(version INTEGER PRIMARY KEY)",
        )
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        for (version, source) in [
            (1, include_str!("../../../migrations/server/0001.sql")),
            (2, include_str!("../../../migrations/server/0002.sql")),
            (3, include_str!("../../../migrations/server/0003.sql")),
            (4, include_str!("../../../migrations/server/0004.sql")),
            (5, include_str!("../../../migrations/server/0005.sql")),
            (6, include_str!("../../../migrations/server/0006.sql")),
            (7, include_str!("../../../migrations/server/0007.sql")),
            (8, include_str!("../../../migrations/server/0008.sql")),
        ] {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM wk_schema_migrations WHERE version=$1)",
            )
            .bind(version)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
            if !exists {
                sqlx::raw_sql(source).execute(&mut *tx).await.map_err(db)?;
                sqlx::query("INSERT INTO wk_schema_migrations(version) VALUES($1)")
                    .bind(version)
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
            }
        }
        tx.commit().await.map_err(db)
    }
    /// Administrative bootstrap only. The public HTTP router does not expose this.
    pub async fn grant(
        &self,
        actor: &Actor,
        product: &str,
        services: &[String],
        review: bool,
    ) -> Result<(), Error> {
        if actor.org.is_empty()
            || actor.user.is_empty()
            || !wk_contracts::valid_id(product)
            || services.is_empty()
            || services.iter().any(|s| !wk_contracts::valid_id(s))
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("INSERT INTO products(org,id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(&actor.org)
            .bind(product)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("INSERT INTO memberships(org,product,user_id,services,can_review) VALUES($1,$2,$3,$4,$5) ON CONFLICT(org,product,user_id) DO UPDATE SET services=excluded.services,can_review=excluded.can_review,policy_revision=memberships.policy_revision+1").bind(&actor.org).bind(product).bind(&actor.user).bind(services).bind(review).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }
    pub async fn revoke(&self, actor: &Actor, product: &str) -> Result<(), Error> {
        sqlx::query("DELETE FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3")
            .bind(&actor.org)
            .bind(product)
            .bind(&actor.user)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }
    pub async fn submit(
        &self,
        actor: &Actor,
        product: &str,
        body: CardSubmission,
    ) -> Result<Card, Error> {
        self.device_scope(product, &body.service_ids)?;
        let mut tx = self.actor_tx(actor).await?;
        let result = Self::submit_in_transaction(&mut tx, actor, product, body).await?;
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    pub(crate) async fn submit_in_transaction(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        actor: &Actor,
        product: &str,
        body: CardSubmission,
    ) -> Result<Card, Error> {
        body.validate().map_err(|_| Error::Invalid)?;
        if body.base_revision.is_some() {
            return Err(Error::Conflict);
        }
        let policy = i64::try_from(body.policy_revision).map_err(|_| Error::Invalid)?;
        let value = serde_json::to_value(&body).map_err(|_| Error::Invalid)?;
        validate_shared_body(&value)?;
        // Lock the membership row through insertion so concurrent revocation has an order.
        let allowed=sqlx::query("SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 AND services @> $4::text[] AND policy_revision=$5 FOR SHARE").bind(&actor.org).bind(product).bind(&actor.user).bind(&body.service_ids).bind(policy).fetch_optional(&mut **tx).await.map_err(db)?;
        if allowed.is_none() {
            return Err(Error::Denied);
        }
        let row=sqlx::query("INSERT INTO cards(id,org,product,author,idempotency_key,services,body,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,COALESCE(to_timestamp($8::bigint/1000.0),now())) ON CONFLICT(org,product,author,idempotency_key) DO NOTHING RETURNING id,body,revision,published").bind(uuid::Uuid::new_v4()).bind(&actor.org).bind(product).bind(&actor.user).bind(&body.idempotency_key).bind(&body.service_ids).bind(&value).bind(body.observed_at_ms).fetch_optional(&mut **tx).await.map_err(db)?;
        let result = match row {
            Some(row) => decode(row)?,
            None => {
                let row=sqlx::query("SELECT id,body,revision,published FROM cards WHERE org=$1 AND product=$2 AND author=$3 AND idempotency_key=$4 AND NOT withdrawn").bind(&actor.org).bind(product).bind(&actor.user).bind(&body.idempotency_key).fetch_optional(&mut **tx).await.map_err(db)?.ok_or(Error::Conflict)?;
                let existing = decode(row)?;
                if serde_json::to_value(&existing.body).map_err(|_| Error::Storage)? != value {
                    return Err(Error::Conflict);
                }
                existing
            }
        };
        sqlx::query("INSERT INTO card_revisions(org,product,card_id,revision,body,changed_by) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING").bind(&actor.org).bind(product).bind(result.id).bind(result.revision).bind(&value).bind(&actor.user).execute(&mut **tx).await.map_err(db)?;
        Ok(result)
    }
    pub async fn publish(&self, actor: &Actor, product: &str, id: uuid::Uuid) -> Result<(), Error> {
        let mut tx = self.actor_tx(actor).await?;
        let row = sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$4 WHERE c.id=$1 AND c.org=$2 AND c.product=$3 AND NOT c.withdrawn AND c.body->>'capture_completeness'='complete' AND m.can_review AND m.services @> c.services AND m.policy_revision=(c.body->>'policy_revision')::bigint FOR UPDATE OF c FOR SHARE OF m")
            .bind(id).bind(&actor.org).bind(product).bind(&actor.user)
            .fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::Denied)?;
        let body: serde_json::Value = row.try_get("body").map_err(db)?;
        validate_shared_body(&body)?;
        let mut card = decode(row)?;
        sqlx::query("UPDATE cards SET published=TRUE WHERE id=$1 AND revision=$2")
            .bind(id)
            .bind(card.revision)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        card.published = true;
        Self::write_graph(
            &mut tx,
            actor,
            product,
            id,
            card.revision,
            &card,
            graph::Provenance {
                source_reference: format!("card:{id}"),
                observed_revision: card.revision.to_string(),
                source_boundary: "reviewed summary".into(),
                capture_mode: "cooperative".into(),
                coverage: "unknown".into(),
                missing_context: vec![
                    "Raw conversation is private; summary is not proof of implementation".into(),
                ],
                extraction_version: "card-graph-v1".into(),
            },
        )
        .await?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }
    pub async fn get(&self, actor: &Actor, product: &str, id: uuid::Uuid) -> Result<Card, Error> {
        let mut tx = self.actor_tx(actor).await?;
        let row=sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND c.id=$4 AND m.services @> c.services AND NOT c.withdrawn AND (c.published OR c.author=$1 OR m.can_review)").bind(&actor.user).bind(&actor.org).bind(product).bind(id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        decode(row)
    }
    pub async fn search(
        &self,
        actor: &Actor,
        product: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<Card>, Error> {
        let mut tx = self.actor_tx(actor).await?;
        if query.trim().is_empty() || query.len() > 1024 || limit == 0 || limit > 20 {
            return Err(Error::Invalid);
        }
        let rows=sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.services @> c.services AND c.published AND NOT c.withdrawn AND c.body->>'lifecycle' NOT IN ('superseded','deprecated') AND to_tsvector('simple',(c.body->>'title') || ' ' || (c.body->>'intent') || ' ' || (c.body->>'rationale') || ' ' || COALESCE(c.body->>'decisions','') || ' ' || COALESCE(c.body->>'constraints','') || ' ' || COALESCE(c.body->>'open_questions','')) @@ plainto_tsquery('simple',$4) ORDER BY c.id LIMIT $5").bind(&actor.user).bind(&actor.org).bind(product).bind(query).bind(i64::from(limit)).fetch_all(&mut *tx).await.map_err(db)?;
        rows.into_iter().map(decode).collect()
    }
}
