//! Prompt-time overlap lookup: existing file/text notices plus meaning matches.
//!
//! One path serves HTTP, MCP and device-signed lookups. Meaning matches are best effort: any
//! gate or provider failure keeps the text notices and reports a fixed reason label.
use crate::*;
use std::time::Duration;
use wk_knowledge::activity::{Notice, OverlapQuery};
const PROMPT_BUDGET: Duration = Duration::from_millis(1500);
const MEANING_CANDIDATES: u32 = 20;
const MEANING_ADDED: usize = 5;
/// Reciprocal-rank constant; ranks are zero-based, so the top result scores 1/61.
const RANK_CONSTANT: f64 = 61.;

/// Why meaning matching is skipped before any provider call, if it is.
async fn gate(
    k: &Knowledge,
    a: &Actor,
    p: &str,
    client: &embeddings::Embeddings,
) -> Result<Option<&'static str>, Failure> {
    if !k.policy(a, p).await?.restrictions.allow_loopback {
        return Ok(Some("embedding_policy_disabled"));
    }
    match k.semantic_ready(a, p, &client.namespace).await {
        Ok(true) => Ok(None),
        Ok(false) => Ok(Some("index_not_ready")),
        Err(Error::Storage) => Ok(Some("semantic_storage_unavailable")),
        Err(e) => Err(e.into()),
    }
}

/// `k` may be device-bound; every query, including the meaning candidates, runs through it.
pub(crate) async fn overlaps_with(
    api: &Api,
    k: &Knowledge,
    a: &Actor,
    product: &str,
    mut q: OverlapQuery,
) -> Result<Value, Failure> {
    // Possibly sensitive request text is dropped, not sent to search or the model; file
    // matching still runs.
    let query_ignored = q.query.as_deref().is_some_and(wk_summarize::sensitive);
    if query_ignored {
        q.query = None;
        if q.files.is_empty() {
            return Ok(
                json!({"notices":[],"text":"","query_ignored":true,"retrieval":{"mode":"lexical","reason":null}}),
            );
        }
    }
    let limit = q.limit as usize;
    let text = q.query.clone().filter(|s| !s.trim().is_empty());
    let (notices, meaning) = match (text, &api.embeddings) {
        (None, _) => (k.overlaps(a, product, q).await?, Err(None)),
        (Some(_), None) => (
            k.overlaps(a, product, q).await?,
            Err(Some("semantic_disabled")),
        ),
        (Some(text), Some(client)) => match gate(k, a, product, client).await? {
            Some(reason) => (k.overlaps(a, product, q).await?, Err(Some(reason))),
            None => {
                let (notices, vector) = tokio::join!(
                    k.overlaps(a, product, q),
                    client.embed_query_within(&text, PROMPT_BUDGET)
                );
                (notices?, vector.map(|v| (client, v)).map_err(Some))
            }
        },
    };
    let (fused, reason) = match meaning {
        Err(reason) => (notices, reason),
        // Authorize again after inference; the candidate query rechecks membership too.
        Ok(_) if !k.policy(a, product).await?.restrictions.allow_loopback => {
            (notices, Some("embedding_policy_disabled"))
        }
        Ok((client, vector)) => {
            let shown: Vec<uuid::Uuid> = notices
                .iter()
                .filter(|n| n.kind != "related")
                .filter_map(|n| n.card_id)
                .collect();
            match k
                .related_by_meaning(
                    a,
                    product,
                    &client.namespace,
                    &vector,
                    &shown,
                    MEANING_CANDIDATES,
                )
                .await
            {
                Ok(meaning) => {
                    return Ok(respond(
                        fuse_related(notices, meaning, limit),
                        query_ignored,
                        json!({"mode":"hybrid","reason":null}),
                    ))
                }
                Err(Error::Storage) => (notices, Some("semantic_storage_unavailable")),
                Err(e) => return Err(e.into()),
            }
        }
    };
    Ok(respond(
        fused,
        query_ignored,
        json!({"mode":"lexical","reason":reason}),
    ))
}

