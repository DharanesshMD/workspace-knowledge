//! Sanitized provenance only. Never contains a source-message retrieval route.
use super::*;
use serde_json::{json, Value};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source_reference: String,
    pub observed_revision: String,
    pub source_boundary: String,
    pub capture_mode: String,
    pub coverage: String,
    pub missing_context: Vec<String>,
    pub extraction_version: String,
}
impl Provenance {
    pub fn validate(&self) -> Result<(), Error> {
        if ![
            "hook",
            "compliance",
            "instrumented_chat",
            "export",
            "browser_observed",
            "cooperative",
        ]
        .contains(&self.capture_mode.as_str())
            || !["complete_within_boundary", "partial", "unknown"].contains(&self.coverage.as_str())
            || self.source_reference.is_empty()
            || self.source_reference.len() > 256
            || self.source_boundary.len() > 256
            || self.observed_revision.is_empty()
            || self.extraction_version.is_empty()
            || self.missing_context.len() > 20
            || (self.coverage == "complete_within_boundary"
                && (!self.missing_context.is_empty() || self.capture_mode == "browser_observed"))
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
impl Knowledge {
    pub async fn graph(&self, a: &Actor, product: &str) -> Result<Value, Error> {
        let mut tx = self.actor_tx(a).await?;
        // Filter every node before edges. A hidden intermediate never bridges visible endpoints.
        let rows=sqlx::query("SELECT n.id,n.card_id,n.kind,n.label,n.provenance FROM graph_nodes n JOIN cards c ON c.org=n.org AND c.product=n.product AND c.id=n.card_id JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE n.org=$2 AND n.product=$3 AND m.services @> c.services AND c.published AND NOT c.withdrawn ORDER BY n.id LIMIT 500").bind(&a.user).bind(&a.org).bind(product).fetch_all(&mut *tx).await.map_err(db)?;
        let mut ids = Vec::new();
        let mut nodes = Vec::new();
        for r in rows {
            let id: uuid::Uuid = r.try_get("id").map_err(db)?;
            ids.push(id);
            nodes.push(json!({"id":id,"card_id":r.try_get::<uuid::Uuid,_>("card_id").map_err(db)?,"kind":r.try_get::<String,_>("kind").map_err(db)?,"label":r.try_get::<String,_>("label").map_err(db)?,"provenance":r.try_get::<Value,_>("provenance").map_err(db)?}));
        }
        let rows=sqlx::query("SELECT source,target,kind FROM graph_edges WHERE org=$1 AND product=$2 AND source=ANY($3) AND target=ANY($3) ORDER BY source,target LIMIT 1000").bind(&a.org).bind(product).bind(ids).fetch_all(&mut *tx).await.map_err(db)?;
        let edges:Result<Vec<_>,Error>=rows.into_iter().map(|r|Ok(json!({"source":r.try_get::<uuid::Uuid,_>("source").map_err(db)?,"target":r.try_get::<uuid::Uuid,_>("target").map_err(db)?,"kind":r.try_get::<String,_>("kind").map_err(db)?}))).collect();
        Ok(json!({"nodes":nodes,"edges":edges?,"limit":500}))
    }
    pub async fn reconcile_graph(
        &self,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
        revision: i64,
        p: Provenance,
    ) -> Result<(), Error> {
        p.validate()?;
        let mut tx = self.actor_tx(a).await?;
        let row=sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND c.id=$4 AND NOT c.withdrawn AND m.services @> c.services AND (c.author=$1 OR m.can_review) FOR UPDATE OF c FOR SHARE OF m").bind(&a.user).bind(&a.org).bind(product).bind(id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let c = decode(row)?;
        if c.revision != revision {
            return Err(Error::Conflict);
        }
        Self::write_graph(&mut tx, a, product, id, revision, &c, p).await?;
        tx.commit().await.map_err(db)
    }
    pub(crate) async fn write_graph(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        product: &str,
        id: uuid::Uuid,
        revision: i64,
        c: &Card,
        p: Provenance,
    ) -> Result<(), Error> {
        let mut provenance = serde_json::to_value(p).map_err(|_| Error::Invalid)?;
        provenance["verification"] =
            json!(if c.body.lifecycle == wk_contracts::Lifecycle::Verified {
                "tests_verified"
            } else {
                "reported"
            });
        provenance["card_revision"] = json!(revision);
        // A same-revision refresh preserves node identities and reviewed links.
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM graph_nodes WHERE org=$1 AND product=$2 AND card_id=$3",
        )
        .bind(&a.org)
        .bind(product)
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map_err(db)?;
        if count > 0 {
            sqlx::query(
                "UPDATE graph_nodes SET provenance=$1 WHERE org=$2 AND product=$3 AND card_id=$4",
            )
            .bind(&provenance)
            .bind(&a.org)
            .bind(product)
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(db)?;
            return Ok(());
        }
        let mut items = vec![
            ("product", product.to_string()),
            ("feature", c.body.title.clone()),
            ("claim", c.body.intent.clone()),
        ];
        items.extend(c.body.service_ids.iter().cloned().map(|s| ("service", s)));
        items.extend(c.body.decisions.iter().cloned().map(|s| ("decision", s)));
        items.extend(
            c.body
                .evidence
                .iter()
                .map(|s| ("evidence", s.reference.clone())),
        );
        let feature = uuid::Uuid::new_v4();
        for (kind, label) in items {
            let nid = if kind == "feature" {
                feature
            } else {
                uuid::Uuid::new_v4()
            };
            sqlx::query("INSERT INTO graph_nodes(id,org,product,card_id,kind,label,provenance) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(nid).bind(&a.org).bind(product).bind(id).bind(kind).bind(label).bind(&provenance).execute(&mut **tx).await.map_err(db)?;
        }
        sqlx::query("INSERT INTO graph_edges(org,product,source,target,kind) SELECT org,product,id,$4,CASE WHEN kind='evidence' THEN 'supports' WHEN kind='service' THEN 'affects' ELSE 'belongs_to' END FROM graph_nodes WHERE org=$1 AND product=$2 AND card_id=$3 AND id<>$4").bind(&a.org).bind(product).bind(id).bind(feature).execute(&mut **tx).await.map_err(db)?;
        Ok(())
    }
    pub async fn link_graph(
        &self,
        a: &Actor,
        product: &str,
        source: uuid::Uuid,
        target: uuid::Uuid,
        kind: &str,
    ) -> Result<(), Error> {
        let mut tx = self.actor_tx(a).await?;
        if ![
            "belongs_to",
            "affects",
            "implements",
            "depends_on",
            "supports",
            "contradicts",
            "supersedes",
        ]
        .contains(&kind)
        {
            return Err(Error::Invalid);
        }
        let result=sqlx::query("INSERT INTO graph_edges(org,product,source,target,kind) SELECT $1,$2,$3,$4,$5 WHERE 2=(SELECT COUNT(*) FROM graph_nodes n JOIN cards c ON c.id=n.card_id JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$6 WHERE n.org=$1 AND n.product=$2 AND n.id IN ($3,$4) AND NOT c.withdrawn AND m.can_review AND m.services @> c.services) ON CONFLICT DO NOTHING").bind(&a.org).bind(product).bind(source).bind(target).bind(kind).bind(&a.user).execute(&mut *tx).await.map_err(db)?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound);
        }
        tx.commit().await.map_err(db)?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardRelationship {
    pub source_card: uuid::Uuid,
    pub target_card: uuid::Uuid,
    pub source_revision: i64,
    pub target_revision: i64,
    pub kind: String,
}
impl Knowledge {
    /// A reviewer must see both current published revisions. Supersession keeps
    /// the old revision readable while removing it from ordinary retrieval.
    pub async fn relate_cards(
        &self,
        a: &Actor,
        product: &str,
        r: CardRelationship,
    ) -> Result<(), Error> {
        if r.source_card == r.target_card
            || !["contradicts", "depends_on", "supersedes"].contains(&r.kind.as_str())
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        let rows = sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND c.id=ANY($4) AND c.published AND NOT c.withdrawn AND m.can_review AND m.services @> c.services ORDER BY c.id FOR UPDATE OF c FOR SHARE OF m")
            .bind(&a.user).bind(&a.org).bind(product).bind(vec![r.source_card,r.target_card]).fetch_all(&mut *tx).await.map_err(db)?;
        if rows.len() != 2 {
            return Err(Error::NotFound);
        }
        let cards = rows
            .into_iter()
            .map(decode)
            .collect::<Result<Vec<_>, _>>()?;
        let source = cards
            .iter()
            .find(|c| c.id == r.source_card)
            .ok_or(Error::NotFound)?;
        let target = cards
            .iter()
            .find(|c| c.id == r.target_card)
            .ok_or(Error::NotFound)?;
        if source.revision != r.source_revision || target.revision != r.target_revision {
            return Err(Error::Conflict);
        }
        if matches!(
            source.body.lifecycle,
            wk_contracts::Lifecycle::Superseded | wk_contracts::Lifecycle::Deprecated
        ) || matches!(
            target.body.lifecycle,
            wk_contracts::Lifecycle::Superseded | wk_contracts::Lifecycle::Deprecated
        ) {
            return Err(Error::Conflict);
        }
        let nodes = sqlx::query("SELECT id,card_id FROM graph_nodes WHERE org=$1 AND product=$2 AND card_id=ANY($3) AND kind='feature'")
            .bind(&a.org).bind(product).bind(vec![r.source_card,r.target_card]).fetch_all(&mut *tx).await.map_err(db)?;
        let node = |id| -> Result<uuid::Uuid, Error> {
            let row = nodes
                .iter()
                .find(|n| n.try_get::<uuid::Uuid, _>("card_id").ok() == Some(id))
                .ok_or(Error::Conflict)?;
            row.try_get("id").map_err(db)
        };
        sqlx::query("INSERT INTO graph_edges(org,product,source,target,kind) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING")
            .bind(&a.org).bind(product).bind(node(r.source_card)?).bind(node(r.target_card)?).bind(&r.kind).execute(&mut *tx).await.map_err(db)?;
        if r.kind == "supersedes" {
            let revision = target.revision.checked_add(1).ok_or(Error::Conflict)?;
            let mut body = serde_json::to_value(&target.body).map_err(|_| Error::Invalid)?;
            body["lifecycle"] = json!("superseded");
            sqlx::query(
                "UPDATE cards SET body=$1,revision=$2 WHERE org=$3 AND product=$4 AND id=$5",
            )
            .bind(&body)
            .bind(revision)
            .bind(&a.org)
            .bind(product)
            .bind(target.id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            sqlx::query("INSERT INTO card_revisions(org,product,card_id,revision,body,changed_by) VALUES($1,$2,$3,$4,$5,$6)")
                .bind(&a.org).bind(product).bind(target.id).bind(revision).bind(body).bind(&a.user).execute(&mut *tx).await.map_err(db)?;
            sqlx::query("UPDATE graph_nodes SET provenance=jsonb_set(jsonb_set(provenance,'{card_revision}',to_jsonb($1::bigint)),'{verification}','\"stale\"'::jsonb) WHERE org=$2 AND product=$3 AND card_id=$4")
                .bind(revision).bind(&a.org).bind(product).bind(target.id).execute(&mut *tx).await.map_err(db)?;
            sqlx::query(
                "UPDATE trusted_evidence SET stale=TRUE WHERE org=$1 AND product=$2 AND card_id=$3",
            )
            .bind(&a.org)
            .bind(product)
            .bind(target.id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
        sqlx::query(
            "INSERT INTO audit_events(org,product,user_id,action,entity_id) VALUES($1,$2,$3,$4,$5)",
        )
        .bind(&a.org)
        .bind(product)
        .bind(&a.user)
        .bind(format!("relationship:{}", r.kind))
        .bind(r.source_card)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)
    }
}
