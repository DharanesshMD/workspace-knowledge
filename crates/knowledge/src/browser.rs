use super::*;

pub struct BrowserSession {
    pub issuer: String,
    pub subject: String,
    pub csrf_hash: String,
}
fn valid_hash(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Knowledge {
    pub async fn create_browser_session(
        &self,
        hash: &str,
        issuer: &str,
        subject: &str,
        csrf_hash: &str,
    ) -> Result<(), Error> {
        if !valid_hash(hash) || !valid_hash(csrf_hash) {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        // Serialize per identity so concurrent callbacks cannot bypass the ten-session cap.
        // An advisory lock avoids granting the runtime UPDATE access to identities.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("wk-browser:{}:{issuer}:{subject}", issuer.len()))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let identity = sqlx::query("SELECT 1 FROM identities WHERE issuer=$1 AND subject=$2")
            .bind(issuer)
            .bind(subject)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        if identity.is_none() {
            return Err(Error::Denied);
        }
        sqlx::query("DELETE FROM browser_sessions WHERE expires_at<=now() OR last_seen<=now()-interval '30 minutes'").execute(&mut *tx).await.map_err(db)?;
        sqlx::query("DELETE FROM browser_sessions WHERE session_hash IN (SELECT session_hash FROM browser_sessions WHERE issuer=$1 AND subject=$2 ORDER BY created_at DESC,session_hash OFFSET 9)").bind(issuer).bind(subject).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("INSERT INTO browser_sessions(session_hash,issuer,subject,csrf_hash) VALUES($1,$2,$3,$4)")
            .bind(hash).bind(issuer).bind(subject).bind(csrf_hash).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }
    pub async fn browser_session(&self, hash: &str) -> Result<BrowserSession, Error> {
        if !valid_hash(hash) {
            return Err(Error::Denied);
        }
        let row = sqlx::query("UPDATE browser_sessions SET last_seen=now() WHERE session_hash=$1 AND expires_at>now() AND last_seen>now()-interval '30 minutes' RETURNING issuer,subject,csrf_hash")
            .bind(hash).fetch_optional(&self.pool).await.map_err(db)?.ok_or(Error::Denied)?;
        Ok(BrowserSession {
            issuer: row.try_get("issuer").map_err(db)?,
            subject: row.try_get("subject").map_err(db)?,
            csrf_hash: row.try_get("csrf_hash").map_err(db)?,
        })
    }
    pub async fn delete_browser_session(&self, hash: &str) -> Result<(), Error> {
        if !valid_hash(hash) {
            return Err(Error::Invalid);
        }
        sqlx::query("DELETE FROM browser_sessions WHERE session_hash=$1")
            .bind(hash)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }
    pub async fn products_for(&self, actor: &Actor) -> Result<Vec<lifecycle::Policy>, Error> {
        let mut tx = self.actor_tx(actor).await?;
        let rows = sqlx::query("SELECT product,services,can_review,policy_revision FROM memberships WHERE org=$1 AND user_id=$2 ORDER BY product LIMIT 100")
            .bind(&actor.org).bind(&actor.user).fetch_all(&mut *tx).await.map_err(db)?;
        let mut result = Vec::new();
        for r in rows {
            let product: String = r.try_get("product").map_err(db)?;
            let restrictions = Self::product_restrictions(&mut tx, actor, &product).await?;
            let features = self.policy_features(&product);
            result.push(lifecycle::Policy {
                product,
                services: r.try_get("services").map_err(db)?,
                can_review: r.try_get("can_review").map_err(db)?,
                policy_revision: r.try_get("policy_revision").map_err(db)?,
                restrictions,
                features,
            });
        }
        Ok(result)
    }
}
