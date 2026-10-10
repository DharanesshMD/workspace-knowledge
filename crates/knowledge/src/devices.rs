//! Owner-only operational metadata and transaction-scoped collector authorization.
use super::*;
use wk_contracts::device::*;
#[derive(Clone, Copy, Debug)]
pub enum DevicePermission {
    Read,
    SharedUpload,
    OwnerUpload,
    ActivityUpload,
}
#[derive(Clone)]
pub(crate) struct DeviceGuard {
    pub id: uuid::Uuid,
    pub product: String,
    pub services: Vec<String>,
    pub approval_hash: String,
    pub public_key: String,
    pub permission: DevicePermission,
}
#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: uuid::Uuid,
    pub product: String,
    pub services: Vec<String>,
    pub settings: DeviceSettings,
    pub revision: i64,
    pub revoked: bool,
    pub created_at_ms: i64,
    pub report: Option<DeviceReport>,
    pub reported_at_ms: Option<i64>,
    #[serde(skip)]
    pub public_key: String,
    #[serde(skip)]
    pub approval_hash: String,
}
const FIELDS:&str="d.id,d.product,d.services,d.settings,d.revision,d.revoked_at IS NOT NULL AS revoked,(extract(epoch FROM d.created_at)*1000)::bigint AS created_at_ms,d.public_key,d.approval_hash,CASE WHEN r.expires_at>now() THEN r.body END AS report,CASE WHEN r.expires_at>now() THEN (extract(epoch FROM r.reported_at)*1000)::bigint END AS reported_at_ms";
fn decode_device(r: sqlx::postgres::PgRow) -> Result<Device, Error> {
    let report: Option<serde_json::Value> = r.try_get("report").map_err(db)?;
    let report = report
        .and_then(|v| serde_json::from_value::<DeviceReport>(v).ok())
        .filter(|r| r.validate().is_ok());
    Ok(Device {
        id: r.try_get("id").map_err(db)?,
        product: r.try_get("product").map_err(db)?,
        services: r.try_get("services").map_err(db)?,
        settings: serde_json::from_value(r.try_get("settings").map_err(db)?)
            .map_err(|_| Error::Storage)?,
        revision: r.try_get("revision").map_err(db)?,
        revoked: r.try_get("revoked").map_err(db)?,
        created_at_ms: r.try_get("created_at_ms").map_err(db)?,
        reported_at_ms: if report.is_some() {
            r.try_get("reported_at_ms").map_err(db)?
        } else {
            None
        },
        report,
        public_key: r.try_get("public_key").map_err(db)?,
        approval_hash: r.try_get("approval_hash").map_err(db)?,
    })
}
impl Knowledge {
    pub fn device_bound(&self, d: &Device, permission: DevicePermission) -> Self {
        let mut k = self.clone();
        k.device_guard = Some(DeviceGuard {
            id: d.id,
            product: d.product.clone(),
            services: d.services.clone(),
            approval_hash: d.approval_hash.clone(),
            public_key: d.public_key.clone(),
            permission,
        });
        k
    }
    pub(crate) fn device_scope(&self, p: &str, services: &[String]) -> Result<(), Error> {
        if self
            .device_guard
            .as_ref()
            .is_some_and(|g| g.product != p || services.iter().any(|s| !g.services.contains(s)))
        {
            return Err(Error::Denied);
        }
        Ok(())
    }
    pub(crate) async fn lock_device(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
    ) -> Result<(), Error> {
        let Some(g) = &self.device_guard else {
            return Ok(());
        };
        let r=sqlx::query("SELECT services,settings FROM registered_devices WHERE id=$1 AND org=$2 AND owner=$3 AND product=$4 AND public_key=$5 AND approval_hash=$6 AND revoked_at IS NULL FOR SHARE").bind(g.id).bind(&a.org).bind(&a.user).bind(&g.product).bind(&g.public_key).bind(&g.approval_hash).fetch_optional(&mut **tx).await.map_err(db)?.ok_or(Error::Denied)?;
        let services: Vec<String> = r.try_get("services").map_err(db)?;
        if services != g.services {
            return Err(Error::Denied);
        }
        let settings: DeviceSettings = serde_json::from_value(r.try_get("settings").map_err(db)?)
            .map_err(|_| Error::Storage)?;
        if !matches!(g.permission, DevicePermission::Read)
            && (settings.paused
                || matches!(g.permission, DevicePermission::OwnerUpload) && !settings.owner_sync
                || matches!(g.permission, DevicePermission::ActivityUpload) && !settings.activity)
        {
            return Err(Error::Denied);
        }
        if sqlx::query("SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 AND services @> $4::text[] FOR SHARE").bind(&a.org).bind(&g.product).bind(&a.user).bind(&g.services).fetch_optional(&mut **tx).await.map_err(db)?.is_none(){return Err(Error::Denied)}
        Ok(())
    }
    pub async fn device_register(&self, a: &Actor, r: DeviceRegistration) -> Result<Device, Error> {
        self.device_register_inner(a, r, None).await
    }
    pub async fn device_register_credential(
        &self,
        a: &Actor,
        r: DeviceRegistration,
        hash: &str,
        expiry: i64,
    ) -> Result<Device, Error> {
        self.device_register_inner(a, r, Some((hash, expiry))).await
    }
    async fn credential_lock(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        id: uuid::Uuid,
        hash: &str,
    ) -> Result<(), Error> {
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(Error::Invalid);
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(jsonb_build_array($1::text,$2::text,'collector-credentials')::text,0))").bind(&a.org).bind(&a.user).execute(&mut **tx).await.map_err(db)?;
        sqlx::query(
            "DELETE FROM collector_credentials WHERE org=$1 AND owner=$2 AND expires_at<now()",
        )
        .bind(&a.org)
        .bind(&a.user)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
        let existing:Option<uuid::Uuid>=sqlx::query_scalar("SELECT device_id FROM collector_credentials WHERE org=$1 AND owner=$2 AND token_hash=$3").bind(&a.org).bind(&a.user).bind(hash).fetch_optional(&mut **tx).await.map_err(db)?;
        if existing.is_some_and(|d| d != id) {
            return Err(Error::Denied);
        }
        if existing.is_none()
            && sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM collector_credentials WHERE org=$1 AND owner=$2",
            )
            .bind(&a.org)
            .bind(&a.user)
            .fetch_one(&mut **tx)
            .await
            .map_err(db)?
                >= 1000
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    async fn credential_save(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        id: uuid::Uuid,
        hash: &str,
        expiry: i64,
    ) -> Result<(), Error> {
        sqlx::query("INSERT INTO collector_credentials(org,owner,token_hash,device_id,expires_at) VALUES($1,$2,$3,$4,to_timestamp($5)) ON CONFLICT(org,owner,token_hash) DO NOTHING").bind(&a.org).bind(&a.user).bind(hash).bind(id).bind(expiry as f64).execute(&mut **tx).await.map_err(db)?;
        Ok(())
    }
    pub async fn device_bind_credential(
        &self,
        a: &Actor,
        d: &Device,
        hash: &str,
        expiry: i64,
    ) -> Result<(), Error> {
        let mut tx = self.actor_tx(a).await?;
        Self::credential_lock(&mut tx, a, d.id, hash).await?;
        self.device_bound(d, DevicePermission::Read)
            .lock_device(&mut tx, a)
            .await?;
        Self::credential_save(&mut tx, a, d.id, hash, expiry).await?;
        tx.commit().await.map_err(db)
    }
    async fn device_register_inner(
        &self,
        a: &Actor,
        mut r: DeviceRegistration,
        credential: Option<(&str, i64)>,
    ) -> Result<Device, Error> {
        r.validate().map_err(|_| Error::Invalid)?;
        r.services.sort();
        r.services.dedup();
        let mut tx = self.actor_tx(a).await?;
        if let Some((hash, _)) = credential {
            Self::credential_lock(&mut tx, a, r.id, hash).await?;
        }
        let capacity: bool = sqlx::query_scalar("SELECT public.wk_device_has_capacity($1,$2)")
            .bind(&a.org)
            .bind(&a.user)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        let existing=sqlx::query("SELECT id,product,services,public_key,approval_hash,revoked_at IS NOT NULL AS revoked FROM registered_devices WHERE org=$1 AND owner=$2 AND (id=$3 OR public_key=$4) FOR UPDATE").bind(&a.org).bind(&a.user).bind(r.id).bind(&r.public_key).fetch_optional(&mut *tx).await.map_err(db)?;
        if let Some(row) = existing {
            let same = row.try_get::<uuid::Uuid, _>("id").map_err(db)? == r.id
                && row.try_get::<String, _>("product").map_err(db)? == r.product
                && row.try_get::<Vec<String>, _>("services").map_err(db)? == r.services
                && row.try_get::<String, _>("public_key").map_err(db)? == r.public_key
                && row.try_get::<String, _>("approval_hash").map_err(db)? == r.approval_hash
                && !row.try_get::<bool, _>("revoked").map_err(db)?;
            if !same {
                return Err(Error::Denied);
            }
        } else {
            if !capacity {
                return Err(Error::Conflict);
            }
            let member=sqlx::query("SELECT 1 FROM memberships WHERE org=$1 AND product=$2 AND user_id=$3 AND services @> $4::text[] FOR SHARE").bind(&a.org).bind(&r.product).bind(&a.user).bind(&r.services).fetch_optional(&mut *tx).await.map_err(db)?;
            if member.is_none() {
                return Err(Error::Denied);
            }
            sqlx::query("INSERT INTO registered_devices(id,org,product,owner,public_key,approval_hash,services) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(r.id).bind(&a.org).bind(&r.product).bind(&a.user).bind(&r.public_key).bind(&r.approval_hash).bind(&r.services).execute(&mut *tx).await.map_err(|e|if e.as_database_error().is_some_and(|e|e.code().as_deref()==Some("23505")){Error::Denied}else{db(e)})?;
            Self::device_audit(&mut tx, a, &r.product, r.id, "device_register").await?;
        }
        if let Some((hash, expiry)) = credential {
            Self::credential_save(&mut tx, a, r.id, hash, expiry).await?;
        }
        tx.commit().await.map_err(db)?;
        self.device_get(a, &r.product, r.id).await
    }
    async fn device_audit(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        action: &str,
    ) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO audit_events(org,product,user_id,action,entity_id) VALUES($1,$2,$3,$4,$5)",
        )
        .bind(&a.org)
        .bind(p)
        .bind(&a.user)
        .bind(action)
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
        Ok(())
    }
    pub async fn device_get(&self, a: &Actor, p: &str, id: uuid::Uuid) -> Result<Device, Error> {
        let mut tx = self.actor_tx(a).await?;
        decode_device(sqlx::query(&format!("SELECT {FIELDS} FROM registered_devices d LEFT JOIN device_reports r ON r.device_id=d.id JOIN memberships m ON m.org=d.org AND m.product=d.product AND m.user_id=d.owner WHERE d.id=$1 AND d.org=$2 AND d.product=$3 AND d.owner=$4 AND m.services @> d.services")).bind(id).bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?)
    }
    pub async fn device_list(&self, a: &Actor, p: &str) -> Result<Vec<Device>, Error> {
        let mut tx = self.actor_tx(a).await?;
        sqlx::query(&format!("SELECT {FIELDS} FROM registered_devices d LEFT JOIN device_reports r ON r.device_id=d.id JOIN memberships m ON m.org=d.org AND m.product=d.product AND m.user_id=d.owner WHERE d.org=$1 AND d.product=$2 AND d.owner=$3 AND m.services @> d.services ORDER BY (d.revoked_at IS NOT NULL),d.created_at DESC,d.id LIMIT 20")).bind(&a.org).bind(p).bind(&a.user).fetch_all(&mut *tx).await.map_err(db)?.into_iter().map(decode_device).collect()
    }
    async fn change_device(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        revision: i64,
        settings: Option<DeviceSettings>,
    ) -> Result<Device, Error> {
        if revision < 1 {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        let current=sqlx::query("SELECT revision,revoked_at IS NOT NULL AS revoked FROM registered_devices d WHERE id=$1 AND org=$2 AND product=$3 AND owner=$4 AND EXISTS(SELECT 1 FROM memberships m WHERE m.org=d.org AND m.product=d.product AND m.user_id=d.owner AND m.services @> d.services) FOR UPDATE").bind(id).bind(&a.org).bind(p).bind(&a.user).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::NotFound)?;
        if current.try_get::<i64, _>("revision").map_err(db)? != revision {
            return Err(Error::Conflict);
        }
        if current.try_get::<bool, _>("revoked").map_err(db)? {
            return Err(Error::Denied);
        }
        let audit_action = if settings.is_some() {
            "device_settings"
        } else {
            "device_revoke"
        };
        match settings {
            Some(s) => {
                sqlx::query(
                    "UPDATE registered_devices SET settings=$2,revision=revision+1 WHERE id=$1",
                )
                .bind(id)
                .bind(serde_json::to_value(s).map_err(|_| Error::Invalid)?)
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            }
            None => {
                sqlx::query("UPDATE registered_devices SET revoked_at=now(),revision=revision+1 WHERE id=$1").bind(id).execute(&mut *tx).await.map_err(db)?;
            }
        }
        Self::device_audit(&mut tx, a, p, id, audit_action).await?;
        tx.commit().await.map_err(db)?;
        self.device_get(a, p, id).await
    }
    pub async fn device_change(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        revision: i64,
        settings: DeviceSettings,
    ) -> Result<Device, Error> {
        self.change_device(a, p, id, revision, Some(settings)).await
    }
    pub async fn device_revoke(
        &self,
        a: &Actor,
        p: &str,
        id: uuid::Uuid,
        revision: i64,
    ) -> Result<Device, Error> {
        self.change_device(a, p, id, revision, None).await
    }
    pub async fn device_report(&self, a: &Actor, d: &Device, r: DeviceReport) -> Result<(), Error> {
        r.validate().map_err(|_| Error::Invalid)?;
        let value = serde_json::to_value(&r).map_err(|_| Error::Invalid)?;
        validate_shared_body(&value)?;
        let k = self.device_bound(d, DevicePermission::Read);
        let mut tx = k.actor_tx(a).await?;
        let revision: i64 =
            sqlx::query_scalar("SELECT revision FROM registered_devices WHERE id=$1")
                .bind(d.id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if revision != r.config_revision {
            return Err(Error::Conflict);
        }
        sqlx::query("INSERT INTO device_reports(device_id,org,owner,body) VALUES($1,$2,$3,$4) ON CONFLICT(device_id) DO UPDATE SET body=EXCLUDED.body,reported_at=now(),expires_at=now()+interval '7 days' WHERE device_reports.org=EXCLUDED.org AND device_reports.owner=EXCLUDED.owner").bind(d.id).bind(&a.org).bind(&a.user).bind(value).execute(&mut *tx).await.map_err(db)?;
        tx.commit().await.map_err(db)
    }
    pub async fn device_nonce(
        &self,
        a: &Actor,
        d: &Device,
        nonce: uuid::Uuid,
    ) -> Result<(), Error> {
        let k = self.device_bound(d, DevicePermission::Read);
        let mut tx = k.actor_tx(a).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("device-nonce:{}", d.id))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("DELETE FROM device_nonces WHERE device_id=$1 AND expires_at<now()")
            .bind(d.id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM device_nonces WHERE device_id=$1")
                .bind(d.id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if count >= 1200 {
            return Err(Error::Denied);
        }
        let result=sqlx::query("INSERT INTO device_nonces(device_id,org,owner,nonce) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(d.id).bind(&a.org).bind(&a.user).bind(nonce).execute(&mut *tx).await.map_err(db)?;
        if result.rows_affected() != 1 {
            return Err(Error::Denied);
        }
        tx.commit().await.map_err(db)
    }
}
