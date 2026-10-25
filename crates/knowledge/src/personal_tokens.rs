//! Personal MCP tokens: long-lived, revocable credentials for one user on one product's hosted MCP
//! endpoint. The caller generates the secret and passes only its SHA-256; the secret is never stored.
use super::*;

/// Live (unrevoked, unexpired) tokens one user may hold at a time.
pub const MAX_LIVE_PER_USER: i64 = 5;

#[derive(Debug, Serialize)]
pub struct PersonalToken {
    pub id: uuid::Uuid,
    pub product: String,
    pub label: String,
    pub created_ms: i64,
    pub expires_ms: i64,
    pub last_used_ms: Option<i64>,
}
pub(crate) fn valid_hash(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn valid_label(v: &str) -> bool {
    !v.is_empty() && v.chars().count() <= 64 && !v.chars().any(char::is_control)
}
pub(crate) const COLUMNS: &str = "id,product,label,(extract(epoch FROM created_at)*1000)::bigint AS created_ms,(extract(epoch FROM expires_at)*1000)::bigint AS expires_ms,(extract(epoch FROM last_used_at)*1000)::bigint AS last_used_ms";
pub(crate) fn token(r: &sqlx::postgres::PgRow) -> Result<PersonalToken, Error> {
    Ok(PersonalToken {
        id: r.try_get("id").map_err(db)?,
        product: r.try_get("product").map_err(db)?,
        label: r.try_get("label").map_err(db)?,
        created_ms: r.try_get("created_ms").map_err(db)?,
        expires_ms: r.try_get("expires_ms").map_err(db)?,
        last_used_ms: r.try_get("last_used_ms").map_err(db)?,
    })
}
impl Knowledge {
    /// Store a new token for a current member of `product`. `Conflict` when the user already
    /// holds [`MAX_LIVE_PER_USER`] live tokens.
    pub async fn create_personal_token(
        &self,
        a: &Actor,
        product: &str,
        label: &str,
        hash: &str,
        days: u32,
    ) -> Result<PersonalToken, Error> {
        let label = label.trim();
        if !wk_contracts::valid_id(product)
            || !valid_label(label)
            || !valid_hash(hash)
            || !(1..=365).contains(&days)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        let member =
            sqlx::query("SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3")
                .bind(&a.org)
                .bind(product)
                .bind(&a.user)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?;
        if member.is_none() {
            return Err(Error::Denied);
        }
        let created = Self::insert_personal_token(&mut tx, a, product, label, hash, days).await?;
        Self::token_audit(
            &mut tx,
            a,
            product,
            created.id,
            "personal_token_create",
            None,
        )
        .await?;
        tx.commit().await.map_err(db)?;
        Ok(created)
    }
    /// The caller has already authorized creating a token for `owner`. Serializes per owner so
    /// concurrent requests cannot exceed the cap, purges long-dead rows, and inserts.
    pub(crate) async fn insert_personal_token(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        product: &str,
        label: &str,
        hash: &str,
        days: u32,
    ) -> Result<PersonalToken, Error> {
        // Serialize per user so concurrent requests cannot exceed the cap.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("wk-token:{}:{}:{}", a.org.len(), a.org, a.user))
            .execute(&mut **tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM personal_tokens WHERE org=$1 AND user_id=$2 AND (expires_at<=now()-interval '30 days' OR revoked_at<=now()-interval '30 days')")
            .bind(&a.org)
            .bind(&a.user)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
        let live: i64 = sqlx::query_scalar("SELECT count(*) FROM personal_tokens WHERE org=$1 AND user_id=$2 AND revoked_at IS NULL AND expires_at>now()")
            .bind(&a.org)
            .bind(&a.user)
            .fetch_one(&mut **tx)
            .await
            .map_err(db)?;
        if live >= MAX_LIVE_PER_USER {
            return Err(Error::Conflict);
        }
        let row = sqlx::query(&format!("INSERT INTO personal_tokens(id,token_hash,org,user_id,product,label,expires_at) VALUES($1,$2,$3,$4,$5,$6,now()+make_interval(days=>$7)) RETURNING {COLUMNS}"))
            .bind(uuid::Uuid::new_v4())
            .bind(hash)
            .bind(&a.org)
            .bind(&a.user)
            .bind(product)
            .bind(label)
            .bind(days as i32)
            .fetch_one(&mut **tx)
            .await
            .map_err(db)?;
        token(&row)
    }
    /// The owner of a live token for exactly this product. Unknown, expired, revoked and
    /// other-product tokens are indistinguishable. `last_used_at` is refreshed at most once a minute.
    pub async fn personal_token_actor(&self, hash: &str, product: &str) -> Result<Actor, Error> {
        if !valid_hash(hash) || !wk_contracts::valid_id(product) {
            return Err(Error::Denied);
        }
        let row = sqlx::query("WITH t AS (SELECT id,org,user_id FROM personal_tokens WHERE token_hash=$1 AND product=$2 AND revoked_at IS NULL AND expires_at>now()), u AS (UPDATE personal_tokens p SET last_used_at=now() FROM t WHERE p.id=t.id AND (p.last_used_at IS NULL OR p.last_used_at<now()-interval '1 minute')) SELECT org,user_id FROM t")
            .bind(hash)
            .bind(product)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?
            .ok_or(Error::Denied)?;
        Ok(Actor {
            org: row.try_get("org").map_err(db)?,
            user: row.try_get("user_id").map_err(db)?,
        })
    }
    /// The caller's unrevoked tokens, newest first. Never includes a secret or hash.
    pub async fn list_personal_tokens(&self, a: &Actor) -> Result<Vec<PersonalToken>, Error> {
        sqlx::query(&format!("SELECT {COLUMNS} FROM personal_tokens WHERE org=$1 AND user_id=$2 AND revoked_at IS NULL ORDER BY created_at DESC,id LIMIT 50"))
            .bind(&a.org)
            .bind(&a.user)
            .fetch_all(&self.pool)
            .await
            .map_err(db)?
            .iter()
            .map(token)
            .collect()
    }
    pub async fn revoke_personal_token(&self, a: &Actor, id: uuid::Uuid) -> Result<(), Error> {
        let mut tx = self.actor_tx(a).await?;
        let product: String = sqlx::query_scalar("UPDATE personal_tokens SET revoked_at=now() WHERE id=$1 AND org=$2 AND user_id=$3 AND revoked_at IS NULL RETURNING product")
            .bind(id)
            .bind(&a.org)
            .bind(&a.user)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .ok_or(Error::NotFound)?;
        Self::token_audit(&mut tx, a, &product, id, "personal_token_revoke", None).await?;
        tx.commit().await.map_err(db)
    }
    /// Revoke every token a user holds for a product. Run whenever the membership is removed, so
    /// that adding the person back later cannot bring an old token back to life.
    pub(crate) async fn revoke_personal_tokens_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: &str,
        user: &str,
        product: &str,
    ) -> Result<(), Error> {
        sqlx::query("UPDATE personal_tokens SET revoked_at=now() WHERE org=$1 AND user_id=$2 AND product=$3 AND revoked_at IS NULL")
            .bind(org)
            .bind(user)
            .bind(product)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
        Ok(())
    }
    pub(crate) async fn token_audit(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
        action: &str,
        target: Option<&str>,
    ) -> Result<(), Error> {
        sqlx::query("INSERT INTO audit_events(org,product,user_id,action,entity_id,target_user) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(&a.org)
            .bind(product)
            .bind(&a.user)
            .bind(action)
            .bind(id)
            .bind(target)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
        Ok(())
    }
}
