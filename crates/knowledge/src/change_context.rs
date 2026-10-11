//! Deterministic scoped references, never an inferred design intent or an all-clear.
use super::*;
use crate::{
    code_anchors::{covers, union_scopes},
    code_provenance::{decode_value, ProfileRecord},
};
use std::collections::{BTreeMap, BTreeSet};
use wk_contracts::change_context::*;
pub(crate) struct Selection {
    pub brief: ChangeContextBrief,
    #[allow(dead_code)] // Consumed by the following preflight task.
    pub versions: BTreeMap<String, serde_json::Value>,
}
fn overlaps(a: &CodeTarget, b: &CodeTarget) -> bool {
    a.path == b.path
        && match (a.start, a.end, b.start, b.end) {
            (Some(x), Some(y), Some(v), Some(w)) => x <= w && v <= y,
            _ => true,
        }
}
fn matches(targets: &[CodeTarget], wanted: &[CodeTarget]) -> bool {
    targets
        .iter()
        .any(|a| wanted.iter().any(|b| overlaps(a, b)))
}
fn text(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(limit)
        .collect()
}
fn source(
    kind: &str,
    id: impl ToString,
    revision: i64,
    code: Option<String>,
    assurance: &str,
    current: bool,
    time: i64,
) -> SourceRef {
    SourceRef {
        kind: kind.into(),
        id: id.to_string(),
        revision,
        code_revision: code,
        assurance: assurance.into(),
        freshness: if current { "current" } else { "historical" }.into(),
        observed_at_ms: time,
    }
}
impl Knowledge {
    pub async fn change_context(
        &self,
        a: &Actor,
        p: &str,
        r: ChangeContextRequest,
    ) -> Result<ChangeContextBrief, Error> {
        let mut tx = self.actor_tx(a).await?;
        let result = Self::select_context(&mut tx, a, p, &r).await?;
        tx.commit().await.map_err(db)?;
        Ok(result.brief)
    }
    pub(crate) async fn select_context(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        r: &ChangeContextRequest,
    ) -> Result<Selection, Error> {
        r.validate().map_err(|_| Error::Invalid)?;
        if !wk_contracts::valid_id(p) {
            return Err(Error::Invalid);
        }
        let repo = Self::code_repository(tx, a, p, r.repository_id).await?;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp())*1000)::bigint")
                .fetch_one(&mut **tx)
                .await
                .map_err(db)?;
        let mut brief = ChangeContextBrief {
            schema_version: 1,
            repository_id: repo.id,
            base_revision: r.base_revision.clone(),
            targets: r.targets.clone(),
            decision: Decision::NeedsReview,
            reasons: vec![],
            gaps: vec![
                "dependency_coverage_incomplete".into(),
                "activity_not_repository_bound".into(),
            ],
            coverage: "reviewed records only".into(),
            observation_assurance: "client_reported_snapshot".into(),
            items: vec![],
            truncated: false,
            brief_id: None,
            expires_at_ms: None,
        };
        if r.targets.iter().any(|t| {
            repo.high_risk_paths.iter().any(|h| {
                t.path == *h || t.path.starts_with(&format!("{}/", h.trim_end_matches('/')))
            })
        }) {
            brief.reasons.push("high_risk_path".into());
        }
        let repo_source = source(
            "repository",
            repo.id,
            repo.revision,
            None,
            "operator_registered",
            true,
            now,
        );
        let mut versions = BTreeMap::new();
        versions.insert(
            format!("repository:{}", repo.id),
            serde_json::to_value(&repo).map_err(|_| Error::Storage)?,
        );
        let paths: Vec<String> = r.targets.iter().map(|t| t.path.clone()).collect();
        // All scope checks are explicit as well as RLS: owner test connections must not bypass filtering.
        let contributions=sqlx::query("SELECT to_jsonb(c)-'org'-'product' AS record,cm.revision AS map_revision FROM code_contributions c JOIN code_mappings cm ON cm.org=c.org AND cm.product=c.product AND cm.id=c.mapping_id AND cm.repository_id=c.repository_id AND cm.revision=c.mapping_revision AND cm.user_id=c.owner AND NOT cm.revoked JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$3 WHERE c.org=$1 AND c.product=$2 AND c.repository_id=$4 AND c.repository_revision=$5 AND NOT c.revoked AND c.confirmed_by=c.owner AND c.reviewed_by IS NOT NULL AND m.services @> c.services AND m.services @> cm.services AND EXISTS(SELECT 1 FROM jsonb_array_elements(c.targets) t WHERE t->>'path'=ANY($6)) ORDER BY (c.code_revision=$7) DESC,c.id LIMIT 21")
            .bind(&a.org).bind(p).bind(&a.user).bind(repo.id).bind(repo.revision).bind(&paths).bind(&r.base_revision).fetch_all(&mut **tx).await.map_err(db)?;
        if contributions.len() > 20 {
            brief.truncated = true;
        }
        for row in contributions.into_iter().take(20) {
            let c: crate::code_provenance::ContributionRecord =
                decode_value(row.try_get("record").map_err(db)?)?;
            if !matches(&c.targets, &r.targets) {
                continue;
            }
            let current = c.code_revision == r.base_revision;
            let mut sources = vec![
                repo_source.clone(),
                source(
                    "contribution",
                    c.id,
                    c.revision,
                    Some(c.code_revision.clone()),
                    "developer_confirmed_reviewed",
                    current,
                    c.confirmed_at_ms.unwrap_or(now),
                ),
                source(
                    "mapping",
                    c.mapping_id,
                    c.mapping_revision,
                    None,
                    "team_attested",
                    true,
                    now,
                ),
            ];
            let profile = Self::context_profile(tx, a, p, &c.owner, &repo.contact_hosts).await?;
            if let Some(profile) = &profile {
                sources.push(source(
                    "profile",
                    &c.owner,
                    profile.revision,
                    None,
                    "owner_opt_in",
                    true,
                    now,
                ));
            } else {
                brief.gaps.push("contributor_name_unavailable".into());
            }
            brief.items.push(ContextItem {
                role: "contributor".into(),
                user_id: Some(c.owner),
                display_name: profile.as_ref().and_then(|p| p.display_name.clone()),
                contact_url: profile.and_then(|p| p.contact_url),
                text: format!(
                    "Developer-confirmed contribution to {} at {}",
                    target_label(&c.targets),
                    c.code_revision
                ),
                matched_by: "repository_revision_path_range".into(),
                sources,
            });
        }
        let anchors=sqlx::query("SELECT to_jsonb(an)-'org'-'product' AS record,c.body FROM code_rationale_anchors an JOIN cards c ON c.org=an.org AND c.product=an.product AND c.id=an.card_id AND c.revision=an.card_revision AND c.author=an.owner JOIN memberships m ON m.org=an.org AND m.product=an.product AND m.user_id=$3 WHERE an.org=$1 AND an.product=$2 AND an.repository_id=$4 AND an.repository_revision=$5 AND NOT an.revoked AND an.confirmed_by=c.author AND an.reviewed_by IS NOT NULL AND c.published AND NOT c.withdrawn AND c.body->>'lifecycle' NOT IN ('superseded','deprecated') AND m.services @> an.services AND m.services @> c.services AND EXISTS(SELECT 1 FROM jsonb_array_elements(an.targets) t WHERE t->>'path'=ANY($6)) AND (an.contribution_id IS NULL OR EXISTS(SELECT 1 FROM code_contributions co JOIN code_mappings cm ON cm.org=co.org AND cm.product=co.product AND cm.id=co.mapping_id AND cm.revision=co.mapping_revision AND cm.user_id=co.owner AND NOT cm.revoked WHERE co.org=an.org AND co.product=an.product AND co.id=an.contribution_id AND co.revision=an.contribution_revision AND co.repository_revision=$5 AND co.repository_id=an.repository_id AND co.code_revision=an.code_revision AND co.confirmed_by=co.owner AND co.reviewed_by IS NOT NULL AND NOT co.revoked AND m.services @> co.services AND m.services @> cm.services)) ORDER BY (an.code_revision=$7) DESC,an.id LIMIT 21")
            .bind(&a.org).bind(p).bind(&a.user).bind(repo.id).bind(repo.revision).bind(&paths).bind(&r.base_revision).fetch_all(&mut **tx).await.map_err(db)?;
        if anchors.len() > 20 {
            brief.truncated = true;
        }
        let mut card_ids = Vec::new();
        let mut has_reason = false;
        for row in anchors.into_iter().take(20) {
            let anchor: crate::code_provenance::AnchorRecord =
                decode_value(row.try_get("record").map_err(db)?)?;
            if !matches(&anchor.targets, &r.targets) {
                continue;
            }
            let body: CardSubmission = decode_value(row.try_get("body").map_err(db)?)?;
            if body.rationale.chars().count() > 2000
                || body.constraints.join("; ").chars().count() > 1000
            {
                brief.truncated = true;
            }
            let current = anchor.code_revision == r.base_revision;
            has_reason |= current && !body.rationale.trim().is_empty();
            if current {
                card_ids.push(anchor.card_id);
            }
            if r.targets
                .iter()
                .any(|t| !anchor.targets.iter().any(|x| covers(x, t)))
            {
                brief.gaps.push("target_scope_partial".into());
            }
            let profile =
                Self::context_profile(tx, a, p, &anchor.owner, &repo.contact_hosts).await?;
            let mut sources = vec![
                repo_source.clone(),
                source(
                    "rationale_anchor",
                    anchor.id,
                    anchor.revision,
                    Some(anchor.code_revision.clone()),
                    "author_confirmed_reviewed",
                    current,
                    anchor.confirmed_at_ms.unwrap_or(now),
                ),
                source(
                    "card",
                    anchor.card_id,
                    anchor.card_revision,
                    None,
                    "published_reviewed",
                    true,
                    now,
                ),
            ];
            if let Some(profile) = &profile {
                sources.push(source(
                    "profile",
                    &anchor.owner,
                    profile.revision,
                    None,
                    "owner_opt_in",
                    true,
                    now,
                ));
            }
            brief.items.push(ContextItem {
                role: "rationale_author".into(),
                user_id: Some(anchor.owner),
                display_name: profile.as_ref().and_then(|p| p.display_name.clone()),
                contact_url: profile.and_then(|p| p.contact_url),
                text: format!(
                    "Confirmed rationale for {}: {}\nConstraints: {}",
                    target_label(&anchor.targets),
                    text(&body.rationale, 2000),
                    text(&body.constraints.join("; "), 1000)
                ),
                matched_by: "repository_revision_path_range".into(),
                sources,
            });
        }
        if !has_reason {
            brief.reasons.push("reason_not_recorded".into());
        }
        let maintainers=sqlx::query("SELECT cm.id,cm.user_id,cm.targets,cm.revision FROM code_maintainers cm JOIN memberships m ON m.org=cm.org AND m.product=cm.product AND m.user_id=$3 WHERE cm.org=$1 AND cm.product=$2 AND cm.repository_id=$4 AND NOT cm.revoked AND m.services @> cm.services AND EXISTS(SELECT 1 FROM jsonb_array_elements(cm.targets) t WHERE t->>'path'=ANY($5)) ORDER BY cm.id LIMIT 21")
            .bind(&a.org).bind(p).bind(&a.user).bind(repo.id).bind(&paths).fetch_all(&mut **tx).await.map_err(db)?;
        if maintainers.len() > 20 {
            brief.truncated = true;
        }
        for row in maintainers.into_iter().take(20) {
            let targets: Vec<CodeTarget> = decode_value(row.try_get("targets").map_err(db)?)?;
            if !matches(&targets, &r.targets) {
                continue;
            }
            let user: String = row.try_get("user_id").map_err(db)?;
            let profile = Self::context_profile(tx, a, p, &user, &repo.contact_hosts).await?;
            let mut sources = vec![
                repo_source.clone(),
                source(
                    "maintainer",
                    row.try_get::<uuid::Uuid, _>("id").map_err(db)?,
                    row.try_get("revision").map_err(db)?,
                    None,
                    "operator_assigned",
                    true,
                    now,
                ),
            ];
            if let Some(profile) = &profile {
                sources.push(source(
                    "profile",
                    &user,
                    profile.revision,
                    None,
                    "owner_opt_in",
                    true,
                    now,
                ));
            }
            brief.items.push(ContextItem {
                role: "maintainer".into(),
                user_id: Some(user),
                display_name: profile.as_ref().and_then(|p| p.display_name.clone()),
                contact_url: profile.and_then(|p| p.contact_url),
                text: format!("Current maintainer for {}", target_label(&targets)),
                matched_by: "repository_path_range".into(),
                sources,
            });
        }
        // One reviewed relationship hop. Both endpoints require current approved anchors in this repository.
        let edges=sqlx::query("SELECT e.source,e.target,e.kind,sc.id AS source_card,sc.revision AS source_revision,tc.id AS target_card,tc.revision AS target_revision,sc.body->>'title' AS source_title,tc.body->>'title' AS target_title FROM graph_edges e JOIN graph_nodes sn ON sn.org=e.org AND sn.product=e.product AND sn.id=e.source AND sn.kind='feature' JOIN graph_nodes tn ON tn.org=e.org AND tn.product=e.product AND tn.id=e.target AND tn.kind='feature' JOIN cards sc ON sc.org=sn.org AND sc.product=sn.product AND sc.id=sn.card_id JOIN cards tc ON tc.org=tn.org AND tc.product=tn.product AND tc.id=tn.card_id JOIN memberships m ON m.org=e.org AND m.product=e.product AND m.user_id=$3 WHERE e.org=$1 AND e.product=$2 AND e.kind IN ('depends_on','contradicts') AND (sc.id=ANY($4) OR tc.id=ANY($4)) AND sc.published AND tc.published AND NOT sc.withdrawn AND NOT tc.withdrawn AND sc.body->>'lifecycle' NOT IN ('superseded','deprecated') AND tc.body->>'lifecycle' NOT IN ('superseded','deprecated') AND sn.provenance->>'card_revision'=sc.revision::text AND tn.provenance->>'card_revision'=tc.revision::text AND m.services @> sc.services AND m.services @> tc.services AND NOT EXISTS(SELECT 1 FROM (VALUES(sc.id,sc.revision),(tc.id,tc.revision)) endpoint(id,revision) WHERE NOT EXISTS(SELECT 1 FROM code_rationale_anchors an WHERE an.org=e.org AND an.product=e.product AND an.card_id=endpoint.id AND an.card_revision=endpoint.revision AND an.repository_id=$5 AND an.repository_revision=$6 AND an.code_revision=$7 AND an.confirmed_by=an.owner AND an.reviewed_by IS NOT NULL AND NOT an.revoked AND m.services @> an.services AND (an.contribution_id IS NULL OR EXISTS(SELECT 1 FROM code_contributions co JOIN code_mappings cm ON cm.org=co.org AND cm.product=co.product AND cm.id=co.mapping_id AND cm.revision=co.mapping_revision AND NOT cm.revoked AND cm.user_id=co.owner WHERE co.org=an.org AND co.product=an.product AND co.id=an.contribution_id AND co.revision=an.contribution_revision AND co.confirmed_by=co.owner AND co.reviewed_by IS NOT NULL AND NOT co.revoked AND m.services @> co.services AND m.services @> cm.services)))) ORDER BY e.source,e.target,e.kind LIMIT 21")
            .bind(&a.org).bind(p).bind(&a.user).bind(&card_ids).bind(repo.id).bind(repo.revision).bind(&r.base_revision).fetch_all(&mut **tx).await.map_err(db)?;
        if edges.len() > 20 {
            brief.truncated = true;
        }
        for row in edges.into_iter().take(20) {
            let kind: String = row.try_get("kind").map_err(db)?;
            if kind == "contradicts" {
                brief
                    .reasons
                    .push("conflicting_recorded_constraints".into());
            }
            brief.items.push(ContextItem {
                role: "impact".into(),
                user_id: None,
                display_name: None,
                contact_url: None,
                text: format!(
                    "{} {} {}",
                    text(&row.try_get::<String, _>("source_title").map_err(db)?, 200),
                    kind,
                    text(&row.try_get::<String, _>("target_title").map_err(db)?, 200)
                ),
                matched_by: "reviewed_relationship_one_hop".into(),
                sources: vec![
                    repo_source.clone(),
                    source(
                        "relationship",
                        format!(
                            "{}:{}:{kind}",
                            row.try_get::<uuid::Uuid, _>("source").map_err(db)?,
                            row.try_get::<uuid::Uuid, _>("target").map_err(db)?
                        ),
                        1,
                        Some(r.base_revision.clone()),
                        "scoped_reviewer_recorded",
                        true,
                        now,
                    ),
                    source(
                        "card",
                        row.try_get::<uuid::Uuid, _>("source_card").map_err(db)?,
                        row.try_get("source_revision").map_err(db)?,
                        None,
                        "published_reviewed",
                        true,
                        now,
                    ),
                    source(
                        "card",
                        row.try_get::<uuid::Uuid, _>("target_card").map_err(db)?,
                        row.try_get("target_revision").map_err(db)?,
                        None,
                        "published_reviewed",
                        true,
                        now,
                    ),
                ],
            });
        }
        let evidence=sqlx::query("SELECT te.id,te.card_revision,te.code_revision,te.reference,te.stale FROM trusted_evidence te JOIN cards c ON c.org=te.org AND c.product=te.product AND c.id=te.card_id JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$3 WHERE te.org=$1 AND te.product=$2 AND te.card_id=ANY($4) AND te.kind='tests_verified' AND c.published AND NOT c.withdrawn AND m.services @> c.services AND te.card_revision=c.revision ORDER BY (te.code_revision=$5 AND NOT te.stale) DESC,te.id LIMIT 21")
            .bind(&a.org).bind(p).bind(&a.user).bind(&card_ids).bind(&r.base_revision).fetch_all(&mut **tx).await.map_err(db)?;
        if evidence.len() > 20 {
            brief.truncated = true;
        }
        let mut has_tests = false;
        for row in evidence.into_iter().take(20) {
            let code: String = row.try_get("code_revision").map_err(db)?;
            let current =
                code == r.base_revision && !row.try_get::<bool, _>("stale").map_err(db)?;
            has_tests |= current;
            brief.items.push(ContextItem {
                role: "test".into(),
                user_id: None,
                display_name: None,
                contact_url: None,
                text: format!(
                    "Test reference: {}",
                    text(&row.try_get::<String, _>("reference").map_err(db)?, 512)
                ),
                matched_by: "anchored_card_test_reference".into(),
                sources: vec![source(
                    "test_evidence",
                    row.try_get::<uuid::Uuid, _>("id").map_err(db)?,
                    row.try_get("card_revision").map_err(db)?,
                    Some(code),
                    "trusted_connector",
                    current,
                    now,
                )],
            });
        }
        if !has_tests {
            brief.gaps.push("current_tests_not_recorded".into());
        }
        let mut sources = BTreeSet::new();
        let mut kept = Vec::new();
        for item in std::mem::take(&mut brief.items) {
            for s in &item.sources {
                versions.insert(format!("{}:{}",s.kind,s.id),serde_json::json!({"revision":s.revision,"code_revision":s.code_revision,"freshness":s.freshness}));
            }
            let next = union_scopes(
                &sources.iter().cloned().collect::<Vec<String>>(),
                &item
                    .sources
                    .iter()
                    .map(|s| format!("{}:{}", s.kind, s.id))
                    .collect::<Vec<_>>(),
            );
            if next.len() > 20 {
                brief.truncated = true;
                continue;
            }
            sources = next.into_iter().collect();
            kept.push(item);
        }
        brief.items = kept;
        brief.reasons.sort();
        brief.reasons.dedup();
        brief.gaps.sort();
        brief.gaps.dedup();
        let original_bytes = serde_json::to_vec(&brief)
            .map_err(|_| Error::Storage)?
            .len();
        if original_bytes > r.budget_bytes as usize || brief.truncated {
            brief.truncated = true;
            brief.gaps.push("context_truncated".into());
        }
        while serde_json::to_vec(&brief)
            .map_err(|_| Error::Storage)?
            .len()
            > r.budget_bytes as usize
        {
            if brief.items.pop().is_none() {
                return Err(Error::Invalid);
            }
        }
        Ok(Selection { brief, versions })
    }
    async fn context_profile(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        user: &str,
        hosts: &[String],
    ) -> Result<Option<ProfileRecord>, Error> {
        let value:Option<serde_json::Value>=sqlx::query_scalar("SELECT to_jsonb(cp)-'org'-'product' FROM code_profiles cp JOIN memberships m ON m.org=cp.org AND m.product=cp.product AND m.user_id=$3 WHERE cp.org=$1 AND cp.product=$2 AND cp.user_id=$4 AND NOT cp.revoked AND cp.display_name IS NOT NULL AND m.services @> cp.services")
            .bind(&a.org).bind(p).bind(&a.user).bind(user).fetch_optional(&mut **tx).await.map_err(db)?;
        let Some(value) = value else {
            return Ok(None);
        };
        let mut result: ProfileRecord = decode_value(value)?;
        result.display_name = result.display_name.map(|s| text(&s, 128));
        result.contact_url = result.contact_url.filter(|s| {
            url::Url::parse(s).is_ok_and(|u| {
                u.scheme() == "https"
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.query().is_none()
                    && u.fragment().is_none()
                    && u.port().is_none()
                    && hosts.iter().any(|h| Some(h.as_str()) == u.host_str())
                    && u.as_str() == s
            })
        });
        Ok(Some(result))
    }
}
fn target_label(targets: &[CodeTarget]) -> String {
    targets
        .iter()
        .take(3)
        .map(|t| match (t.start, t.end) {
            (Some(a), Some(b)) => format!("{}:{a}-{b}", t.path),
            _ => t.path.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
