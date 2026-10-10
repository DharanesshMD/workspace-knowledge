//! Owner-only sanitized notes; no shared retrieval path uses this module.
use super::*;
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PersonalProvenance {
    Manual,
    Captured,
}
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct PersonalInput {
    pub body: CardSubmission,
    pub provenance: PersonalProvenance,
}
#[derive(Debug, Serialize)]
pub struct PersonalSummary {
    pub id: uuid::Uuid,
    pub body: CardSubmission,
    pub revision: i64,
    pub provenance: PersonalProvenance,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub submitted_card: Option<uuid::Uuid>,
}
fn checked(input: &PersonalInput) -> Result<serde_json::Value, Error> {
    input.body.validate().map_err(|_| Error::Invalid)?;
    if input.body.base_revision.is_some()
        || input.body.idempotency_key.len() > 128
        || !matches!(
            input.body.capture_completeness,
            wk_contracts::Completeness::Complete
        )
        || (input.provenance == PersonalProvenance::Manual
            && (!matches!(input.body.lifecycle, wk_contracts::Lifecycle::Proposed)
                || !input.body.evidence.is_empty()))
    {
        return Err(Error::Invalid);
    }
    let body = serde_json::to_value(&input.body).map_err(|_| Error::Invalid)?;
    validate_shared_body(&body)?;
    if body.to_string().len() > 60000 {
        return Err(Error::Invalid);
    }
    Ok(body)
}
fn decode_personal(row: sqlx::postgres::PgRow) -> Result<PersonalSummary, Error> {
    Ok(PersonalSummary {
        id: row.try_get("id").map_err(db)?,
        body: serde_json::from_value(row.try_get("body").map_err(db)?)
            .map_err(|_| Error::Storage)?,
        revision: row.try_get("revision").map_err(db)?,
        provenance: match row.try_get::<String, _>("provenance").map_err(db)?.as_str() {
            "manual" => PersonalProvenance::Manual,
            "captured" => PersonalProvenance::Captured,
            _ => return Err(Error::Storage),
        },
        created_at_ms: row.try_get("created_at_ms").map_err(db)?,
        expires_at_ms: row.try_get("expires_at_ms").map_err(db)?,
        submitted_card: row.try_get("submitted_card").map_err(db)?,
    })
}
const FIELDS:&str="s.id,s.body,s.revision,s.provenance,s.submitted_card,(extract(epoch FROM s.created_at)*1000)::bigint AS created_at_ms,(extract(epoch FROM s.expires_at)*1000)::bigint AS expires_at_ms";
impl Knowledge {
    async fn personal_lock_membership(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        body: &CardSubmission,
    ) -> Result<(), Error> {
        let revision = i64::try_from(body.policy_revision).map_err(|_| Error::Invalid)?;
        if sqlx::query("SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 AND services @> $4::text[] AND policy_revision=$5 FOR SHARE").bind(&a.org).bind(p).bind(&a.user).bind(&body.service_ids).bind(revision).fetch_optional(&mut **tx).await.map_err(db)?.is_none(){return Err(Error::Denied);}
        Ok(())
    }
    pub async fn personal_put(
        &self,
        a: &Actor,
        p: &str,
        input: PersonalInput,
    ) -> Result<PersonalSummary, Error> {
        let value = checked(&input)?;
        let mut tx = self.actor_tx(a).await?;
        Self::personal_lock_membership(&mut tx, a, p, &input.body).await?;
        let has_capacity: bool =
            sqlx::query_scalar("SELECT public.wk_personal_has_capacity($1,$2,$3)")
                .bind(&a.org)
                .bind(p)
                .bind(&a.user)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        let existing=sqlx::query(&format!("SELECT {FIELDS},s.deleted,s.expires_at>now() AS live FROM personal_summaries s WHERE s.org=$1 AND s.product=$2 AND s.owner=$3 AND s.idempotency_key=$4")).bind(&a.org).bind(p).bind(&a.user).bind(&input.body.idempotency_key).fetch_optional(&mut *tx).await.map_err(db)?;
        if let Some(row) = existing {
            if row.try_get::<bool, _>("deleted").map_err(db)?
                || !row.try_get::<bool, _>("live").map_err(db)?
            {
                return Err(Error::Conflict);
            }
            let note = decode_personal(row)?;
            if serde_json::to_value(&note.body).map_err(|_| Error::Storage)? != value
                || note.provenance != input.provenance
            {
                return Err(Error::Conflict);
            }
            return Ok(note);
        }
        if !has_capacity {
            return Err(Error::Conflict);
        }
        let provenance = match input.provenance {
            PersonalProvenance::Manual => "manual",
            PersonalProvenance::Captured => "captured",
        };
        let row=sqlx::query(&format!("INSERT INTO personal_summaries AS s(id,org,product,owner,idempotency_key,services,body,provenance) VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING {FIELDS}")).bind(uuid::Uuid::new_v4()).bind(&a.org).bind(p).bind(&a.user).bind(&input.body.idempotency_key).bind(&input.body.service_ids).bind(value).bind(provenance).fetch_one(&mut *tx).await.map_err(db)?;
        let note = decode_personal(row)?;
        tx.commit().await.map_err(db)?;
        Ok(note)
    }
    pub async fn personal_get(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<PersonalSummary, Error> {
        let mut tx = self.actor_tx(a).await?;
        let row=sqlx::query(&format!("SELECT {FIELDS} FROM personal_summaries s JOIN memberships m ON m.org=s.org AND m.product=s.product AND m.user_id=s.owner WHERE s.id=$1 AND s.org=$2 AND s.product=$3 AND s.owner=$4 AND m.services @> s.services AND NOT s.deleted AND s.expires_at>now()")).bind(id).bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        decode_personal(row)
    }
    pub async fn personal_list(
        &self,
        a: &Actor,
        p: &str,
        before: Option<uuid::Uuid>,
    ) -> Result<Vec<PersonalSummary>, Error> {
        let mut tx = self.actor_tx(a).await?;
        let rows=sqlx::query(&format!("SELECT {FIELDS} FROM personal_summaries s JOIN memberships m ON m.org=s.org AND m.product=s.product AND m.user_id=s.owner WHERE s.org=$1 AND s.product=$2 AND s.owner=$3 AND m.services @> s.services AND NOT s.deleted AND s.expires_at>now() AND ($4::uuid IS NULL OR (s.created_at,s.id)<(SELECT anchor.created_at,anchor.id FROM personal_summaries anchor WHERE anchor.id=$4 AND anchor.org=$1 AND anchor.product=$2 AND anchor.owner=$3)) ORDER BY s.created_at DESC,s.id DESC LIMIT 50")).bind(&a.org).bind(p).bind(&a.user).bind(before).fetch_all(&mut *tx).await.map_err(db)?;
        rows.into_iter().map(decode_personal).collect()
    }
    pub async fn personal_edit(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        expected: i64,
        body: CardSubmission,
    ) -> Result<PersonalSummary, Error> {
        let mut tx = self.actor_tx(a).await?;
        let row=sqlx::query(&format!("SELECT {FIELDS} FROM personal_summaries s JOIN memberships m ON m.org=s.org AND m.product=s.product AND m.user_id=s.owner WHERE s.id=$1 AND s.org=$2 AND s.product=$3 AND s.owner=$4 AND m.services @> s.services AND NOT s.deleted AND s.expires_at>now() FOR UPDATE OF s")).bind(id).bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let note = decode_personal(row)?;
        if note.revision != expected {
            return Err(Error::Conflict);
        }
        if note.body.idempotency_key != body.idempotency_key {
            return Err(Error::Invalid);
        }
        let value = checked(&PersonalInput {
            body: body.clone(),
            provenance: note.provenance,
        })?;
        Self::personal_lock_membership(&mut tx, a, p, &body).await?;
        let row=sqlx::query(&format!("UPDATE personal_summaries AS s SET body=$2,services=$3,revision=revision+1,submitted_card=NULL WHERE id=$1 RETURNING {FIELDS}")).bind(id).bind(value).bind(&body.service_ids).fetch_one(&mut *tx).await.map_err(db)?;
        let note = decode_personal(row)?;
        tx.commit().await.map_err(db)?;
        Ok(note)
    }
    pub async fn personal_delete(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        expected: i64,
    ) -> Result<(), Error> {
        let mut tx = self.actor_tx(a).await?;
        let row=sqlx::query("SELECT s.revision FROM personal_summaries s JOIN memberships m ON m.org=s.org AND m.product=s.product AND m.user_id=s.owner WHERE s.id=$1 AND s.org=$2 AND s.product=$3 AND s.owner=$4 AND m.services @> s.services AND NOT s.deleted AND s.expires_at>now() FOR UPDATE OF s FOR SHARE OF m").bind(id).bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        if row.try_get::<i64, _>("revision").map_err(db)? != expected {
            return Err(Error::Conflict);
        }
        sqlx::query(
            "UPDATE personal_summaries SET body=NULL,deleted=TRUE,revision=revision+1 WHERE id=$1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)
    }
    pub async fn personal_submit(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        expected: i64,
    ) -> Result<Card, Error> {
        let mut tx = self.actor_tx(a).await?;
        let row=sqlx::query(&format!("SELECT {FIELDS},m.policy_revision AS current_policy_revision FROM personal_summaries s JOIN memberships m ON m.org=s.org AND m.product=s.product AND m.user_id=s.owner WHERE s.id=$1 AND s.org=$2 AND s.product=$3 AND s.owner=$4 AND m.services @> s.services AND NOT s.deleted AND s.expires_at>now() FOR UPDATE OF s FOR SHARE OF m")).bind(id).bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        let policy: i64 = row.try_get("current_policy_revision").map_err(db)?;
        let note = decode_personal(row)?;
        if note.revision != expected {
            return Err(Error::Conflict);
        }
        checked(&PersonalInput {
            body: note.body.clone(),
            provenance: note.provenance,
        })?;
        let mut body = note.body;
        body.policy_revision = u64::try_from(policy).map_err(|_| Error::Storage)?;
        body.idempotency_key = format!("owner-note:{id}:{}:{policy}", note.revision);
        let card = Self::submit_in_transaction(&mut tx, a, p, body).await?;
        sqlx::query("UPDATE personal_summaries SET submitted_card=$2 WHERE id=$1")
            .bind(id)
            .bind(card.id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(card)
    }
}
