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
        let key = self
            .share_key(actor, product, &service, policy.policy_revision, &request)
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

    /// Hex SHA-256 of the product, author and every content field, so repeating the same
    /// share finds the same card. Computed by the database, like the other content hashes here.
    async fn share_key(
        &self,
        actor: &Actor,
        product: &str,
        service: &str,
        policy_revision: i64,
        r: &ShareRequest,
    ) -> Result<String, Error> {
        let canonical = serde_json::json!([
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
        ])
        .to_string();
        sqlx::query_scalar("SELECT encode(sha256(convert_to($1,'UTF8')),'hex')")
            .bind(canonical)
            .fetch_one(&self.pool)
            .await
            .map_err(db)
    }
}
