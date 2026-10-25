//! Share a session summary from simple fields: the developer's own assistant writes it, the
//! developer's own sign-in submits it, and the existing publish-on-submit rules decide whether
//! teammates see it at once. Nothing here widens what `submit_publishing` already allows.
use super::*;

/// What an assistant supplies. There is deliberately no provenance, policy, lifecycle,
/// completeness or evidence field: the server sets those.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareRequest {
    /// Required only when the member holds more than one service in the product.
    #[serde(default)]
    pub service: Option<String>,
    pub title: String,
    pub intent: String,
    pub rationale: String,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    /// Workspace-relative files the assistant changed or relied on (at most 20). They are what lets a
    /// teammate ask about a file or module later. Agent-reported, so shown as such, never as observed.
    #[serde(default)]
    pub files: Vec<String>,
}
/// Most files one summary may list.
pub const MAX_FILES: usize = 20;
/// Normalize and check the files an assistant listed: `./` prefixes dropped, duplicates removed,
/// files nobody wants to be told about (plans, task lists, lockfiles) skipped. Anything that is not a
/// plain workspace-relative path is refused, as is a list that is too long.
pub fn clean_files(files: &[String]) -> Result<Vec<String>, Error> {
    if files.len() > MAX_FILES * 2 {
        return Err(Error::Invalid);
    }
    let mut out: Vec<String> = Vec::new();
    for raw in files {
        let mut f = raw.trim();
        while let Some(rest) = f.strip_prefix("./") {
            f = rest;
        }
        if !activity::valid_path(f) || f.contains(':') {
            return Err(Error::Invalid);
        }
        if activity::bookkeeping(f) || out.iter().any(|x| x == f) {
            continue;
        }
        out.push(f.to_string());
    }
    if out.len() > MAX_FILES {
        return Err(Error::Invalid);
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareStatus {
    /// Teammates can find it now.
    Published,
    /// The product is not enabled for publish on submit: an ordinary review copy.
    SubmittedForReview,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShareOutcome {
    pub card_id: uuid::Uuid,
    pub revision: i64,
    pub published: bool,
    pub status: ShareStatus,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ShareError {
    /// The member holds several services in this product; the caller must pick one.
    #[error("service required: choose one of {0:?}")]
    ServiceRequired(Vec<String>),
    /// Privacy screening refused the text. Nothing was stored.
    #[error("summary requires private review")]
    Sensitive,
    #[error(transparent)]
    Failed(#[from] Error),
}

impl Knowledge {
    pub async fn share_session(
        &self,
        actor: &Actor,
        product: &str,
        request: ShareRequest,
    ) -> Result<ShareOutcome, ShareError> {
        // A non-member is denied, the same as for any other submission.
        let policy = self.policy(actor, product).await.map_err(|e| match e {
            Error::NotFound => Error::Denied,
            other => other,
        })?;
        let mut held = policy.services;
        held.sort();
        held.dedup();
        let service = match request.service.clone() {
            Some(s) if held.contains(&s) => s,
            Some(_) => return Err(Error::Denied.into()),
            None => match held.as_slice() {
                [] => return Err(Error::Denied.into()),
                [only] => only.clone(),
                _ => return Err(ShareError::ServiceRequired(held)),
            },
        };
        let files = clean_files(&request.files)?;
        if !files.is_empty()
            && wk_summarize::sensitive(&serde_json::json!({ "files": files }).to_string())
        {
            return Err(ShareError::Sensitive);
        }
        let key = self
            .share_key(
                actor,
                product,
                &service,
                policy.policy_revision,
                &request,
                &files,
            )
            .await?;
        let body = CardSubmission {
            schema_version: 1,
            idempotency_key: key,
            policy_revision: u64::try_from(policy.policy_revision).map_err(|_| Error::Invalid)?,
            title: request.title,
            intent: request.intent,
            rationale: request.rationale,
            decisions: request.decisions,
            constraints: request.constraints,
            open_questions: request.open_questions,
            service_ids: vec![service],
            evidence: vec![],
            lifecycle: wk_contracts::Lifecycle::Proposed,
            capture_completeness: wk_contracts::Completeness::Partial,
            base_revision: None,
            observed_at_ms: None,
            provenance: Some(wk_contracts::Provenance::AssistantWritten),
        };
        body.validate().map_err(|_| Error::Invalid)?;
        // The same screen `POST /v1/cards` applies to the whole submission.
        if wk_summarize::sensitive(&serde_json::to_string(&body).map_err(|_| Error::Invalid)?) {
            return Err(ShareError::Sensitive);
        }
        let card = self.submit_publishing(actor, product, body, true).await?;
        self.record_card_files(actor, product, card.id, &files)
            .await?;
        Ok(ShareOutcome {
            card_id: card.id,
            revision: card.revision,
            published: card.published,
            status: if card.published {
                ShareStatus::Published
            } else {
                ShareStatus::SubmittedForReview
            },
        })
    }

    /// Remember the files a shared summary lists. Only the card's own author can add rows (the
    /// statement joins on the author), repeating a share adds nothing new, and it is a no-op for
    /// an empty list.
    async fn record_card_files(
        &self,
        actor: &Actor,
        product: &str,
        card_id: uuid::Uuid,
        files: &[String],
    ) -> Result<(), Error> {
        if files.is_empty() {
            return Ok(());
        }
        let mut tx = self.actor_tx(actor).await?;
        sqlx::query("INSERT INTO card_files(org,product,card_id,path) SELECT c.org,c.product,c.id,p FROM cards c, unnest($4::text[]) p WHERE c.id=$3 AND c.org=$1 AND c.product=$2 AND c.author=$5 ON CONFLICT DO NOTHING")
            .bind(&actor.org)
            .bind(product)
            .bind(card_id)
            .bind(files)
            .bind(&actor.user)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)
    }

    /// Hex SHA-256 of the product, author and every content field, so repeating the same
    /// share finds the same card. Computed by the database, like the other content hashes here.
    async fn share_key(
        &self,
        actor: &Actor,
        product: &str,
        service: &str,
        policy_revision: i64,
        r: &ShareRequest,
        files: &[String],
    ) -> Result<String, Error> {
        let mut canonical = serde_json::json!([
            "share_session/v1",
            product,
            actor.org,
            actor.user,
            service,
            policy_revision,
            r.title,
            r.intent,
            r.rationale,
            r.decisions,
            r.constraints,
            r.open_questions
        ]);
        // Only a share that lists files includes them, so a share made before this field existed
        // keeps the key it was stored under.
        if !files.is_empty() {
            canonical
                .as_array_mut()
                .expect("array")
                .push(serde_json::json!(files));
        }
        let canonical = canonical.to_string();
        sqlx::query_scalar("SELECT encode(sha256(convert_to($1,'UTF8')),'hex')")
            .bind(canonical)
            .fetch_one(&self.pool)
            .await
            .map_err(db)
    }
}
