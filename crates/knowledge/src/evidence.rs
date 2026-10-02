//! Operator/trusted connector evidence ingestion; never accepts a client assertion as proof.
use super::*;
use serde_json::Value;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedEvidence {
    pub provider: String,
    pub reference: String,
    pub code_revision: String,
    pub kind: String,
    pub metadata: Value,
}
impl VerifiedEvidence {
    pub fn validate(&self, allowed_hosts: &[String]) -> Result<(), Error> {
        let u = url::Url::parse(&self.reference).map_err(|_| Error::Invalid)?;
        if u.scheme() != "https"
            || !u.username().is_empty()
            || u.password().is_some()
            || !allowed_hosts
                .iter()
                .any(|h| Some(h.as_str()) == u.host_str())
            || self.provider.is_empty()
            || self.code_revision.len() < 7
            || self.code_revision.len() > 128
            || !self.code_revision.bytes().all(|b| b.is_ascii_hexdigit())
            || !["code_observed", "tests_verified", "owner_confirmed"].contains(&self.kind.as_str())
        {
            return Err(Error::Invalid);
        }
        if self.kind == "tests_verified"
            && (self.metadata.get("passed").and_then(Value::as_bool) != Some(true)
                || ["command", "executed_at", "environment"].iter().any(|k| {
                    self.metadata
                        .get(k)
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                }))
        {
            return Err(Error::Invalid);
        }
        if serde_json::to_vec(&self.metadata)
            .map_err(|_| Error::Invalid)?
            .len()
            > 8192
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
impl Knowledge {
    /// Administrative connector boundary, deliberately absent from the public router.
    /// Caller must independently verify repository/test attestation authenticity.
    pub async fn attest(
        &self,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
        revision: i64,
        e: VerifiedEvidence,
        hosts: &[String],
    ) -> Result<(), Error> {
        self.attest_event(
            a,
            product,
            id,
            revision,
            e,
            hosts,
            uuid::Uuid::new_v4(),
            false,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn attest_event(
        &self,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
        revision: i64,
        e: VerifiedEvidence,
        hosts: &[String],
        event: uuid::Uuid,
        revoke: bool,
    ) -> Result<(), Error> {
        e.validate(hosts)?;
        let mut tx = self.actor_tx(a).await?;
        let row=sqlx::query("SELECT c.revision,c.body FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND c.id=$4 AND NOT c.withdrawn AND m.can_review AND m.services @> c.services FOR UPDATE OF c FOR SHARE OF m").bind(&a.user).bind(&a.org).bind(product).bind(id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let body: Value = row.try_get("body").map_err(db)?;
        if matches!(
            body["lifecycle"].as_str(),
            Some("superseded" | "deprecated")
        ) {
            return Err(Error::Conflict);
        }
        if row.try_get::<i64, _>("revision").map_err(db)? != revision {
            return Err(Error::Conflict);
        }
        // A new source revision invalidates all earlier code/test claims. A signed
        // revocation invalidates this provider's claims without reviving older runs.
        if revoke {
            sqlx::query("UPDATE trusted_evidence SET stale=TRUE WHERE org=$1 AND product=$2 AND card_id=$3 AND provider=$4")
                .bind(&a.org).bind(product).bind(id).bind(&e.provider).execute(&mut *tx).await.map_err(db)?;
        } else if e.kind == "code_observed" {
            sqlx::query("UPDATE trusted_evidence SET stale=TRUE WHERE org=$1 AND product=$2 AND card_id=$3 AND code_revision<>$4")
                .bind(&a.org).bind(product).bind(id).bind(&e.code_revision).execute(&mut *tx).await.map_err(db)?;
        }
        let inserted = sqlx::query("INSERT INTO trusted_evidence(id,org,product,card_id,card_revision,provider,reference,code_revision,kind,metadata,stale) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT(id) DO NOTHING").bind(event).bind(&a.org).bind(product).bind(id).bind(revision).bind(&e.provider).bind(&e.reference).bind(&e.code_revision).bind(&e.kind).bind(&e.metadata).bind(revoke).execute(&mut *tx).await.map_err(db)?;
        if inserted.rows_affected() == 0 {
            return Err(Error::Conflict);
        }
        // A matching immutable code observation and test run are both necessary.
        let verified:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM trusted_evidence c JOIN trusted_evidence t ON t.org=c.org AND t.product=c.product AND t.card_id=c.card_id AND t.card_revision=c.card_revision AND t.code_revision=c.code_revision WHERE c.org=$1 AND c.product=$2 AND c.card_id=$3 AND c.card_revision=$4 AND c.kind='code_observed' AND t.kind='tests_verified' AND NOT c.stale AND NOT t.stale)").bind(&a.org).bind(product).bind(id).bind(revision).fetch_one(&mut *tx).await.map_err(db)?;
        let observed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM trusted_evidence WHERE org=$1 AND product=$2 AND card_id=$3 AND card_revision=$4 AND kind='code_observed' AND NOT stale)")
            .bind(&a.org).bind(product).bind(id).bind(revision).fetch_one(&mut *tx).await.map_err(db)?;
        let lifecycle = if verified {
            "verified"
        } else if observed {
            "implemented"
        } else {
            "in_progress"
        };
        sqlx::query(
            "UPDATE cards SET body=jsonb_set(body,'{lifecycle}',to_jsonb($1::text)) WHERE id=$2",
        )
        .bind(lifecycle)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("UPDATE card_revisions SET body=jsonb_set(body,'{lifecycle}',to_jsonb($1::text)) WHERE org=$2 AND product=$3 AND card_id=$4 AND revision=$5").bind(lifecycle).bind(&a.org).bind(product).bind(id).bind(revision).execute(&mut *tx).await.map_err(db)?;
        let verification = if verified {
            "tests_verified"
        } else if observed {
            "code_observed"
        } else {
            "stale"
        };
        sqlx::query("UPDATE graph_nodes SET provenance=jsonb_set(provenance,'{verification}',to_jsonb($1::text)) WHERE org=$2 AND product=$3 AND card_id=$4")
            .bind(verification).bind(&a.org).bind(product).bind(id).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }
}

impl Knowledge {
    /// Shared evidence exposes immutable references and freshness, never connector
    /// payloads, commands, credentials or private conversation content.
    pub async fn evidence(
        &self,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
    ) -> Result<Vec<Value>, Error> {
        let mut tx = self.actor_tx(a).await?;
        let revision: i64 = sqlx::query_scalar("SELECT c.revision FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND c.id=$4 AND NOT c.withdrawn AND m.services @> c.services AND (c.published OR c.author=$1 OR m.can_review) FOR SHARE OF c,m")
            .bind(&a.user).bind(&a.org).bind(product).bind(id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let rows = sqlx::query("SELECT e.provider,e.reference,e.code_revision,e.kind,e.card_revision,e.stale FROM trusted_evidence e JOIN card_revisions r ON r.org=e.org AND r.product=e.product AND r.card_id=e.card_id AND r.revision=e.card_revision JOIN memberships m ON m.org=e.org AND m.product=e.product AND m.user_id=$4 WHERE e.org=$1 AND e.product=$2 AND e.card_id=$3 AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements_text(r.body->'service_ids') s WHERE NOT s=ANY(m.services)) ORDER BY e.card_revision DESC,e.id LIMIT 100")
            .bind(&a.org).bind(product).bind(id).bind(&a.user).fetch_all(&mut *tx).await.map_err(db)?;
        rows.into_iter().map(|r| {
            let observed: i64 = r.try_get("card_revision").map_err(db)?;
            Ok(serde_json::json!({"provider":r.try_get::<String,_>("provider").map_err(db)?,"reference":r.try_get::<String,_>("reference").map_err(db)?,"code_revision":r.try_get::<String,_>("code_revision").map_err(db)?,"kind":r.try_get::<String,_>("kind").map_err(db)?,"card_revision":observed,"stale":r.try_get::<bool,_>("stale").map_err(db)? || observed != revision}))
        }).collect()
    }
}
