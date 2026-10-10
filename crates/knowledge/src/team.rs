//! Published team browsing and explicitly shared activity. Never reads owner summaries.
use super::*;
#[derive(Default, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct TeamQuery {
    pub before: Option<uuid::Uuid>,
    pub author: Option<String>,
    pub service: Option<String>,
    pub since_ms: Option<i64>,
    pub file: Option<String>,
}
#[derive(Serialize, Debug)]
pub struct TeamEntry {
    pub card: Card,
    pub author: String,
    pub created_at_ms: i64,
}
#[derive(Serialize, Debug)]
pub struct SharedActivity {
    pub developer: String,
    pub intent: String,
    pub files: Vec<String>,
    pub services: Vec<String>,
    pub reported_at_ms: i64,
    pub expires_at_ms: i64,
}
fn safe_file(value: &str) -> bool {
    activity::valid_path(value) && !value.contains(':')
}
impl TeamQuery {
    fn validate(&self) -> Result<(), Error> {
        if self
            .author
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
            || self
                .service
                .as_ref()
                .is_some_and(|s| !wk_contracts::valid_id(s))
            || self
                .file
                .as_ref()
                .is_some_and(|s| s.len() > 256 || !safe_file(s))
        {
            return Err(Error::Invalid);
        }
        if let Some(at) = self.since_ms {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Error::Storage)?
                .as_millis() as i64;
            if at < 0 || at > now + 300000 {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}
impl Knowledge {
    pub async fn team_context(
        &self,
        a: &Actor,
        p: &str,
        q: TeamQuery,
    ) -> Result<Vec<TeamEntry>, Error> {
        q.validate()?;
        let mut tx = self.actor_tx(a).await?;
        let rows=sqlx::query("SELECT c.id,c.body,c.revision,c.published,c.author,(extract(epoch from c.created_at)*1000)::bigint AS created_at_ms FROM cards c JOIN memberships m ON m.org=c.org AND m.product=c.product AND m.user_id=$1 WHERE c.org=$2 AND c.product=$3 AND m.services @> c.services AND c.published AND NOT c.withdrawn AND ($4::uuid IS NULL OR (c.created_at,c.id)<(SELECT anchor.created_at,anchor.id FROM cards anchor WHERE anchor.id=$4 AND anchor.org=$2 AND anchor.product=$3 AND m.services @> anchor.services AND anchor.published AND NOT anchor.withdrawn AND ($5::text IS NULL OR anchor.author=$5) AND ($6::text IS NULL OR $6=ANY(anchor.services)) AND ($7::bigint IS NULL OR anchor.created_at>=to_timestamp($7::bigint/1000.0)) AND ($8::text IS NULL OR EXISTS(SELECT 1 FROM jsonb_array_elements(anchor.body->'evidence') e WHERE e->>'kind'='file_observation' AND e->>'reference'=$8)))) AND ($5::text IS NULL OR c.author=$5) AND ($6::text IS NULL OR $6=ANY(c.services)) AND ($7::bigint IS NULL OR c.created_at>=to_timestamp($7::bigint/1000.0)) AND ($8::text IS NULL OR EXISTS(SELECT 1 FROM jsonb_array_elements(c.body->'evidence') e WHERE e->>'kind'='file_observation' AND e->>'reference'=$8)) ORDER BY c.created_at DESC,c.id DESC LIMIT 50")
  .bind(&a.user).bind(&a.org).bind(p).bind(q.before).bind(q.author).bind(q.service).bind(q.since_ms).bind(q.file).fetch_all(&mut *tx).await.map_err(db)?;
        rows.into_iter()
            .map(|row| {
                let author = row.try_get("author").map_err(db)?;
                let created_at_ms = row.try_get("created_at_ms").map_err(db)?;
                Ok(TeamEntry {
                    card: decode(row)?,
                    author,
                    created_at_ms,
                })
            })
            .collect()
    }
    pub async fn team_activity(&self, a: &Actor, p: &str) -> Result<Vec<SharedActivity>, Error> {
        let mut tx = self.actor_tx(a).await?;
        let rows=sqlx::query("SELECT t.user_id,t.intent,t.files,t.services,(extract(epoch from t.last_seen)*1000)::bigint AS reported_at_ms,(extract(epoch from t.expires_at)*1000)::bigint AS expires_at_ms FROM activity t JOIN memberships m ON m.org=t.org AND m.product=t.product AND m.user_id=$1 WHERE t.org=$2 AND t.product=$3 AND m.services @> t.services AND t.status='active' AND t.expires_at>now() ORDER BY t.last_seen DESC,t.user_id LIMIT 50").bind(&a.user).bind(&a.org).bind(p).fetch_all(&mut *tx).await.map_err(db)?;
        let mut reports = Vec::new();
        for row in rows {
            let intent: String = row.try_get("intent").map_err(db)?;
            let files: Vec<String> = row.try_get("files").map_err(db)?;
            if !activity::safe_content(&intent, &files) {
                continue;
            }
            reports.push(SharedActivity {
                developer: row.try_get("user_id").map_err(db)?,
                intent,
                files,
                services: row.try_get("services").map_err(db)?,
                reported_at_ms: row.try_get("reported_at_ms").map_err(db)?,
                expires_at_ms: row.try_get("expires_at_ms").map_err(db)?,
            });
        }
        Ok(reports)
    }
}