fn respond(notices: Vec<Notice>, query_ignored: bool, retrieval: Value) -> Value {
    json!({"text":wk_knowledge::activity::render(&notices),"notices":notices,"query_ignored":query_ignored,"retrieval":retrieval})
}

/// Non-related notices keep their order and come first; related notices are merged by
/// reciprocal rank with ties broken by card ID. Cards found both ways become `both`; at most
/// `MEANING_ADDED` meaning-only cards are added, never one already shown as other work.
pub(crate) fn fuse_related(
    notices: Vec<Notice>,
    meaning: Vec<Notice>,
    limit: usize,
) -> Vec<Notice> {
    let (mut out, text): (Vec<Notice>, Vec<Notice>) =
        notices.into_iter().partition(|n| n.kind != "related");
    let score = |rank: usize| 1. / (RANK_CONSTANT + rank as f64);
    let mut related: Vec<(Notice, f64)> = text
        .into_iter()
        .enumerate()
        .map(|(rank, n)| (n, score(rank)))
        .collect();
    let mut added = 0;
    for (rank, n) in meaning.into_iter().enumerate() {
        let Some(id) = n.card_id else { continue };
        if let Some((found, s)) = related.iter_mut().find(|(t, _)| t.card_id == Some(id)) {
            found.matched_by = "both";
            *s += score(rank);
        } else if added < MEANING_ADDED && !out.iter().any(|o| o.card_id == Some(id)) {
            added += 1;
            related.push((n, score(rank)));
        }
    }
    related.sort_by(|(a, x), (b, y)| y.total_cmp(x).then_with(|| a.card_id.cmp(&b.card_id)));
    out.extend(related.into_iter().map(|(n, _)| n));
    out.truncate(limit);
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    fn notice(kind: &'static str, by: &'static str, id: u128) -> Notice {
        Notice {
            kind,
            matched_by: by,
            developer: "dev".into(),
            summary: format!("s{id}"),
            matched_files: vec![],
            card_id: Some(uuid::Uuid::from_u128(id)),
            revision: Some(1),
            lifecycle: Some("proposed".into()),
            open_questions: vec![],
            when_ms: 0,
            text: format!("t{id}"),
        }
    }
    #[test]
    fn fusion_keeps_other_notices_first_marks_both_and_caps_meaning() {
        let base = vec![
            notice("active", "file", 1),
            notice("related", "text", 2),
            notice("related", "text", 3),
        ];
        let meaning = (2..=9).map(|i| notice("related", "meaning", i)).collect();
        let fused = fuse_related(base, meaning, 20);
        assert_eq!(fused[0].kind, "active");
        let related: Vec<_> = fused[1..]
            .iter()
            .map(|n| (n.card_id.unwrap().as_u128(), n.matched_by))
            .collect();
        assert_eq!(related[0], (2, "both"));
        assert_eq!(
            related.iter().filter(|(_, by)| *by == "meaning").count(),
            5,
            "at most 5 meaning-only"
        );
        assert!(related.contains(&(3, "both")));
    }
    #[test]
    fn fusion_respects_the_limit_and_keeps_text_only_results_without_meaning() {
        let base = vec![notice("related", "text", 2)];
        assert_eq!(fuse_related(base.clone(), vec![], 8)[0].matched_by, "text");
        let many = (1..=10).map(|i| notice("related", "meaning", i)).collect();
        assert_eq!(fuse_related(vec![], many, 3).len(), 3);
    }
    #[test]
    fn fusion_never_repeats_a_card_already_shown_as_other_work() {
        let base = vec![notice("changed", "file", 1)];
        let meaning = vec![
            notice("related", "meaning", 1),
            notice("related", "meaning", 2),
        ];
        let ids: Vec<_> = fuse_related(base, meaning, 8)
            .iter()
            .map(|n| n.card_id.unwrap().as_u128())
            .collect();
        assert_eq!(ids, [1, 2]);
    }
}
