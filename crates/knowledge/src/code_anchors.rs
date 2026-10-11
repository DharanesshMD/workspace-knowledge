//! Human-confirmed contribution and rationale bindings, separate from code/test verification.
use super::*;
use crate::code_provenance::{decode_value, scopes, MappingRecord, RepositoryRecord};
use wk_contracts::change_context::{valid_revision, valid_targets, CodeTarget};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContributionInput {
    pub repository_id: uuid::Uuid,
    pub mapping_id: uuid::Uuid,
    pub code_revision: String,
    pub targets: Vec<CodeTarget>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ContributionRecord {
    pub id: uuid::Uuid,
    pub repository_id: uuid::Uuid,
    pub repository_revision: i64,
    pub mapping_id: uuid::Uuid,
    pub mapping_revision: i64,
    pub code_revision: String,
    pub targets: Vec<CodeTarget>,
    pub services: Vec<String>,
    pub owner: String,
    pub submitted_by: String,
    pub revision: i64,
    pub confirmed_by: Option<String>,
    pub confirmed_at_ms: Option<i64>,
    pub reviewed_by: Option<String>,
    pub reviewed_at_ms: Option<i64>,
    pub revoked: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorInput {
    pub repository_id: uuid::Uuid,
    pub card_id: uuid::Uuid,
    pub card_revision: i64,
    pub code_revision: String,
    pub targets: Vec<CodeTarget>,
    pub contribution_id: Option<uuid::Uuid>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnchorRecord {
    pub id: uuid::Uuid,
    pub repository_id: uuid::Uuid,
    pub repository_revision: i64,
    pub card_id: uuid::Uuid,
    pub card_revision: i64,
    pub code_revision: String,
    pub contribution_id: Option<uuid::Uuid>,
    pub contribution_revision: Option<i64>,
    pub targets: Vec<CodeTarget>,
    pub services: Vec<String>,
    pub owner: String,
    pub submitted_by: String,
    pub revision: i64,
    pub confirmed_by: Option<String>,
    pub confirmed_at_ms: Option<i64>,
    pub reviewed_by: Option<String>,
    pub reviewed_at_ms: Option<i64>,
    pub revoked: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaintainerInput {
    pub repository_id: uuid::Uuid,
    pub user_id: String,
    pub targets: Vec<CodeTarget>,
    pub services: Vec<String>,
    pub base_revision: Option<i64>,
}
pub(crate) fn union_scopes(left: &[String], right: &[String]) -> Vec<String> {
    left.iter()
        .chain(right)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub(crate) fn covers(a: &CodeTarget, b: &CodeTarget) -> bool {
    a.path == b.path
        && match (a.start, a.end, b.start, b.end) {
            (None, None, _, _) => true,
            (Some(x), Some(y), Some(v), Some(w)) => x <= v && w <= y,
            _ => false,
        }
}
impl Knowledge {
    pub(crate) async fn code_repository(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<RepositoryRecord, Error> {
        let v:Option<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(r)-'org'-'product' FROM code_repositories r JOIN memberships m ON m.org=r.org AND m.product=r.product AND m.user_id=$3 WHERE r.org=$1 AND r.product=$2 AND r.id=$4 AND NOT r.revoked AND m.services @> r.services FOR SHARE OF r,m")
            .bind(&a.org).bind(p).bind(&a.user).bind(id).fetch_optional(&mut **tx).await.map_err(db)?;
        decode_value(v.ok_or(Error::NotFound)?)
    }
    async fn code_mapping(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<MappingRecord, Error> {
        let v:Option<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(cm)-'org'-'product' FROM code_mappings cm JOIN memberships m ON m.org=cm.org AND m.product=cm.product AND m.user_id=$3 WHERE cm.org=$1 AND cm.product=$2 AND cm.id=$4 AND NOT cm.revoked AND m.services @> cm.services FOR SHARE OF cm,m")
            .bind(&a.org).bind(p).bind(&a.user).bind(id).fetch_optional(&mut **tx).await.map_err(db)?;
        decode_value(v.ok_or(Error::NotFound)?)
    }
    pub(crate) async fn code_contribution(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<ContributionRecord, Error> {
        let v:Option<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(c)-'org'-'product' FROM code_contributions c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$3 WHERE c.org=$1 AND c.product=$2 AND c.id=$4 AND NOT c.revoked AND m.services @> c.services AND (c.owner=$3 OR c.submitted_by=$3 OR m.can_review OR (c.confirmed_by=c.owner AND c.reviewed_by IS NOT NULL)) FOR UPDATE OF c FOR SHARE OF m")
            .bind(&a.org).bind(p).bind(&a.user).bind(id).fetch_optional(&mut **tx).await.map_err(db)?;
        let c: ContributionRecord = decode_value(v.ok_or(Error::NotFound)?)?;
        let repo = Self::code_repository(tx, a, p, c.repository_id).await?;
        let mapping = Self::code_mapping(tx, a, p, c.mapping_id).await?;
        if repo.revision != c.repository_revision
            || mapping.repository_id != repo.id
            || mapping.revision != c.mapping_revision
            || mapping.user_id != c.owner
        {
            return Err(Error::Conflict);
        }
        Ok(c)
    }
    pub async fn propose_contribution(
        &self,
        a: &Actor,
        p: &str,
        input: ContributionInput,
    ) -> Result<ContributionRecord, Error> {
        if !wk_contracts::valid_id(p)
            || !valid_revision(&input.code_revision)
            || !valid_targets(&input.targets)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        Self::lock_administration(&mut tx, a, p).await?;
        let repo = Self::code_repository(&mut tx, a, p, input.repository_id).await?;
        let mapping = Self::code_mapping(&mut tx, a, p, input.mapping_id).await?;
        if mapping.repository_id != repo.id {
            return Err(Error::Invalid);
        }
        let services = union_scopes(&repo.services, &mapping.services);
        Self::code_member(&mut tx, a, p, &services).await?;
        let v:serde_json::Value=sqlx::query_scalar("INSERT INTO code_contributions(org,product,id,repository_id,repository_revision,mapping_id,mapping_revision,owner,submitted_by,code_revision,targets,services) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING to_jsonb(code_contributions)-'org'-'product'")
            .bind(&a.org).bind(p).bind(uuid::Uuid::new_v4()).bind(repo.id).bind(repo.revision).bind(mapping.id).bind(mapping.revision).bind(mapping.user_id).bind(&a.user).bind(input.code_revision).bind(serde_json::to_value(input.targets).map_err(|_|Error::Invalid)?).bind(services).fetch_one(&mut *tx).await.map_err(db)?;
        let result = decode_value(v)?;
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    async fn code_reviewer(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
    ) -> Result<(), Error> {
        let ok=sqlx::query("SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 AND can_review FOR SHARE")
            .bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut **tx).await.map_err(db)?;
        ok.map(|_| ()).ok_or(Error::Denied)
    }
    pub async fn confirm_contribution(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        revision: i64,
    ) -> Result<ContributionRecord, Error> {
        self.contribution_transition(a, p, id, revision, false)
            .await
    }
    pub async fn review_contribution(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        revision: i64,
    ) -> Result<ContributionRecord, Error> {
        self.contribution_transition(a, p, id, revision, true).await
    }
    async fn contribution_transition(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        revision: i64,
        review: bool,
    ) -> Result<ContributionRecord, Error> {
        let mut tx = self.actor_tx(a).await?;
        Self::lock_administration(&mut tx, a, p).await?;
        if review {
            Self::code_reviewer(&mut tx, a, p).await?;
        }
        let c = Self::code_contribution(&mut tx, a, p, id).await?;
        if !review && c.owner != a.user {
            return Err(Error::Denied);
        }
        if c.revision != revision {
            return Err(Error::Conflict);
        }
        if review && c.confirmed_by.as_deref() != Some(&c.owner) {
            return Err(Error::Conflict);
        }
        let query = if review {
            "UPDATE code_contributions SET reviewed_by=$4,reviewed_at_ms=floor(extract(epoch FROM clock_timestamp())*1000)::bigint,revision=revision+1 WHERE org=$1 AND product=$2 AND id=$3 RETURNING to_jsonb(code_contributions)-'org'-'product'"
        } else {
            "UPDATE code_contributions SET confirmed_by=$4,confirmed_at_ms=floor(extract(epoch FROM clock_timestamp())*1000)::bigint,reviewed_by=NULL,reviewed_at_ms=NULL,revision=revision+1 WHERE org=$1 AND product=$2 AND id=$3 RETURNING to_jsonb(code_contributions)-'org'-'product'"
        };
        let v: serde_json::Value = sqlx::query_scalar(query)
            .bind(&a.org)
            .bind(p)
            .bind(id)
            .bind(&a.user)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        let result = decode_value(v)?;
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    pub(crate) async fn anchor_card(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<(Card, String), Error> {
        let row=sqlx::query("SELECT c.id,c.body,c.revision,c.published,c.author FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$3 WHERE c.org=$1 AND c.product=$2 AND c.id=$4 AND NOT c.withdrawn AND m.services @> c.services AND (c.published OR c.author=$3 OR m.can_review) FOR SHARE OF c,m")
            .bind(&a.org).bind(p).bind(&a.user).bind(id).fetch_optional(&mut **tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let owner: String = row.try_get("author").map_err(db)?;
        Ok((decode(row)?, owner))
    }
    pub async fn propose_rationale_anchor(
        &self,
        a: &Actor,
        p: &str,
        input: AnchorInput,
    ) -> Result<AnchorRecord, Error> {
        if !wk_contracts::valid_id(p)
            || input.card_revision < 1
            || !valid_revision(&input.code_revision)
            || !valid_targets(&input.targets)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        Self::lock_administration(&mut tx, a, p).await?;
        let repo = Self::code_repository(&mut tx, a, p, input.repository_id).await?;
        let (card, owner) = Self::anchor_card(&mut tx, a, p, input.card_id).await?;
        if card.revision != input.card_revision {
            return Err(Error::Conflict);
        }
        let mut services = union_scopes(&repo.services, &card.body.service_ids);
        let contribution_revision = if let Some(id) = input.contribution_id {
            let c = Self::code_contribution(&mut tx, a, p, id).await?;
            if c.repository_id != repo.id
                || c.code_revision != input.code_revision
                || c.reviewed_by.is_none()
                || c.confirmed_by.as_deref() != Some(&c.owner)
                || input
                    .targets
                    .iter()
                    .any(|t| !c.targets.iter().any(|covered| covers(covered, t)))
            {
                return Err(Error::Invalid);
            }
            services = union_scopes(&services, &c.services);
            Some(c.revision)
        } else {
            None
        };
        Self::code_member(&mut tx, a, p, &services).await?;
        let v:serde_json::Value=sqlx::query_scalar("INSERT INTO code_rationale_anchors(org,product,id,repository_id,repository_revision,card_id,card_revision,contribution_id,contribution_revision,owner,submitted_by,code_revision,targets,services) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) RETURNING to_jsonb(code_rationale_anchors)-'org'-'product'")
            .bind(&a.org).bind(p).bind(uuid::Uuid::new_v4()).bind(repo.id).bind(repo.revision).bind(card.id).bind(card.revision).bind(input.contribution_id).bind(contribution_revision).bind(owner).bind(&a.user).bind(input.code_revision).bind(serde_json::to_value(input.targets).map_err(|_|Error::Invalid)?).bind(services).fetch_one(&mut *tx).await.map_err(db)?;
        let result = decode_value(v)?;
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    pub async fn confirm_rationale_anchor(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        revision: i64,
    ) -> Result<AnchorRecord, Error> {
        self.anchor_transition(a, p, id, revision, false).await
    }
    pub async fn review_rationale_anchor(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        revision: i64,
    ) -> Result<AnchorRecord, Error> {
        self.anchor_transition(a, p, id, revision, true).await
    }
    async fn anchor_transition(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        revision: i64,
        review: bool,
    ) -> Result<AnchorRecord, Error> {
        let mut tx = self.actor_tx(a).await?;
        Self::lock_administration(&mut tx, a, p).await?;
        if review {
            Self::code_reviewer(&mut tx, a, p).await?;
        }
        let v:Option<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(ca)-'org'-'product' FROM code_rationale_anchors ca JOIN memberships m ON m.org=ca.org AND m.product=ca.product AND m.user_id=$3 WHERE ca.org=$1 AND ca.product=$2 AND ca.id=$4 AND NOT ca.revoked AND m.services @> ca.services AND (ca.owner=$3 OR ca.submitted_by=$3 OR m.can_review) FOR UPDATE OF ca FOR SHARE OF m")
            .bind(&a.org).bind(p).bind(&a.user).bind(id).fetch_optional(&mut *tx).await.map_err(db)?;
        let anchor: AnchorRecord = decode_value(v.ok_or(Error::NotFound)?)?;
        if !review && anchor.owner != a.user {
            return Err(Error::Denied);
        }
        let repo = Self::code_repository(&mut tx, a, p, anchor.repository_id).await?;
        let (card, owner) = Self::anchor_card(&mut tx, a, p, anchor.card_id).await?;
        if anchor.revision != revision
            || repo.revision != anchor.repository_revision
            || card.revision != anchor.card_revision
            || owner != anchor.owner
        {
            return Err(Error::Conflict);
        }
        if let Some(cid) = anchor.contribution_id {
            let c = Self::code_contribution(&mut tx, a, p, cid).await?;
            if Some(c.revision) != anchor.contribution_revision
                || c.reviewed_by.is_none()
                || c.confirmed_by.as_deref() != Some(&c.owner)
            {
                return Err(Error::Conflict);
            }
        }
        if review && (!card.published || anchor.confirmed_by.as_deref() != Some(&owner)) {
            return Err(Error::Conflict);
        }
        let query = if review {
            "UPDATE code_rationale_anchors SET reviewed_by=$4,reviewed_at_ms=floor(extract(epoch FROM clock_timestamp())*1000)::bigint,revision=revision+1 WHERE org=$1 AND product=$2 AND id=$3 RETURNING to_jsonb(code_rationale_anchors)-'org'-'product'"
        } else {
            "UPDATE code_rationale_anchors SET confirmed_by=$4,confirmed_at_ms=floor(extract(epoch FROM clock_timestamp())*1000)::bigint,reviewed_by=NULL,reviewed_at_ms=NULL,revision=revision+1 WHERE org=$1 AND product=$2 AND id=$3 RETURNING to_jsonb(code_rationale_anchors)-'org'-'product'"
        };
        let v: serde_json::Value = sqlx::query_scalar(query)
            .bind(&a.org)
            .bind(p)
            .bind(id)
            .bind(&a.user)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        let result = decode_value(v)?;
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    pub async fn configure_code_maintainer(
        &self,
        a: &Actor,
        p: &str,
        input: MaintainerInput,
    ) -> Result<(), Error> {
        scopes(p, &input.services, input.base_revision)?;
        if !valid_targets(&input.targets) || !wk_contracts::valid_id(&input.user_id) {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        Self::lock_administration(&mut tx, a, p).await?;
        Self::code_operator(&mut tx, a, p, &input.services).await?;
        let repo = Self::code_repository(&mut tx, a, p, input.repository_id).await?;
        let services = union_scopes(&repo.services, &input.services);
        Self::code_operator(&mut tx, a, p, &services).await?;
        Self::code_member(
            &mut tx,
            &Actor {
                org: a.org.clone(),
                user: input.user_id.clone(),
            },
            p,
            &services,
        )
        .await?;
        let targets = serde_json::to_value(input.targets).map_err(|_| Error::Invalid)?;
        let old=sqlx::query("SELECT id,revision,services FROM code_maintainers WHERE org=$1 AND product=$2 AND repository_id=$3 AND targets=$4 FOR UPDATE")
            .bind(&a.org).bind(p).bind(repo.id).bind(&targets).fetch_optional(&mut *tx).await.map_err(db)?;
        if let Some(old) = old {
            if input.base_revision != Some(old.try_get("revision").map_err(db)?) {
                return Err(Error::Conflict);
            }
            Self::code_operator(
                &mut tx,
                a,
                p,
                &old.try_get::<Vec<String>, _>("services").map_err(db)?,
            )
            .await?;
            sqlx::query("UPDATE code_maintainers SET user_id=$4,services=$5,revision=revision+1,reviewer=$6 WHERE org=$1 AND product=$2 AND id=$3")
                .bind(&a.org).bind(p).bind(old.try_get::<uuid::Uuid,_>("id").map_err(db)?).bind(input.user_id).bind(services).bind(&a.user).execute(&mut *tx).await.map_err(db)?;
        } else {
            if input.base_revision.is_some() {
                return Err(Error::Conflict);
            }
            sqlx::query("INSERT INTO code_maintainers(org,product,id,repository_id,user_id,targets,services,reviewer) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind(&a.org).bind(p).bind(uuid::Uuid::new_v4()).bind(repo.id).bind(input.user_id).bind(targets).bind(services).bind(&a.user).execute(&mut *tx).await.map_err(db)?;
        }
        tx.commit().await.map_err(db)
    }
}
