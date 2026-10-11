//! Deployment-operator cleanup, never an application/HTTP capability.
use super::*;
#[derive(Debug, Default, Serialize)]
pub struct RetentionCounts {
    pub busy: bool,
    pub personal: u64,
    pub preflights: u64,
    pub reports: u64,
    pub activity: u64,
    pub sessions: u64,
    pub nonces: u64,
    pub credentials: u64,
    pub audit: u64,
}
impl Knowledge {
    async fn retention_operator(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<(), Error> {
        let allowed:bool=sqlx::query_scalar("SELECT pg_has_role(current_user,c.relowner,'USAGE') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname='personal_deletions'").fetch_one(&mut **tx).await.map_err(db)?;
        if !allowed {
            return Err(Error::Denied);
        }
        Ok(())
    }
    pub async fn retention_batch(&self, limit: u32) -> Result<RetentionCounts, Error> {
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        Self::retention_operator(&mut tx).await?;
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(733092)")
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        if !locked {
            return Ok(RetentionCounts {
                busy: true,
                ..Default::default()
            });
        }
        let mut result = RetentionCounts::default();
        result.personal=sqlx::query("WITH expired AS (SELECT id FROM personal_summaries WHERE NOT deleted AND expires_at<=now() ORDER BY expires_at,id LIMIT $1 FOR UPDATE SKIP LOCKED) UPDATE personal_summaries s SET body=NULL,deleted=true,revision=revision+1 FROM expired e WHERE s.id=e.id").bind(i64::from(limit)).execute(&mut *tx).await.map_err(db)?.rows_affected();
        result.preflights=sqlx::query("WITH expired AS (SELECT id FROM code_preflights WHERE cleanup_at<=now() ORDER BY cleanup_at,id LIMIT $1 FOR UPDATE SKIP LOCKED) DELETE FROM code_preflights p USING expired e WHERE p.id=e.id").bind(i64::from(limit)).execute(&mut *tx).await.map_err(db)?.rows_affected();
        // Names/predicates are fixed code, not user-controlled SQL or identifiers.
        for (table, predicate, order, target) in [
            (
                "device_reports",
                "expires_at<=now()",
                "expires_at",
                &mut result.reports,
            ),
            (
                "activity",
                "expires_at<=now()",
                "expires_at",
                &mut result.activity,
            ),
            (
                "browser_sessions",
                "expires_at<=now() OR last_seen<=now()-interval '30 minutes'",
                "LEAST(expires_at,last_seen+interval '30 minutes')",
                &mut result.sessions,
            ),
            (
                "device_nonces",
                "expires_at<=now()",
                "expires_at",
                &mut result.nonces,
            ),
            (
                "collector_credentials",
                "expires_at<=now()",
                "expires_at",
                &mut result.credentials,
            ),
            (
                "audit_events",
                "created_at<=now()-interval '90 days'",
                "created_at",
                &mut result.audit,
            ),
        ] {
            let sql=format!("WITH expired AS (SELECT ctid FROM {table} WHERE {predicate} ORDER BY {order},ctid LIMIT $1 FOR UPDATE SKIP LOCKED) DELETE FROM {table} t USING expired e WHERE t.ctid=e.ctid");
            *target = sqlx::query(&sql)
                .bind(i64::from(limit))
                .execute(&mut *tx)
                .await
                .map_err(db)?
                .rows_affected();
        }
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    pub async fn replay_personal_deletions(&self) -> Result<u64, Error> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        Self::retention_operator(&mut tx).await?;
        let count=sqlx::query("UPDATE personal_summaries s SET body=NULL,deleted=true,revision=GREATEST(s.revision,d.revision) FROM personal_deletions d WHERE s.org=d.org AND s.product=d.product AND s.owner=d.owner AND (s.id=d.note_id OR encode(sha256(convert_to(s.idempotency_key,'UTF8')),'hex')=d.key_hash) AND (NOT s.deleted OR s.body IS NOT NULL OR s.revision<d.revision)").execute(&mut *tx).await.map_err(db)?.rows_affected();
        tx.commit().await.map_err(db)?;
        Ok(count)
    }
}
