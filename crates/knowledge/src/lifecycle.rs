use super::*;
use serde_json::{json, Value};
#[derive(schemars::JsonSchema, Serialize)]
pub struct Policy {
    pub product: String,
    pub services: Vec<String>,
    pub can_review: bool,
    pub policy_revision: i64,
    pub restrictions: admin::ProductPolicy,
    /// Server capabilities a collector may rely on. `card_provenance`: accepts `provenance` on
    /// card submissions and lets a reviewer publish an `assistant_written` partial card.
    pub features: Vec<String>,
}
impl Knowledge {
    pub async fn policy(&self, a: &Actor, product: &str) -> Result<Policy, Error> {
        self.device_scope(product, &[])?;
        let mut tx = self.actor_tx(a).await?;
        let r=sqlx::query("SELECT services,can_review,policy_revision FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3").bind(&a.org).bind(product).bind(&a.user).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let restrictions = Self::product_restrictions(&mut tx, a, product).await?;
        Ok(Policy {
            product: product.into(),
            restrictions,
            features: vec!["card_provenance".into()],
            services: if let Some(g) = &self.device_guard {
                g.services.clone()
            } else {
                r.try_get("services").map_err(db)?
            },
            can_review: r.try_get("can_review").map_err(db)?,
            policy_revision: r.try_get("policy_revision").map_err(db)?,
        })
    }
    pub async fn identity(&self, issuer: &str, subject: &str) -> Result<Actor, Error> {
        let r = sqlx::query("SELECT org,user_id FROM identities WHERE issuer=$1 AND subject=$2")
            .bind(issuer)
            .bind(subject)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?
            .ok_or(Error::Denied)?;
        Ok(Actor {
            org: r.try_get("org").map_err(db)?,
            user: r.try_get("user_id").map_err(db)?,
        })
    }
    pub async fn provision_identity(
        &self,
        issuer: &str,
        subject: &str,
        a: &Actor,
    ) -> Result<(), Error> {
        if issuer.is_empty() || subject.is_empty() || a.org.is_empty() || a.user.is_empty() {
            return Err(Error::Invalid);
        }
        sqlx::query("INSERT INTO identities(issuer,subject,org,user_id) VALUES($1,$2,$3,$4) ON CONFLICT(issuer,subject) DO UPDATE SET org=excluded.org,user_id=excluded.user_id").bind(issuer).bind(subject).bind(&a.org).bind(&a.user).execute(&self.pool).await.map_err(db)?;
        Ok(())
    }
    pub async fn bind_workspace(
        &self,
        a: &Actor,
        product: &str,
        device: &str,
        approval: &str,
        services: &[String],
        revision: i64,
    ) -> Result<uuid::Uuid, Error> {
        let mut tx = self.code_tx(a, product).await?;
        if !wk_contracts::valid_id(device)
            || approval.len() != 64
            || !approval.bytes().all(|b| b.is_ascii_hexdigit())
            || services.is_empty()
        {
            return Err(Error::Invalid);
        }
        let id = uuid::Uuid::new_v4();
        let r=sqlx::query("INSERT INTO workspace_bindings(id,org,product,user_id,device_id,approval_hash,services,policy_revision) SELECT $1,$2,$3,$4,$5,$6,$7,$8 FROM memberships WHERE org=$2 AND product=$3 AND user_id=$4 AND services @> $7::text[] AND policy_revision=$8 ON CONFLICT(org,product,user_id,device_id,approval_hash) DO UPDATE SET services=excluded.services,policy_revision=excluded.policy_revision,revoked=FALSE RETURNING id").bind(id).bind(&a.org).bind(product).bind(&a.user).bind(device).bind(approval).bind(services).bind(revision).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::Denied)?;
        let id = r.try_get("id").map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(id)
    }
    pub async fn review_queue(&self, a: &Actor, product: &str) -> Result<Vec<Card>, Error> {
        let mut tx = self.code_tx(a, product).await?;
        let rows=sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.can_review AND m.services @> c.services AND NOT c.published AND NOT c.withdrawn ORDER BY c.id LIMIT 100").bind(&a.user).bind(&a.org).bind(product).fetch_all(&mut *tx).await.map_err(db)?;
        rows.into_iter().map(decode).collect()
    }
    pub async fn correct(
        &self,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
        body: CardSubmission,
    ) -> Result<Card, Error> {
        body.validate().map_err(|_| Error::Invalid)?;
        let v = serde_json::to_value(&body).map_err(|_| Error::Invalid)?;
        validate_shared_body(&v)?;
        let base =
            i64::try_from(body.base_revision.ok_or(Error::Invalid)?).map_err(|_| Error::Invalid)?;
        let mut tx = self.code_tx(a, product).await?;
        let r=sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND c.id=$4 AND NOT c.withdrawn AND m.services @> c.services AND m.services @> $5::text[] AND (c.author=$1 OR m.can_review) AND m.policy_revision=$6 FOR UPDATE OF c FOR SHARE OF m").bind(&a.user).bind(&a.org).bind(product).bind(id).bind(&body.service_ids).bind(i64::try_from(body.policy_revision).map_err(|_|Error::Invalid)?).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let old = decode(r)?;
        if old.revision != base {
            return Err(Error::Conflict);
        }
        // The origin label is part of what made the card publishable; a correction keeps it as is.
        if old.body.provenance != body.provenance {
            return Err(Error::Invalid);
        }
        sqlx::query("INSERT INTO card_revisions(org,product,card_id,revision,body,changed_by) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING").bind(&a.org).bind(product).bind(id).bind(base).bind(serde_json::to_value(&old.body).map_err(|_|Error::Invalid)?).bind(&a.user).execute(&mut *tx).await.map_err(db)?;
        let r=sqlx::query("UPDATE cards SET body=$1,services=$2,revision=revision+1,published=FALSE WHERE id=$3 RETURNING id,body,revision,published").bind(&v).bind(&body.service_ids).bind(id).fetch_one(&mut *tx).await.map_err(db)?;
        sqlx::query("INSERT INTO card_revisions(org,product,card_id,revision,body,changed_by) VALUES($1,$2,$3,$4,$5,$6)").bind(&a.org).bind(product).bind(id).bind(base+1).bind(v).bind(&a.user).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("DELETE FROM graph_nodes WHERE org=$1 AND product=$2 AND card_id=$3")
            .bind(&a.org)
            .bind(product)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query(
            "UPDATE trusted_evidence SET stale=TRUE WHERE org=$1 AND product=$2 AND card_id=$3",
        )
        .bind(&a.org)
        .bind(product)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)?;
        decode(r)
    }
    pub async fn withdraw(
        &self,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
        revision: i64,
    ) -> Result<(), Error> {
        let mut tx = self.code_tx(a, product).await?;
        let r=sqlx::query("SELECT c.revision FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND c.id=$4 AND m.services @> c.services AND (c.author=$1 OR m.can_review) AND NOT c.withdrawn FOR UPDATE OF c FOR SHARE OF m").bind(&a.user).bind(&a.org).bind(product).bind(id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        if r.try_get::<i64, _>("revision").map_err(db)? != revision {
            return Err(Error::Conflict);
        }
        sqlx::query("INSERT INTO tombstones(org,product,card_id,revision) VALUES($1,$2,$3,$4) ON CONFLICT(org,product,card_id) DO UPDATE SET revision=GREATEST(tombstones.revision,excluded.revision)").bind(&a.org).bind(product).bind(id).bind(revision+1).execute(&mut *tx).await.map_err(db)?;
        sqlx::query(
            "UPDATE cards SET withdrawn=TRUE,published=FALSE,revision=revision+1 WHERE id=$1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("DELETE FROM graph_nodes WHERE org=$1 AND product=$2 AND card_id=$3")
            .bind(&a.org)
            .bind(product)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("INSERT INTO audit_events(org,product,user_id,action,entity_id) VALUES($1,$2,$3,'withdraw',$4)").bind(&a.org).bind(product).bind(&a.user).bind(id).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }
    pub async fn replay_tombstones(&self) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("UPDATE cards c SET withdrawn=TRUE,published=FALSE,revision=GREATEST(c.revision,t.revision) FROM tombstones t WHERE c.org=t.org AND c.product=t.product AND c.id=t.card_id").execute(&mut *tx).await.map_err(db)?;
        sqlx::query("DELETE FROM graph_nodes n USING tombstones t WHERE n.org=t.org AND n.product=t.product AND n.card_id=t.card_id").execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }
    pub async fn feedback(
        &self,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
        revision: i64,
        issue: &str,
        suggestion: &str,
    ) -> Result<uuid::Uuid, Error> {
        let mut tx = self.actor_tx(a).await?;
        if issue.trim().is_empty() || issue.len() > 2048 || suggestion.len() > 8192 {
            return Err(Error::Invalid);
        }
        let card = self.get(a, product, id).await?;
        if card.revision != revision {
            return Err(Error::Conflict);
        }
        let fid = uuid::Uuid::new_v4();
        // The insertion rechecks access and revision atomically after the explanatory check.
        let count=sqlx::query("INSERT INTO feedback(id,org,product,card_id,revision,user_id,issue,suggestion) SELECT $1,c.org,c.product,c.id,c.revision,$2,$3,$4 FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$2 WHERE c.org=$5 AND c.product=$6 AND c.id=$7 AND c.revision=$8 AND NOT c.withdrawn AND m.services @> c.services AND (c.published OR c.author=$2)").bind(fid).bind(&a.user).bind(issue).bind(suggestion).bind(&a.org).bind(product).bind(id).bind(revision).execute(&mut *tx).await.map_err(db)?.rows_affected();
        if count == 0 {
            return Err(Error::Conflict);
        }
        tx.commit().await.map_err(db)?;
        Ok(fid)
    }
    pub async fn revisions(
        &self,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
    ) -> Result<Vec<Value>, Error> {
        let mut tx = self.actor_tx(a).await?;
        self.get(a, product, id).await?;
        // Historical scopes may be broader than current scopes, so filter each snapshot.
        let rows=sqlx::query("SELECT r.revision,r.body FROM card_revisions r JOIN cards c ON c.id=r.card_id JOIN memberships m ON m.org=r.org AND m.product=r.product AND m.user_id=$1 WHERE r.org=$2 AND r.product=$3 AND r.card_id=$4 AND NOT c.withdrawn AND (c.published OR c.author=$1 OR m.can_review) AND m.services @> c.services AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements_text(r.body->'service_ids') s WHERE NOT s=ANY(m.services)) ORDER BY r.revision DESC LIMIT 100").bind(&a.user).bind(&a.org).bind(product).bind(id).fetch_all(&mut *tx).await.map_err(db)?;
        rows.into_iter().map(|r|Ok(json!({"revision":r.try_get::<i64,_>("revision").map_err(db)?,"body":r.try_get::<Value,_>("body").map_err(db)?}))).collect()
    }
}

/// Effective server membership projection, not a client-authorized policy update.
pub fn policy_schema() -> serde_json::Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(Policy)).expect("schema serialization");
    schema["$id"] = json!("urn:workspace-knowledge:effective-policy:v1");
    schema
}
