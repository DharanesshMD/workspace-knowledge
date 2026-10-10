//! Restore invalidates pre-restore collector tokens even when absent from a snapshot.
use super::*;
impl Knowledge {
    pub async fn collector_auth_allowed(&self, issued: Option<u64>) -> Result<bool, Error> {
        let cutoff: i64 =
            sqlx::query_scalar("SELECT not_before FROM collector_security_epoch WHERE singleton")
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Storage)?
            .as_secs();
        Ok(issued.is_none_or(|at| at <= now.saturating_add(60))
            && (cutoff == 0 || issued.is_some_and(|at| at >= cutoff as u64)))
    }
    /// Protected operator only. Allow60seconds of issuer clock skew, then fresh sign-in.
    pub async fn collector_reset_auth(&self) -> Result<u64, Error> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let allowed:bool=sqlx::query_scalar("SELECT pg_has_role(current_user,c.relowner,'USAGE') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname='collector_security_epoch'").fetch_one(&mut *tx).await.map_err(db)?;
        if !allowed {
            return Err(Error::Denied);
        }
        let cutoff:i64=sqlx::query_scalar("UPDATE collector_security_epoch SET not_before=GREATEST(not_before,floor(extract(epoch FROM clock_timestamp()))::bigint+61) WHERE singleton RETURNING not_before").fetch_one(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(cutoff as u64)
    }
}
