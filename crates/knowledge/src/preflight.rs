//! Owner/browser confirmation and single-use device receipts. Never an MCP acknowledgement API.
use super::*;
use crate::{change_context::Selection, code_anchors::union_scopes, code_provenance::decode_value};
use wk_contracts::change_context::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedIdentity {
    pub device_id: uuid::Uuid,
    pub binding_id: uuid::Uuid,
    pub security_epoch: i64,
}
#[derive(Serialize)]
pub struct PreflightView {
    pub brief: ChangeContextBrief,
    pub operation_class: String,
    pub acknowledged: bool,
    pub consumed: bool,
}
struct IdentityState {
    services: Vec<String>,
    policy_revision: i64,
    stamp: serde_json::Value,
}
struct Stored {
    identity: ProtectedIdentity,
    request: ChangeContextRequest,
    brief: ChangeContextBrief,
    versions: serde_json::Value,
    policy_revision: i64,
    acknowledged: bool,
    consumed: bool,
}
impl Knowledge {
    /// Caller must derive the device ID from validated signed collector authentication.
    pub async fn protected_identity(
        &self,
        a: &Actor,
        p: &str,
        device_id: uuid::Uuid,
    ) -> Result<ProtectedIdentity, Error> {
        let mut tx = self.code_tx(a, p).await?;
        let row=sqlx::query("SELECT b.id FROM workspace_bindings b JOIN registered_devices d ON d.org=b.org AND d.product=b.product AND d.owner=b.user_id AND d.id::text=b.device_id AND d.approval_hash=b.approval_hash JOIN memberships m ON m.org=b.org AND m.product=b.product AND m.user_id=b.user_id WHERE b.org=$1 AND b.product=$2 AND b.user_id=$3 AND d.id=$4 AND NOT b.revoked AND d.revoked_at IS NULL AND b.policy_revision=m.policy_revision AND m.services @> b.services AND m.services @> d.services ORDER BY b.id LIMIT 2")
            .bind(&a.org).bind(p).bind(&a.user).bind(device_id).fetch_all(&mut *tx).await.map_err(db)?;
        if row.len() != 1 {
            return Err(Error::Denied);
        }
        let epoch: i64 = sqlx::query_scalar(
            "SELECT not_before FROM collector_security_epoch WHERE singleton FOR SHARE",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        let id = ProtectedIdentity {
            device_id,
            binding_id: row[0].try_get("id").map_err(db)?,
            security_epoch: epoch,
        };
        Self::preflight_identity(&mut tx, a, p, &id).await?;
        tx.commit().await.map_err(db)?;
        Ok(id)
    }
    async fn preflight_identity(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        id: &ProtectedIdentity,
    ) -> Result<IdentityState, Error> {
        let epoch: i64 = sqlx::query_scalar(
            "SELECT not_before FROM collector_security_epoch WHERE singleton FOR SHARE",
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(db)?;
        if epoch != id.security_epoch {
            return Err(Error::Denied);
        }
        let row=sqlx::query("SELECT d.services,d.revision,m.policy_revision,to_jsonb(b) AS binding,d.settings FROM registered_devices d JOIN workspace_bindings b ON b.org=d.org AND b.product=d.product AND b.user_id=d.owner AND b.device_id=d.id::text AND b.approval_hash=d.approval_hash JOIN memberships m ON m.org=d.org AND m.product=d.product AND m.user_id=d.owner WHERE d.org=$1 AND d.product=$2 AND d.owner=$3 AND d.id=$4 AND b.id=$5 AND d.revoked_at IS NULL AND NOT b.revoked AND b.policy_revision=m.policy_revision AND m.services @> d.services AND m.services @> b.services AND b.services @> d.services FOR SHARE OF d,b,m")
            .bind(&a.org).bind(p).bind(&a.user).bind(id.device_id).bind(id.binding_id).fetch_optional(&mut **tx).await.map_err(db)?.ok_or(Error::Denied)?;
        let settings: serde_json::Value = row.try_get("settings").map_err(db)?;
        if settings["paused"] == true {
            return Err(Error::Denied);
        }
        let version: i64 = row.try_get("revision").map_err(db)?;
        let binding: serde_json::Value = row.try_get("binding").map_err(db)?;
        Ok(IdentityState {
            services: row.try_get("services").map_err(db)?,
            policy_revision: row.try_get("policy_revision").map_err(db)?,
            stamp: serde_json::json!({"device_revision":version,"binding":binding,"security_epoch":epoch}),
        })
    }
    async fn preflight_selection(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        r: &ChangeContextRequest,
        state: &IdentityState,
    ) -> Result<Selection, Error> {
        let mut selected = Self::select_context(tx, a, p, r, Some(&state.services)).await?;
        selected
            .versions
            .insert("protected_identity".into(), state.stamp.clone());
        let watermark: String = sqlx::query_scalar(SOURCE_WATERMARK)
            .bind(&a.org)
            .bind(p)
            .bind(&a.user)
            .bind(r.repository_id)
            .bind(&state.services)
            .fetch_one(&mut **tx)
            .await
            .map_err(db)?;
        selected
            .versions
            .insert("repository_source_set".into(), serde_json::json!(watermark));
        Ok(selected)
    }
    pub async fn create_preflight(
        &self,
        a: &Actor,
        p: &str,
        id: ProtectedIdentity,
        r: ChangeContextRequest,
    ) -> Result<ChangeContextBrief, Error> {
        r.validate().map_err(|_| Error::Invalid)?;
        if r.operation_class == "read_context" || r.operation_handle.is_none() {
            return Err(Error::Invalid);
        }
        if self
            .device_guard
            .as_ref()
            .is_some_and(|g| g.id != id.device_id || g.product != p)
        {
            return Err(Error::Denied);
        }
        let mut tx = self.code_tx(a, p).await?;
        let state = Self::preflight_identity(&mut tx, a, p, &id).await?;
        let capacity: bool = sqlx::query_scalar("SELECT wk_preflight_has_capacity($1,$2,$3)")
            .bind(&a.org)
            .bind(p)
            .bind(&a.user)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        if !capacity {
            return Err(Error::Conflict);
        }
        let mut selected = Self::preflight_selection(&mut tx, a, p, &r, &state).await?;
        let repo = Self::code_repository(&mut tx, a, p, r.repository_id).await?;
        let brief_id = uuid::Uuid::new_v4();
        let expires: i64 = sqlx::query_scalar(
            "SELECT floor(extract(epoch FROM now()+interval '5 minutes')*1000)::bigint",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        selected.brief.brief_id = Some(brief_id);
        selected.brief.expires_at_ms = Some(expires);
        selected.brief.observation_assurance = "authenticated_device_report".into();
        while serde_json::to_vec(&selected.brief)
            .map_err(|_| Error::Storage)?
            .len()
            > r.budget_bytes as usize
        {
            selected.brief.truncated = true;
            if !selected.brief.gaps.contains(&"context_truncated".into()) {
                selected.brief.gaps.push("context_truncated".into());
            }
            if selected.brief.items.pop().is_none() {
                return Err(Error::Invalid);
            }
        }
        let inserted=sqlx::query("INSERT INTO code_preflights(id,org,product,owner,device_id,binding_id,security_epoch,services,policy_revision,repository_id,repository_revision,operation_handle,request,source_versions,brief,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,now()+interval '5 minutes') ON CONFLICT(org,product,owner,device_id,operation_handle) DO NOTHING")
            .bind(brief_id).bind(&a.org).bind(p).bind(&a.user).bind(id.device_id).bind(id.binding_id).bind(id.security_epoch).bind(union_scopes(&repo.services,&state.services)).bind(state.policy_revision).bind(repo.id).bind(repo.revision).bind(r.operation_handle.unwrap()).bind(serde_json::to_value(&r).map_err(|_|Error::Invalid)?).bind(serde_json::to_value(selected.versions).map_err(|_|Error::Storage)?).bind(serde_json::to_value(&selected.brief).map_err(|_|Error::Storage)?).execute(&mut *tx).await.map_err(db)?;
        if inserted.rows_affected() != 1 {
            return Err(Error::Conflict);
        }
        Self::preflight_audit(&mut tx, a, p, brief_id, "preflight_created").await?;
        tx.commit().await.map_err(db)?;
        Ok(selected.brief)
    }
    async fn stored_preflight(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<Stored, Error> {
        let row=sqlx::query("SELECT pf.device_id,pf.binding_id,pf.security_epoch,pf.request,pf.brief,pf.source_versions,pf.policy_revision,pf.acknowledged_at IS NOT NULL AS acknowledged,pf.consumed_at IS NOT NULL AS consumed,pf.expires_at>clock_timestamp() AS active FROM code_preflights pf JOIN memberships m ON m.org=pf.org AND m.product=pf.product AND m.user_id=pf.owner WHERE pf.id=$1 AND pf.org=$2 AND pf.product=$3 AND pf.owner=$4 AND m.services @> pf.services FOR UPDATE OF pf FOR SHARE OF m")
            .bind(id).bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut **tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        if !row.try_get::<bool, _>("active").map_err(db)? {
            return Err(Error::Conflict);
        }
        Ok(Stored {
            identity: ProtectedIdentity {
                device_id: row.try_get("device_id").map_err(db)?,
                binding_id: row.try_get("binding_id").map_err(db)?,
                security_epoch: row.try_get("security_epoch").map_err(db)?,
            },
            request: decode_value(row.try_get("request").map_err(db)?)?,
            brief: decode_value(row.try_get("brief").map_err(db)?)?,
            versions: row.try_get("source_versions").map_err(db)?,
            policy_revision: row.try_get("policy_revision").map_err(db)?,
            acknowledged: row.try_get("acknowledged").map_err(db)?,
            consumed: row.try_get("consumed").map_err(db)?,
        })
    }
    async fn validate_stored(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        stored: &Stored,
    ) -> Result<(), Error> {
        let state = Self::preflight_identity(tx, a, p, &stored.identity).await?;
        if state.policy_revision != stored.policy_revision {
            return Err(Error::Conflict);
        }
        let selected = Self::preflight_selection(tx, a, p, &stored.request, &state).await?;
        if serde_json::to_value(selected.versions).map_err(|_| Error::Storage)? != stored.versions
            || stored.request.repository_id != stored.brief.repository_id
            || stored.request.base_revision != stored.brief.base_revision
            || stored.request.targets != stored.brief.targets
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    pub async fn get_preflight(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<ChangeContextBrief, Error> {
        Ok(self.preflight_view(a, p, id).await?.brief)
    }
    pub async fn preflight_view(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<PreflightView, Error> {
        let mut tx = self.code_tx(a, p).await?;
        let stored = Self::stored_preflight(&mut tx, a, p, id).await?;
        Self::validate_stored(&mut tx, a, p, &stored).await?;
        tx.commit().await.map_err(db)?;
        Ok(PreflightView {
            brief: stored.brief,
            operation_class: stored.request.operation_class,
            acknowledged: stored.acknowledged,
            consumed: stored.consumed,
        })
    }
    pub async fn acknowledge_preflight(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<(), Error> {
        // Only browser-authenticated routes may invoke this; a signed device cannot.
        if self.device_guard.is_some() {
            return Err(Error::Denied);
        }
        let mut tx = self.code_tx(a, p).await?;
        let stored = Self::stored_preflight(&mut tx, a, p, id).await?;
        if stored.acknowledged || stored.consumed {
            return Err(Error::Conflict);
        }
        Self::validate_stored(&mut tx, a, p, &stored).await?;
        let result=sqlx::query("UPDATE code_preflights SET acknowledged_at=clock_timestamp() WHERE id=$1 AND org=$2 AND product=$3 AND owner=$4 AND acknowledged_at IS NULL AND consumed_at IS NULL AND expires_at>clock_timestamp()")
            .bind(id).bind(&a.org).bind(p).bind(&a.user).execute(&mut *tx).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(Error::Conflict);
        }
        Self::preflight_audit(&mut tx, a, p, id, "preflight_acknowledged").await?;
        tx.commit().await.map_err(db)
    }
    pub async fn consume_preflight(
        &self,
        a: &Actor,
        p: &str,
        identity: ProtectedIdentity,
        id: uuid::Uuid,
        operation: uuid::Uuid,
    ) -> Result<(), Error> {
        if self
            .device_guard
            .as_ref()
            .is_some_and(|g| g.id != identity.device_id || g.product != p)
        {
            return Err(Error::Denied);
        }
        let mut tx = self.code_tx(a, p).await?;
        let stored = Self::stored_preflight(&mut tx, a, p, id).await?;
        if stored.identity != identity
            || stored.request.operation_handle != Some(operation)
            || !stored.acknowledged
            || stored.consumed
        {
            return Err(Error::Denied);
        }
        Self::validate_stored(&mut tx, a, p, &stored).await?;
        let result=sqlx::query("UPDATE code_preflights SET consumed_at=clock_timestamp() WHERE id=$1 AND org=$2 AND product=$3 AND owner=$4 AND device_id=$5 AND operation_handle=$6 AND consumed_at IS NULL AND acknowledged_at IS NOT NULL AND expires_at>clock_timestamp()")
            .bind(id).bind(&a.org).bind(p).bind(&a.user).bind(identity.device_id).bind(operation).execute(&mut *tx).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(Error::Denied);
        }
        Self::preflight_audit(&mut tx, a, p, id, "preflight_consumed").await?;
        tx.commit().await.map_err(db)
    }
    async fn preflight_audit(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        action: &str,
    ) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO audit_events(org,product,user_id,action,entity_id) VALUES($1,$2,$3,$4,$5)",
        )
        .bind(&a.org)
        .bind(p)
        .bind(&a.user)
        .bind(action)
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
        Ok(())
    }
}
// Body hashes stay internal; no fingerprint, source counts, command metadata or
// directory enumeration is exposed to a caller. Includes visible sources beyond
// the response limit; deletions and relationship removals alter the same set.
const SOURCE_WATERMARK: &str = r#"
WITH member AS (SELECT services FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3),
anchors AS (SELECT an.* FROM code_rationale_anchors an,member m WHERE an.org=$1 AND an.product=$2 AND an.repository_id=$4 AND NOT an.revoked AND an.confirmed_by=an.owner AND an.reviewed_by IS NOT NULL AND m.services @> an.services AND $5::text[] @> an.services),
visible_cards AS (SELECT DISTINCT c.* FROM cards c JOIN anchors an ON an.org=c.org AND an.product=c.product AND an.card_id=c.id,member m WHERE c.published AND NOT c.withdrawn AND c.revision=an.card_revision AND m.services @> c.services AND $5::text[] @> c.services),
parts AS (
 SELECT 'anchor:'||an.id::text||':'||to_jsonb(an)::text AS part FROM anchors an JOIN visible_cards c ON c.id=an.card_id
 UNION ALL SELECT 'card:'||c.id::text||':'||to_jsonb(c)::text FROM visible_cards c
 UNION ALL SELECT 'contribution:'||co.id::text||':'||to_jsonb(co)::text FROM code_contributions co,member m WHERE co.org=$1 AND co.product=$2 AND co.repository_id=$4 AND m.services @> co.services AND $5::text[] @> co.services AND co.confirmed_by=co.owner AND co.reviewed_by IS NOT NULL
 UNION ALL SELECT 'mapping:'||cm.id::text||':'||to_jsonb(cm)::text FROM code_mappings cm,member m WHERE cm.org=$1 AND cm.product=$2 AND cm.repository_id=$4 AND m.services @> cm.services AND $5::text[] @> cm.services
 UNION ALL SELECT 'maintainer:'||cm.id::text||':'||to_jsonb(cm)::text FROM code_maintainers cm,member m WHERE cm.org=$1 AND cm.product=$2 AND cm.repository_id=$4 AND m.services @> cm.services AND $5::text[] @> cm.services
 UNION ALL SELECT 'profile:'||cp.user_id||':'||to_jsonb(cp)::text FROM code_profiles cp,member m WHERE cp.org=$1 AND cp.product=$2 AND m.services @> cp.services AND $5::text[] @> cp.services AND (EXISTS(SELECT 1 FROM anchors an WHERE an.owner=cp.user_id) OR EXISTS(SELECT 1 FROM code_contributions co WHERE co.org=$1 AND co.product=$2 AND co.repository_id=$4 AND co.owner=cp.user_id AND co.confirmed_by=co.owner AND co.reviewed_by IS NOT NULL AND m.services @> co.services AND $5::text[] @> co.services) OR EXISTS(SELECT 1 FROM code_maintainers cm WHERE cm.org=$1 AND cm.product=$2 AND cm.repository_id=$4 AND cm.user_id=cp.user_id AND m.services @> cm.services AND $5::text[] @> cm.services))
 UNION ALL SELECT 'node:'||n.id::text||':'||to_jsonb(n)::text FROM graph_nodes n JOIN visible_cards c ON c.id=n.card_id WHERE n.org=$1 AND n.product=$2
 UNION ALL SELECT 'edge:'||e.source::text||':'||e.target::text||':'||e.kind FROM graph_edges e JOIN graph_nodes sn ON sn.org=e.org AND sn.product=e.product AND sn.id=e.source JOIN graph_nodes tn ON tn.org=e.org AND tn.product=e.product AND tn.id=e.target JOIN visible_cards sc ON sc.id=sn.card_id JOIN visible_cards tc ON tc.id=tn.card_id WHERE e.org=$1 AND e.product=$2
 UNION ALL SELECT 'evidence:'||te.id::text||':'||to_jsonb(te)::text FROM trusted_evidence te JOIN visible_cards c ON c.id=te.card_id WHERE te.org=$1 AND te.product=$2
)
SELECT encode(sha256(convert_to(COALESCE(string_agg(part,E'\n' ORDER BY part),''),'UTF8')),'hex') FROM parts
"#;
