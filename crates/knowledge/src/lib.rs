//! PostgreSQL knowledge core. Actor identities must come from trusted authentication.
//! Grant/revoke are administrative provisioning APIs, never exposed as client tools.
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use wk_contracts::CardSubmission;
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
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn connect(url: &str) -> Result<Self, Error> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(url)
            .await
            .map_err(db)?;
        let mut tx = pool.begin().await.map_err(db)?;
        sqlx::query("SELECT pg_advisory_xact_lock(733091)")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::raw_sql(include_str!("../../../migrations/server/0001.sql"))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(Self { pool })
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
        sqlx::query("INSERT INTO memberships(org,product,user_id,services,can_review) VALUES($1,$2,$3,$4,$5) ON CONFLICT(org,product,user_id) DO UPDATE SET services=excluded.services,can_review=excluded.can_review").bind(&actor.org).bind(product).bind(&actor.user).bind(services).bind(review).execute(&mut *tx).await.map_err(db)?;
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
        body.validate().map_err(|_| Error::Invalid)?;
        if body.base_revision.is_some() {
            return Err(Error::Conflict);
        }
        let policy = i64::try_from(body.policy_revision).map_err(|_| Error::Invalid)?;
        let value = serde_json::to_value(&body).map_err(|_| Error::Invalid)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        // Lock the membership row through insertion so concurrent revocation has an order.
        let allowed=sqlx::query("SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 AND services @> $4::text[] AND policy_revision=$5 FOR SHARE").bind(&actor.org).bind(product).bind(&actor.user).bind(&body.service_ids).bind(policy).fetch_optional(&mut *tx).await.map_err(db)?;
        if allowed.is_none() {
            return Err(Error::Denied);
        }
        let row=sqlx::query("INSERT INTO cards(id,org,product,author,idempotency_key,services,body) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(org,product,author,idempotency_key) DO NOTHING RETURNING id,body,revision,published").bind(uuid::Uuid::new_v4()).bind(&actor.org).bind(product).bind(&actor.user).bind(&body.idempotency_key).bind(&body.service_ids).bind(&value).fetch_optional(&mut *tx).await.map_err(db)?;
        let result = match row {
            Some(row) => decode(row)?,
            None => {
                let row=sqlx::query("SELECT id,body,revision,published FROM cards WHERE org=$1 AND product=$2 AND author=$3 AND idempotency_key=$4 AND NOT withdrawn").bind(&actor.org).bind(product).bind(&actor.user).bind(&body.idempotency_key).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::Conflict)?;
                let existing = decode(row)?;
                if serde_json::to_value(&existing.body).map_err(|_| Error::Storage)? != value {
                    return Err(Error::Conflict);
                }
                existing
            }
        };
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    pub async fn publish(&self, actor: &Actor, product: &str, id: uuid::Uuid) -> Result<(), Error> {
        let result=sqlx::query("UPDATE cards c SET published=TRUE WHERE c.id=$1 AND c.org=$2 AND c.product=$3 AND NOT c.withdrawn AND c.body->>'capture_completeness'='complete' AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=c.org AND m.product=c.product AND m.user_id=$4 AND m.can_review AND m.services @> c.services AND m.policy_revision=(c.body->>'policy_revision')::bigint)").bind(id).bind(&actor.org).bind(product).bind(&actor.user).execute(&self.pool).await.map_err(db)?;
        if result.rows_affected() == 0 {
            return Err(Error::Denied);
        }
        Ok(())
    }
    pub async fn get(&self, actor: &Actor, product: &str, id: uuid::Uuid) -> Result<Card, Error> {
        let row=sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND c.id=$4 AND m.services @> c.services AND NOT c.withdrawn AND (c.published OR c.author=$1)").bind(&actor.user).bind(&actor.org).bind(product).bind(id).fetch_optional(&self.pool).await.map_err(db)?.ok_or(Error::NotFound)?;
        decode(row)
    }
    pub async fn search(
        &self,
        actor: &Actor,
        product: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<Card>, Error> {
        if query.trim().is_empty() || query.len() > 1024 || limit == 0 || limit > 20 {
            return Err(Error::Invalid);
        }
        let rows=sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.services @> c.services AND c.published AND NOT c.withdrawn AND to_tsvector('simple',(c.body->>'title') || ' ' || (c.body->>'intent') || ' ' || (c.body->>'rationale')) @@ plainto_tsquery('simple',$4) ORDER BY c.id LIMIT $5").bind(&actor.user).bind(&actor.org).bind(product).bind(query).bind(i64::from(limit)).fetch_all(&self.pool).await.map_err(db)?;
        rows.into_iter().map(decode).collect()
    }
}
