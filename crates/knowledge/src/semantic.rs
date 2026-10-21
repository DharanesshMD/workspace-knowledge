//! Optional exact vector retrieval over current, published, SQL-permitted cards.
use crate::*;
use std::collections::HashMap;
pub const FORMAT: &str = "nomic-search-v1";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Namespace {
    pub model: String,
    pub digest: String,
    pub dimensions: usize,
}
impl Namespace {
    pub fn validate(&self) -> Result<(), Error> {
        if self.model.is_empty()
            || self.model.len() > 128
            || !self
                .model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
            || self.digest.len() != 64
            || !self
                .digest
                .bytes()
                .all(|b| b.is_ascii_digit() || b"abcdef".contains(&b))
            || !(1..=2048).contains(&self.dimensions)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn vector(&self, vector: &[f32]) -> Result<String, Error> {
        self.validate()?;
        let norm = vector.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        if vector.len() != self.dimensions
            || vector.iter().any(|x| !x.is_finite())
            || !norm.is_finite()
            || norm <= 0.
            || norm > 1e12
        {
            return Err(Error::Invalid);
        }
        serde_json::to_string(vector).map_err(|_| Error::Invalid)
    }
}
#[derive(Debug)]
pub struct Pending {
    pub id: uuid::Uuid,
    pub revision: i64,
    pub text: String,
}
fn document(body: &wk_contracts::CardSubmission) -> String {
    [&body.title, &body.intent, &body.rationale]
        .into_iter()
        .cloned()
        .chain(body.decisions.iter().cloned())
        .chain(body.constraints.iter().cloned())
        .chain(body.open_questions.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n")
}
impl Knowledge {
    async fn semantic_owner(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<(), Error> {
        let allowed:bool=sqlx::query_scalar("SELECT pg_has_role(current_user,relowner,'USAGE') FROM pg_class WHERE oid='public.cards'::regclass")
            .fetch_one(&mut **tx).await.map_err(db)?;
        if self.device_guard.is_some() || !allowed {
            return Err(Error::Denied);
        }
        Ok(())
    }
    /// Owner-only inference destination check for a scoped indexing pass.
    pub async fn semantic_index_allowed(&self, org: &str, product: &str) -> Result<(), Error> {
        if org.is_empty() || org.len() > 256 || !wk_contracts::valid_id(product) {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        self.semantic_owner(&mut tx).await?;
        let actor = Actor {
            org: org.into(),
            user: String::new(),
        };
        Self::lock_administration(&mut tx, &actor, product).await?;
        if !Self::product_restrictions(&mut tx, &actor, product)
            .await?
            .allow_loopback
        {
            return Err(Error::Denied);
        }
        tx.commit().await.map_err(db)
    }
    pub async fn semantic_migrate(&self) -> Result<(), Error> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        self.semantic_owner(&mut tx).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(733096)")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS wk_semantic_migrations(version INTEGER PRIMARY KEY)",
        )
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        let installed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM wk_semantic_migrations WHERE version=1)",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        if installed {
            return tx.commit().await.map_err(db);
        }
        sqlx::raw_sql(include_str!("../../../migrations/semantic/0001.sql"))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("INSERT INTO wk_semantic_migrations(version) VALUES(1)")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)
    }
    pub async fn semantic_pending(
        &self,
        org: &str,
        product: &str,
        ns: &Namespace,
    ) -> Result<Vec<Pending>, Error> {
        ns.validate()?;
        if org.is_empty() || org.len() > 256 || !wk_contracts::valid_id(product) {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        self.semantic_owner(&mut tx).await?;
        sqlx::query("DELETE FROM card_embeddings e USING cards c WHERE e.org=$1 AND e.product=$2 AND c.org=e.org AND c.product=e.product AND c.id=e.card_id AND (e.model<>$3 OR e.digest<>$4 OR e.dimensions<>$5 OR e.format<>$6 OR e.revision<>c.revision OR NOT c.published OR c.withdrawn OR c.body->>'lifecycle' IN ('superseded','deprecated'))")
            .bind(org).bind(product).bind(&ns.model).bind(&ns.digest).bind(ns.dimensions as i32).bind(FORMAT).execute(&mut *tx).await.map_err(db)?;
        let rows=sqlx::query("SELECT c.id,c.revision,c.body FROM cards c WHERE c.org=$1 AND c.product=$2 AND c.published AND NOT c.withdrawn AND c.body->>'lifecycle' NOT IN ('superseded','deprecated') AND NOT EXISTS(SELECT 1 FROM card_embeddings e WHERE e.org=c.org AND e.product=c.product AND e.card_id=c.id AND e.revision=c.revision AND e.model=$3 AND e.digest=$4 AND e.dimensions=$5 AND e.format=$6 AND (e.status='ready' OR e.retry_at>now())) ORDER BY c.id LIMIT 20")
            .bind(org).bind(product).bind(&ns.model).bind(&ns.digest).bind(ns.dimensions as i32).bind(FORMAT).fetch_all(&mut *tx).await.map_err(db)?;
        let mut pending = vec![];
        for row in rows {
            let body = serde_json::from_value(row.try_get("body").map_err(db)?)
                .map_err(|_| Error::Storage)?;
            pending.push(Pending {
                id: row.try_get("id").map_err(db)?,
                revision: row.try_get("revision").map_err(db)?,
                text: document(&body),
            });
        }
        tx.commit().await.map_err(db)?;
        Ok(pending)
    }
    pub async fn semantic_store(
        &self,
        org: &str,
        product: &str,
        ns: &Namespace,
        pending: &Pending,
        vector: &[f32],
    ) -> Result<bool, Error> {
        self.semantic_write(org, product, ns, pending, Some(ns.vector(vector)?))
            .await
    }
    pub async fn semantic_failed(
        &self,
        org: &str,
        product: &str,
        ns: &Namespace,
        pending: &Pending,
    ) -> Result<bool, Error> {
        self.semantic_write(org, product, ns, pending, None).await
    }
    async fn semantic_write(
        &self,
        org: &str,
        product: &str,
        ns: &Namespace,
        pending: &Pending,
        vector: Option<String>,
    ) -> Result<bool, Error> {
        ns.validate()?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        self.semantic_owner(&mut tx).await?;
        let actor = Actor {
            org: org.into(),
            user: String::new(),
        };
        Self::lock_administration(&mut tx, &actor, product).await?;
        if !Self::product_restrictions(&mut tx, &actor, product)
            .await?
            .allow_loopback
        {
            return Ok(false);
        }
        let row=sqlx::query("SELECT body FROM cards WHERE org=$1 AND product=$2 AND id=$3 AND revision=$4 AND published AND NOT withdrawn AND body->>'lifecycle' NOT IN ('superseded','deprecated') FOR SHARE")
            .bind(org).bind(product).bind(pending.id).bind(pending.revision).fetch_optional(&mut *tx).await.map_err(db)?;
        let Some(row) = row else { return Ok(false) };
        let body =
            serde_json::from_value(row.try_get("body").map_err(db)?).map_err(|_| Error::Storage)?;
        if document(&body) != pending.text {
            return Ok(false);
        }
        if vector.is_some()
            && (pending.text.len() > 16384 || wk_summarize::sensitive(&pending.text))
        {
            return Err(Error::Invalid);
        }
        let status = if vector.is_some() { "ready" } else { "held" };
        sqlx::query("INSERT INTO card_embeddings(org,product,card_id,revision,model,digest,dimensions,format,embedding,status,retry_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9::vector,$10,now()+interval '300 seconds') ON CONFLICT(org,product,card_id,model,digest,dimensions,format) DO UPDATE SET revision=EXCLUDED.revision,embedding=EXCLUDED.embedding,status=EXCLUDED.status,retry_at=EXCLUDED.retry_at")
            .bind(org).bind(product).bind(pending.id).bind(pending.revision).bind(&ns.model).bind(&ns.digest).bind(ns.dimensions as i32).bind(FORMAT).bind(vector).bind(status).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(true)
    }
    async fn semantic_services(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
    ) -> Result<Vec<String>, Error> {
        self.device_scope(p, &[])?;
        if !Self::product_restrictions(tx, a, p).await?.allow_loopback {
            return Err(Error::Denied);
        }
        let row = sqlx::query(
            "SELECT services FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 FOR SHARE",
        )
        .bind(&a.org)
        .bind(p)
        .bind(&a.user)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db)?
        .ok_or(Error::NotFound)?;
        let services: Vec<String> = row.try_get("services").map_err(db)?;
        Ok(self
            .device_guard
            .as_ref()
            .map(|g| g.services.clone())
            .unwrap_or(services))
    }
    pub async fn semantic_ready(&self, a: &Actor, p: &str, ns: &Namespace) -> Result<bool, Error> {
        ns.validate()?;
        let mut tx = self.read_tx(a, p).await?;
        let services = self.semantic_services(&mut tx, a, p).await?;
        let exists: bool =
            sqlx::query_scalar("SELECT to_regclass('public.card_embeddings') IS NOT NULL")
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if !exists {
            return Ok(false);
        }
        let ready:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM card_embeddings e JOIN cards c ON c.org=e.org AND c.product=e.product AND c.id=e.card_id AND c.revision=e.revision JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.services @> c.services AND $4::text[] @> c.services AND c.published AND NOT c.withdrawn AND c.body->>'lifecycle' NOT IN ('superseded','deprecated') AND e.model=$5 AND e.digest=$6 AND e.dimensions=$7 AND e.format=$8 AND e.status='ready')")
            .bind(&a.user).bind(&a.org).bind(p).bind(&services).bind(&ns.model).bind(&ns.digest).bind(ns.dimensions as i32).bind(FORMAT).fetch_one(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(ready)
    }
    pub async fn semantic_candidates(
        &self,
        a: &Actor,
        p: &str,
        ns: &Namespace,
        vector: &[f32],
        limit: u32,
    ) -> Result<Vec<Card>, Error> {
        if !(1..=20).contains(&limit) {
            return Err(Error::Invalid);
        }
        let encoded = ns.vector(vector)?;
        let mut tx = self.read_tx(a, p).await?;
        let services = self.semantic_services(&mut tx, a, p).await?;
        let cards = vector_candidates(&mut tx, a, p, &services, ns, &encoded, limit).await?;
        tx.commit().await.map_err(db)?;
        Ok(cards)
    }
    pub async fn search_hybrid(
        &self,
        a: &Actor,
        p: &str,
        query: &str,
        limit: u32,
        ns: &Namespace,
        vector: &[f32],
    ) -> Result<Vec<Card>, Error> {
        if query.trim().is_empty() || query.len() > 1024 || !(1..=20).contains(&limit) {
            return Err(Error::Invalid);
        }
        let encoded = ns.vector(vector)?;
        let mut tx = self.read_tx(a, p).await?;
        let services = self.semantic_services(&mut tx, a, p).await?;
        let semantic = vector_candidates(&mut tx, a, p, &services, ns, &encoded, 20).await?;
        let rows=sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.services @> c.services AND $4::text[] @> c.services AND c.published AND NOT c.withdrawn AND c.body->>'lifecycle' NOT IN ('superseded','deprecated') AND c.search_vector @@ plainto_tsquery('simple',$5) ORDER BY ts_rank_cd(c.search_vector,plainto_tsquery('simple',$5)) DESC,c.id LIMIT 20")
            .bind(&a.user).bind(&a.org).bind(p).bind(&services).bind(query).fetch_all(&mut *tx).await.map_err(db)?;
        let lexical = rows
            .into_iter()
            .map(decode)
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(db)?;
        let mut combined: HashMap<uuid::Uuid, (Card, f64)> = HashMap::new();
        for list in [lexical, semantic] {
            for (rank, card) in list.into_iter().enumerate() {
                let score = 1. / (60. + rank as f64 + 1.);
                combined
                    .entry(card.id)
                    .and_modify(|(_, s)| *s += score)
                    .or_insert((card, score));
            }
        }
        let mut results = combined.into_values().collect::<Vec<_>>();
        results.sort_by(|(a, x), (b, y)| y.total_cmp(x).then_with(|| a.id.cmp(&b.id)));
        Ok(results
            .into_iter()
            .take(limit as usize)
            .map(|(c, _)| c)
            .collect())
    }
    /// Published summaries by other developers that match `vector` by meaning,
    /// within the caller's services, excluding `exclude`, with cosine similarity of at least
    /// `min_similarity` (0.3 to 0.95). Its scoping clauses must stay in sync with
    /// `vector_candidates`, which keeps its own 0.6 floor for search.
    #[allow(clippy::too_many_arguments)]
    pub async fn related_by_meaning(
        &self,
        a: &Actor,
        p: &str,
        ns: &Namespace,
        vector: &[f32],
        exclude: &[uuid::Uuid],
        limit: u32,
        min_similarity: f32,
    ) -> Result<Vec<crate::activity::Notice>, Error> {
        if !(1..=20).contains(&limit)
            || exclude.len() > 100
            || !(0.3..=0.95).contains(&min_similarity)
        {
            return Err(Error::Invalid);
        }
        let encoded = ns.vector(vector)?;
        let mut tx = self.read_tx(a, p).await?;
        let services = self.semantic_services(&mut tx, a, p).await?;
        let now_ms: i64 = sqlx::query_scalar("SELECT (extract(epoch from now())*1000)::bigint")
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        let rows=sqlx::query("SELECT c.id,c.revision,c.author,c.body,(extract(epoch from c.created_at)*1000)::bigint AS at FROM card_embeddings e JOIN cards c ON c.org=e.org AND c.product=e.product AND c.id=e.card_id AND c.revision=e.revision JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.services @> c.services AND $4::text[] @> c.services AND c.author<>$1 AND NOT (c.id = ANY($11::uuid[])) AND c.published AND NOT c.withdrawn AND c.body->>'lifecycle' NOT IN ('superseded','deprecated') AND e.model=$5 AND e.digest=$6 AND e.dimensions=$7 AND e.format=$8 AND e.status='ready' AND (e.embedding <=> $9::vector)<=1-$12::float8 ORDER BY e.embedding <=> $9::vector,c.id LIMIT $10")
            .bind(&a.user).bind(&a.org).bind(p).bind(&services).bind(&ns.model).bind(&ns.digest).bind(ns.dimensions as i32).bind(FORMAT).bind(&encoded).bind(i64::from(limit)).bind(exclude).bind(f64::from(min_similarity)).fetch_all(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)?;
        rows.into_iter()
            .map(|r| {
                let body: serde_json::Value = r.try_get("body").map_err(db)?;
                Ok(crate::activity::card_notice(
                    "related",
                    "meaning",
                    r.try_get("author").map_err(db)?,
                    r.try_get("id").map_err(db)?,
                    r.try_get("revision").map_err(db)?,
                    &body,
                    vec![],
                    r.try_get("at").map_err(db)?,
                    now_ms,
                ))
            })
            .collect()
    }
}
// Its scoping clauses must stay in sync with `Knowledge::related_by_meaning`.
async fn vector_candidates(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    a: &Actor,
    p: &str,
    services: &[String],
    ns: &Namespace,
    vector: &str,
    limit: u32,
) -> Result<Vec<Card>, Error> {
    let rows=sqlx::query("SELECT c.id,c.body,c.revision,c.published FROM card_embeddings e JOIN cards c ON c.org=e.org AND c.product=e.product AND c.id=e.card_id AND c.revision=e.revision JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.services @> c.services AND $4::text[] @> c.services AND c.published AND NOT c.withdrawn AND c.body->>'lifecycle' NOT IN ('superseded','deprecated') AND e.model=$5 AND e.digest=$6 AND e.dimensions=$7 AND e.format=$8 AND e.status='ready' AND (e.embedding <=> $9::vector)<=0.4 ORDER BY e.embedding <=> $9::vector,c.id LIMIT $10")
        .bind(&a.user).bind(&a.org).bind(p).bind(services).bind(&ns.model).bind(&ns.digest).bind(ns.dimensions as i32).bind(FORMAT).bind(vector).bind(i64::from(limit)).fetch_all(&mut **tx).await.map_err(db)?;
    rows.into_iter().map(decode).collect()
}
