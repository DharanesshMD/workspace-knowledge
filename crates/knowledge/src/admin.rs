//! Narrow metadata administration, independent of review/content authorization.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductPolicy {
    #[schemars(range(min = 1, max = 30))]
    pub private_days: u16,
    #[schemars(range(min = 1, max = 7))]
    pub report_days: u16,
    pub allow_loopback: bool,
    #[schemars(length(max = 16))]
    pub https_origins: Vec<String>,
}
impl Default for ProductPolicy {
    fn default() -> Self {
        Self {
            private_days: 30,
            report_days: 7,
            allow_loopback: true,
            https_origins: vec![],
        }
    }
}
impl ProductPolicy {
    pub fn validate(&self) -> Result<(), Error> {
        if !(1..=30).contains(&self.private_days)
            || !(1..=7).contains(&self.report_days)
            || self.https_origins.len() > 16
        {
            return Err(Error::Invalid);
        }
        for origin in &self.https_origins {
            let u = url::Url::parse(origin).map_err(|_| Error::Invalid)?;
            if u.scheme() != "https"
                || u.origin().ascii_serialization() != *origin
                || !u.username().is_empty()
                || u.password().is_some()
                || origin.len() > 256
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberChange {
    pub user: String,
    pub services: Vec<String>,
    pub can_review: bool,
}
#[derive(Debug, Serialize)]
pub struct MemberMetadata {
    pub user: String,
    pub services: Vec<String>,
    pub can_review: bool,
    pub policy_revision: i64,
}
fn failure(e: sqlx::Error) -> Error {
    match e.as_database_error().and_then(|e| e.code()).as_deref() {
        Some("42501") => Error::Denied,
        Some("22023" | "22P02" | "23514") => Error::Invalid,
        _ => Error::Storage,
    }
}
fn scope(p: &str, services: &[String]) -> Result<(), Error> {
    if !wk_contracts::valid_id(p)
        || services.is_empty()
        || services.len() > 50
        || services.iter().any(|s| !wk_contracts::valid_id(s))
    {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
impl Knowledge {
    // Match wk_admin_ceiling's gate before taking role/device/membership rows.
    // Bootstrap and authenticated administration must use the same ordering.
    pub(crate) async fn lock_administration(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
    ) -> Result<(), Error> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(jsonb_build_array($1::text,$2::text,'administration')::text,0))")
            .bind(&a.org).bind(p).execute(&mut **tx).await.map_err(db)?;
        Ok(())
    }
    // Shared form of the same gate for transactions that only read. Every writer and
    // administration change (including wk_admin_ceiling in SQL) holds it exclusively, so
    // a reader still never overlaps a change to roles, devices or memberships, while
    // concurrent readers no longer queue behind each other. Never upgrade it in place:
    // a transaction that may write must take the exclusive gate first.
    pub(crate) async fn share_administration(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
    ) -> Result<(), Error> {
        sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended(jsonb_build_array($1::text,$2::text,'administration')::text,0))")
            .bind(&a.org).bind(p).execute(&mut **tx).await.map_err(db)?;
        Ok(())
    }
    /// Protected operator bootstrap; the application role cannot write this table.
    pub async fn admin_assign(&self, a: &Actor, p: &str, services: &[String]) -> Result<(), Error> {
        scope(p, services)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        Self::lock_administration(&mut tx, a, p).await?;
        let allowed=sqlx::query("SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 AND services @> $4::text[] FOR SHARE").bind(&a.org).bind(p).bind(&a.user).bind(services).fetch_optional(&mut *tx).await.map_err(failure)?;
        if allowed.is_none() {
            return Err(Error::Denied);
        }
        sqlx::query("INSERT INTO product_admins(org,product,user_id,services) VALUES($1,$2,$3,$4) ON CONFLICT(org,product,user_id) DO UPDATE SET services=EXCLUDED.services").bind(&a.org).bind(p).bind(&a.user).bind(services).execute(&mut *tx).await.map_err(failure)?;
        sqlx::query("INSERT INTO audit_events(org,product,user_id,action,entity_id,target_user) VALUES($1,$2,current_user::text,'operator_admin_assign','00000000-0000-0000-0000-000000000000',$3)").bind(&a.org).bind(p).bind(&a.user).execute(&mut *tx).await.map_err(failure)?;
        tx.commit().await.map_err(db)
    }
    pub async fn admin_members(&self, a: &Actor, p: &str) -> Result<Vec<MemberMetadata>, Error> {
        if !wk_contracts::valid_id(p) {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        let rows = sqlx::query("SELECT * FROM public.wk_admin_members($1)")
            .bind(p)
            .fetch_all(&mut *tx)
            .await
            .map_err(failure)?;
        rows.into_iter()
            .map(|r| {
                Ok(MemberMetadata {
                    user: r.try_get("user_id").map_err(db)?,
                    services: r.try_get("services").map_err(db)?,
                    can_review: r.try_get("can_review").map_err(db)?,
                    policy_revision: r.try_get("policy_revision").map_err(db)?,
                })
            })
            .collect()
    }
    pub async fn admin_member_put(
        &self,
        a: &Actor,
        p: &str,
        change: MemberChange,
    ) -> Result<(), Error> {
        scope(p, &change.services)?;
        self.admin_member(
            a,
            p,
            &change.user,
            &change.services,
            change.can_review,
            false,
        )
        .await
    }
    pub async fn admin_member_remove(&self, a: &Actor, p: &str, user: &str) -> Result<(), Error> {
        self.admin_member(a, p, user, &[], false, true).await
    }
    async fn admin_member(
        &self,
        a: &Actor,
        p: &str,
        user: &str,
        services: &[String],
        review: bool,
        remove: bool,
    ) -> Result<(), Error> {
        if !wk_contracts::valid_id(p) || user.is_empty() || user.len() > 256 {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        sqlx::query("SELECT public.wk_admin_member($1,$2,$3,$4,$5)")
            .bind(p)
            .bind(user)
            .bind(services)
            .bind(review)
            .bind(remove)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        if remove {
            Self::revoke_personal_tokens_in(&mut tx, &a.org, user, p).await?;
        }
        tx.commit().await.map_err(db)
    }
    pub async fn admin_device_revoke(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
    ) -> Result<(), Error> {
        let mut tx = self.actor_tx(a).await?;
        sqlx::query("SELECT public.wk_admin_device($1,$2)")
            .bind(p)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        tx.commit().await.map_err(db)
    }
    pub async fn admin_policy_put(
        &self,
        a: &Actor,
        p: &str,
        policy: ProductPolicy,
    ) -> Result<(), Error> {
        policy.validate()?;
        let mut tx = self.actor_tx(a).await?;
        sqlx::query("SELECT public.wk_admin_policy($1,$2)")
            .bind(p)
            .bind(serde_json::to_value(policy).map_err(|_| Error::Invalid)?)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        tx.commit().await.map_err(db)
    }
    /// Only the protected database owner can execute the policy helper directly.
    pub async fn admin_policy_bootstrap(
        &self,
        a: &Actor,
        p: &str,
        policy: ProductPolicy,
    ) -> Result<(), Error> {
        policy.validate()?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        Self::lock_administration(&mut tx, a, p).await?;
        sqlx::query(
            "SELECT 1 FROM registered_devices WHERE org=$1 AND product=$2 ORDER BY id FOR UPDATE",
        )
        .bind(&a.org)
        .bind(p)
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        sqlx::query(
            "SELECT 1 FROM memberships WHERE org=$1 AND product=$2 ORDER BY user_id FOR UPDATE",
        )
        .bind(&a.org)
        .bind(p)
        .execute(&mut *tx)
        .await
        .map_err(failure)?;
        sqlx::query("SELECT public.wk_product_policy_apply($1,$2,$3,$4,false)")
            .bind(&a.org)
            .bind(p)
            .bind(&a.user)
            .bind(serde_json::to_value(policy).map_err(|_| Error::Invalid)?)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        tx.commit().await.map_err(db)
    }
    pub(crate) async fn product_restrictions(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
    ) -> Result<ProductPolicy, Error> {
        let v: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT body FROM product_settings WHERE org=$1 AND product=$2")
                .bind(&a.org)
                .bind(p)
                .fetch_optional(&mut **tx)
                .await
                .map_err(db)?;
        v.map(|v| serde_json::from_value(v).map_err(|_| Error::Storage))
            .unwrap_or_else(|| Ok(ProductPolicy::default()))
    }
}
