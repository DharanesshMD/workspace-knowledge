//! Explicit repository and contributor metadata; identity mappings never prove authorship.
use super::*;
pub use crate::code_anchors::{
    AnchorInput, AnchorRecord, ContributionInput, ContributionRecord, MaintainerInput,
};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryInput {
    pub id: uuid::Uuid,
    pub services: Vec<String>,
    pub high_risk_paths: Vec<String>,
    pub contact_hosts: Vec<String>,
    pub base_revision: Option<i64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RepositoryRecord {
    pub id: uuid::Uuid,
    pub services: Vec<String>,
    pub high_risk_paths: Vec<String>,
    pub contact_hosts: Vec<String>,
    pub revision: i64,
    pub revoked: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileInput {
    pub display_name: Option<String>,
    pub contact_url: Option<String>,
    pub services: Vec<String>,
    pub base_revision: Option<i64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProfileRecord {
    pub user_id: String,
    pub display_name: Option<String>,
    pub contact_url: Option<String>,
    pub services: Vec<String>,
    pub revision: i64,
    pub revoked: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MappingInput {
    pub repository_id: uuid::Uuid,
    pub user_id: String,
    pub identity_reference: String,
    pub proof_reference: String,
    pub services: Vec<String>,
    pub base_revision: Option<i64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MappingRecord {
    pub id: uuid::Uuid,
    pub repository_id: uuid::Uuid,
    pub user_id: String,
    pub identity_reference: String,
    pub proof_reference: String,
    pub services: Vec<String>,
    pub revision: i64,
    pub reviewer: String,
    pub assurance: String,
    pub revoked: bool,
}
pub(crate) fn scopes(p: &str, services: &[String], base: Option<i64>) -> Result<(), Error> {
    let unique: std::collections::BTreeSet<_> = services.iter().collect();
    if !wk_contracts::valid_id(p)
        || services.is_empty()
        || services.len() > 50
        || unique.len() != services.len()
        || services.iter().any(|v| !wk_contracts::valid_id(v))
        || base.is_some_and(|v| v < 1)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn safe_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty()
        && s.len() <= max
        && !s.chars().any(char::is_control)
        && !wk_summarize::sensitive(s)
}
fn safe_host(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
        && s.split('.')
            .all(|part| !part.is_empty() && !part.starts_with('-') && !part.ends_with('-'))
}
pub(crate) fn decode_value<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, Error> {
    serde_json::from_value(v).map_err(|_| Error::Storage)
}
fn write_failure(e: sqlx::Error) -> Error {
    match e
        .as_database_error()
        .and_then(|d| d.code().map(|c| c.into_owned()))
        .as_deref()
    {
        Some("42501") => Error::Denied,
        Some("23505") => Error::Conflict,
        Some("23503" | "23514") => Error::Invalid,
        _ => Error::Storage,
    }
}
impl Knowledge {
    pub(crate) async fn code_member(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        services: &[String],
    ) -> Result<(), Error> {
        let ok=sqlx::query("SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 AND services @> $4::text[] FOR SHARE")
            .bind(&a.org).bind(p).bind(&a.user).bind(services).fetch_optional(&mut **tx).await.map_err(db)?;
        ok.map(|_| ()).ok_or(Error::Denied)
    }
    pub(crate) async fn code_operator(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        services: &[String],
    ) -> Result<(), Error> {
        // No runtime-role writer grant and no HTTP operator endpoint.
        let owner:bool=sqlx::query_scalar("SELECT pg_has_role(current_user,c.relowner,'USAGE') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname='code_repositories'")
            .fetch_one(&mut **tx).await.map_err(db)?;
        if !owner {
            return Err(Error::Denied);
        }
        let ok=sqlx::query("SELECT 1 FROM product_admins WHERE org=$1 AND product=$2 AND user_id=$3 AND services @> $4::text[] FOR SHARE")
            .bind(&a.org).bind(p).bind(&a.user).bind(services).fetch_optional(&mut **tx).await.map_err(db)?;
        if ok.is_none() {
            return Err(Error::Denied);
        }
        Self::code_member(tx, a, p, services).await
    }
    pub async fn configure_repository(
        &self,
        a: &Actor,
        p: &str,
        input: RepositoryInput,
    ) -> Result<RepositoryRecord, Error> {
        scopes(p, &input.services, input.base_revision)?;
        if input.id.is_nil()
            || input.high_risk_paths.len() > 20
            || input.contact_hosts.len() > 16
            || input.contact_hosts.iter().any(|h| !safe_host(h))
            || input
                .high_risk_paths
                .iter()
                .any(|v| !wk_contracts::change_context::valid_path(v.trim_end_matches('/')))
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.code_tx(a, p).await?;
        Self::lock_administration(&mut tx, a, p).await?;
        Self::code_operator(&mut tx, a, p, &input.services).await?;
        let old:Option<Vec<String>>=sqlx::query_scalar("SELECT services FROM code_repositories WHERE org=$1 AND product=$2 AND id=$3 FOR UPDATE")
            .bind(&a.org).bind(p).bind(input.id).fetch_optional(&mut *tx).await.map_err(db)?;
        if input.base_revision.is_some() && old.is_none() {
            return Err(Error::Conflict);
        }
        if let Some(old) = old {
            Self::code_operator(&mut tx, a, p, &old).await?;
        }
        let row:Option<Value>=sqlx::query_scalar("INSERT INTO code_repositories(org,product,id,services,high_risk_paths,contact_hosts) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(org,product,id) DO UPDATE SET services=EXCLUDED.services,high_risk_paths=EXCLUDED.high_risk_paths,contact_hosts=EXCLUDED.contact_hosts,revision=code_repositories.revision+1 WHERE code_repositories.revision=$7 RETURNING to_jsonb(code_repositories)-'org'-'product'")
            .bind(&a.org).bind(p).bind(input.id).bind(input.services).bind(input.high_risk_paths).bind(input.contact_hosts).bind(input.base_revision).fetch_optional(&mut *tx).await.map_err(write_failure)?;
        let result = decode_value(row.ok_or(Error::Conflict)?)?;
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    pub async fn put_code_profile(
        &self,
        a: &Actor,
        p: &str,
        input: ProfileInput,
    ) -> Result<ProfileRecord, Error> {
        scopes(p, &input.services, input.base_revision)?;
        if input
            .display_name
            .as_ref()
            .is_some_and(|s| !safe_text(s, 128))
            || (input.display_name.is_none() && input.contact_url.is_some())
        {
            return Err(Error::Invalid);
        }
        if input.display_name.is_none() && input.base_revision.is_some() {
            let mut tx = self.code_tx(a, p).await?;
            let v:Option<Value>=sqlx::query_scalar("UPDATE code_profiles SET display_name=NULL,contact_url=NULL,revoked=true,revision=revision+1 WHERE org=$1 AND product=$2 AND user_id=$3 AND revision=$4 RETURNING to_jsonb(code_profiles)-'org'-'product'")
                .bind(&a.org).bind(p).bind(&a.user).bind(input.base_revision).fetch_optional(&mut *tx).await.map_err(write_failure)?;
            let result = decode_value(v.ok_or(Error::Conflict)?)?;
            tx.commit().await.map_err(db)?;
            return Ok(result);
        }
        let mut tx = self.code_tx(a, p).await?;
        Self::lock_administration(&mut tx, a, p).await?;
        Self::code_member(&mut tx, a, p, &input.services).await?;
        if let Some(ref contact) = input.contact_url {
            let u = url::Url::parse(contact).map_err(|_| Error::Invalid)?;
            if contact.len() > 512
                || contact.chars().any(char::is_control)
                || u.as_str() != contact
                || u.scheme() != "https"
                || !u.username().is_empty()
                || u.password().is_some()
                || u.query().is_some()
                || u.fragment().is_some()
                || u.port().is_some()
            {
                return Err(Error::Invalid);
            }
            let host = u.host_str().ok_or(Error::Invalid)?;
            let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM code_repositories r WHERE r.org=$1 AND r.product=$2 AND NOT r.revoked AND $3::text=ANY(r.contact_hosts) AND r.services <@ $4::text[])")
                .bind(&a.org).bind(p).bind(host).bind(&input.services).fetch_one(&mut *tx).await.map_err(db)?;
            if !allowed {
                return Err(Error::Invalid);
            }
        }
        let existing:Option<i64>=sqlx::query_scalar("SELECT revision FROM code_profiles WHERE org=$1 AND product=$2 AND user_id=$3 FOR UPDATE")
            .bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut *tx).await.map_err(db)?;
        if input.base_revision.is_some() && existing.is_none() {
            return Err(Error::Conflict);
        }
        let revoked = input.display_name.is_none();
        let row:Option<Value>=sqlx::query_scalar("INSERT INTO code_profiles(org,product,user_id,services,display_name,contact_url,revoked) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(org,product,user_id) DO UPDATE SET services=EXCLUDED.services,display_name=EXCLUDED.display_name,contact_url=EXCLUDED.contact_url,revoked=EXCLUDED.revoked,revision=code_profiles.revision+1 WHERE code_profiles.revision=$8 RETURNING to_jsonb(code_profiles)-'org'-'product'")
            .bind(&a.org).bind(p).bind(&a.user).bind(input.services).bind(input.display_name).bind(input.contact_url).bind(revoked).bind(input.base_revision).fetch_optional(&mut *tx).await.map_err(write_failure)?;
        let result = decode_value(row.ok_or(Error::Conflict)?)?;
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    pub async fn configure_code_mapping(
        &self,
        a: &Actor,
        p: &str,
        input: MappingInput,
    ) -> Result<MappingRecord, Error> {
        scopes(p, &input.services, input.base_revision)?;
        let (prefix, id) = input
            .identity_reference
            .split_once(':')
            .ok_or(Error::Invalid)?;
        if input.repository_id.is_nil()
            || !["team", "provider"].contains(&prefix)
            || !wk_contracts::valid_id(id)
            || !wk_contracts::valid_id(&input.user_id)
            || !safe_text(&input.proof_reference, 512)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.code_tx(a, p).await?;
        Self::lock_administration(&mut tx, a, p).await?;
        Self::code_operator(&mut tx, a, p, &input.services).await?;
        let member = Actor {
            org: a.org.clone(),
            user: input.user_id.clone(),
        };
        Self::code_member(&mut tx, &member, p, &input.services).await?;
        let repo:Option<Vec<String>>=sqlx::query_scalar("SELECT services FROM code_repositories WHERE org=$1 AND product=$2 AND id=$3 AND NOT revoked FOR SHARE")
            .bind(&a.org).bind(p).bind(input.repository_id).fetch_optional(&mut *tx).await.map_err(db)?;
        Self::code_operator(&mut tx, a, p, &repo.ok_or(Error::Denied)?).await?;
        let old:Option<Vec<String>>=sqlx::query_scalar("SELECT services FROM code_mappings WHERE org=$1 AND product=$2 AND repository_id=$3 AND identity_reference=$4 FOR UPDATE")
            .bind(&a.org).bind(p).bind(input.repository_id).bind(&input.identity_reference).fetch_optional(&mut *tx).await.map_err(db)?;
        if input.base_revision.is_some() && old.is_none() {
            return Err(Error::Conflict);
        }
        if let Some(old) = old {
            Self::code_operator(&mut tx, a, p, &old).await?;
        }
        let row:Option<Value>=sqlx::query_scalar("INSERT INTO code_mappings(org,product,id,repository_id,user_id,identity_reference,proof_reference,services,reviewer) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(org,product,repository_id,identity_reference) DO UPDATE SET user_id=EXCLUDED.user_id,proof_reference=EXCLUDED.proof_reference,services=EXCLUDED.services,reviewer=EXCLUDED.reviewer,revision=code_mappings.revision+1 WHERE code_mappings.revision=$10 RETURNING to_jsonb(code_mappings)-'org'-'product'")
            .bind(&a.org).bind(p).bind(uuid::Uuid::new_v4()).bind(input.repository_id).bind(input.user_id).bind(input.identity_reference).bind(input.proof_reference).bind(input.services).bind(&a.user).bind(input.base_revision).fetch_optional(&mut *tx).await.map_err(write_failure)?;
        let result = decode_value(row.ok_or(Error::Conflict)?)?;
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
}

impl Knowledge {
    /// Owner/reviewer work queue, not a directory or shared intent feed.
    pub async fn code_attribution(&self, a: &Actor, p: &str) -> Result<Value, Error> {
        if !wk_contracts::valid_id(p) {
            return Err(Error::Invalid);
        }
        let mut tx = self.code_tx(a, p).await?;
        let profile:Option<Value>=sqlx::query_scalar("SELECT to_jsonb(cp)-'org'-'product' FROM code_profiles cp WHERE cp.org=$1 AND cp.product=$2 AND cp.user_id=$3")
            .bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut *tx).await.map_err(db)?;
        let can_review:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 AND can_review)")
            .bind(&a.org).bind(p).bind(&a.user).fetch_one(&mut *tx).await.map_err(db)?;
        let repositories:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(r)-'org'-'product' FROM code_repositories r JOIN memberships m ON m.org=r.org AND m.product=r.product AND m.user_id=$3 WHERE r.org=$1 AND r.product=$2 AND NOT r.revoked AND m.services @> r.services ORDER BY r.id LIMIT 21")
            .bind(&a.org).bind(p).bind(&a.user).fetch_all(&mut *tx).await.map_err(db)?;
        // Identity references are candidates, not contributor names. Proof text stays operator-only.
        let mappings:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',cm.id,'repository_id',cm.repository_id,'user_id',cm.user_id,'revision',cm.revision,'assurance',cm.assurance) FROM code_mappings cm JOIN code_repositories r ON r.org=cm.org AND r.product=cm.product AND r.id=cm.repository_id JOIN memberships m ON m.org=cm.org AND m.product=cm.product AND m.user_id=$3 WHERE cm.org=$1 AND cm.product=$2 AND NOT cm.revoked AND NOT r.revoked AND m.services @> cm.services AND m.services @> r.services ORDER BY cm.id LIMIT 21")
            .bind(&a.org).bind(p).bind(&a.user).fetch_all(&mut *tx).await.map_err(db)?;
        let contributions:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(co)-'org'-'product' FROM code_contributions co JOIN code_repositories r ON r.org=co.org AND r.product=co.product AND r.id=co.repository_id JOIN memberships m ON m.org=co.org AND m.product=co.product AND m.user_id=$3 WHERE co.org=$1 AND co.product=$2 AND NOT co.revoked AND NOT r.revoked AND m.services @> co.services AND m.services @> r.services AND (co.owner=$3 OR m.can_review) ORDER BY co.id LIMIT 21")
            .bind(&a.org).bind(p).bind(&a.user).fetch_all(&mut *tx).await.map_err(db)?;
        let rows=sqlx::query("SELECT to_jsonb(an)-'org'-'product' AS anchor,c.id,c.revision,c.published,c.author,c.body->>'title' AS title,c.body->>'rationale' AS rationale FROM code_rationale_anchors an JOIN code_repositories r ON r.org=an.org AND r.product=an.product AND r.id=an.repository_id JOIN cards c ON c.org=an.org AND c.product=an.product AND c.id=an.card_id JOIN memberships m ON m.org=an.org AND m.product=an.product AND m.user_id=$3 WHERE an.org=$1 AND an.product=$2 AND NOT an.revoked AND NOT r.revoked AND NOT c.withdrawn AND m.services @> an.services AND m.services @> c.services AND m.services @> r.services AND (an.owner=$3 OR m.can_review) ORDER BY an.id LIMIT 21")
            .bind(&a.org).bind(p).bind(&a.user).fetch_all(&mut *tx).await.map_err(db)?;
        let mut anchors = Vec::new();
        for row in rows {
            let rationale: String = row.try_get("rationale").map_err(db)?;
            anchors.push(serde_json::json!({"anchor":row.try_get::<Value,_>("anchor").map_err(db)?,"card":{"id":row.try_get::<uuid::Uuid,_>("id").map_err(db)?,"revision":row.try_get::<i64,_>("revision").map_err(db)?,"published":row.try_get::<bool,_>("published").map_err(db)?,"author":row.try_get::<String,_>("author").map_err(db)?,"title":row.try_get::<String,_>("title").map_err(db)?,"rationale":rationale.chars().take(2000).collect::<String>(),"truncated":rationale.chars().count()>2000}}));
        }
        let mut out = serde_json::json!({"profile":profile,"can_review":can_review,"repositories":repositories,"mappings":mappings,"contributions":contributions,"anchors":anchors,"truncated":false});
        for key in ["repositories", "mappings", "contributions", "anchors"] {
            let list = out[key].as_array_mut().ok_or(Error::Storage)?;
            if list.len() > 20 {
                list.truncate(20);
                out["truncated"] = true.into();
            }
        }
        while serde_json::to_vec(&out).map_err(|_| Error::Storage)?.len() > 16384 {
            out["truncated"] = true.into();
            let mut removed = false;
            for key in ["anchors", "contributions", "mappings", "repositories"] {
                if out[key]
                    .as_array_mut()
                    .ok_or(Error::Storage)?
                    .pop()
                    .is_some()
                {
                    removed = true;
                    break;
                }
            }
            if !removed {
                return Err(Error::Storage);
            }
        }
        tx.commit().await.map_err(db)?;
        Ok(out)
    }
}

impl Knowledge {
    pub async fn registered_repository(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<RepositoryRecord, Error> {
        if !wk_contracts::valid_id(p) {
            return Err(Error::Invalid);
        }
        let mut tx = self.code_tx(a, p).await?;
        let record = Self::code_repository(&mut tx, a, p, id).await?;
        if self.device_guard.as_ref().is_some_and(|g| {
            g.product != p || record.services.iter().any(|s| !g.services.contains(s))
        }) {
            return Err(Error::Denied);
        }
        tx.commit().await.map_err(db)?;
        Ok(record)
    }
}
