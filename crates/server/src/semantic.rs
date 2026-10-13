//! One authorization-preserving retrieval path for HTTP and MCP.
use crate::*;
pub(crate) async fn search_for(
    api: &Api,
    a: &Actor,
    p: &str,
    q: &str,
    limit: u32,
) -> Result<Value, Failure> {
    if !wk_contracts::valid_id(p)
        || q.trim().is_empty()
        || q.len() > 1024
        || !(1..=20).contains(&limit)
    {
        return Err(Error::Invalid.into());
    }
    if wk_summarize::sensitive(q) {
        return Err(Failure(StatusCode::UNPROCESSABLE_ENTITY, "sensitive_query"));
    }
    // Authorize before provider use; final candidate SQL rechecks after inference.
    api.knowledge.policy(a, p).await?;
    let reason = if let Some(client) = &api.embeddings {
        match api.knowledge.semantic_ready(a, p, &client.namespace).await {
            Ok(true) => match client.embed_query(q).await {
                Ok(vector) => match api
                    .knowledge
                    .search_hybrid(a, p, q, limit, &client.namespace, &vector)
                    .await
                {
                    Ok(cards) => return Ok(json!({"cards":cards,"retrieval":{"mode":"hybrid"}})),
                    Err(Error::Storage) => "semantic_storage_unavailable",
                    Err(e) => return Err(e.into()),
                },
                Err(reason) => reason,
            },
            Ok(false) => "index_not_ready",
            Err(Error::Storage) => "semantic_storage_unavailable",
            Err(e) => return Err(e.into()),
        }
    } else {
        "semantic_disabled"
    };
    let cards = api.knowledge.search(a, p, q, limit).await?;
    Ok(json!({"cards":cards,"retrieval":{"mode":"lexical","reason":reason}}))
}
pub async fn index_once(
    k: &Knowledge,
    client: &embeddings::Embeddings,
    org: &str,
    product: &str,
) -> Result<Value, Error> {
    let pending = k.semantic_pending(org, product, &client.namespace).await?;
    let (mut indexed, mut held, mut changed) = (0, 0, 0);
    let selected = pending.len();
    for source in pending {
        match client.embed_document(&source.text).await {
            Ok(vector) => {
                if k.semantic_store(org, product, &client.namespace, &source, &vector)
                    .await?
                {
                    indexed += 1
                } else {
                    changed += 1
                }
            }
            Err(_) => {
                if k.semantic_failed(org, product, &client.namespace, &source)
                    .await?
                {
                    held += 1
                } else {
                    changed += 1
                }
            }
        }
    }
    Ok(json!({"selected":selected,"indexed":indexed,"held":held,"changed":changed}))
}
