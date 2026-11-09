//! Token-only developer accounts created by a product admin. The account is an identity under the
//! reserved issuer `wk-admin-created` plus an ordinary membership, so the developer never signs in:
//! they connect with a personal token (see `personal_tokens`). Authority, the services ceiling and the
//! refusal to touch signed-in developers live in the SQL functions of migrations 20 and 22; this module runs
//! them and the token inserts that must succeed or fail together with them.
use super::*;
use crate::personal_tokens::{token, valid_hash, valid_label, PersonalToken, COLUMNS};
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct AdminUser {
    pub user: String,
    pub services: Vec<String>,
    pub can_review: bool,
    /// An account an admin created (no sign-in). Only these can be issued tokens by an admin.
    pub token_only: bool,
    /// Live tokens of a token-only account. Never a secret or hash.
    pub tokens: Vec<PersonalToken>,
}
#[derive(Debug, Serialize)]
pub struct AdminUsers {
    /// The services this admin may grant.
    pub services: Vec<String>,
    pub users: Vec<AdminUser>,
}
#[derive(Deserialize)]
struct RawUsers {
    services: Vec<String>,
    users: Vec<RawUser>,
}
#[derive(Deserialize)]
struct RawUser {
    user: String,
    services: Vec<String>,
    can_review: bool,
    token_only: bool,
}
fn failure(e: sqlx::Error) -> Error {
    match e.as_database_error().and_then(|e| e.code()).as_deref() {
        Some("42501") => Error::Denied,
        Some("22023" | "22P02" | "23514") => Error::Invalid,
        Some("23505") => Error::Conflict,
        _ => Error::Storage,
    }
}
fn valid_scope(product: &str, services: &[String]) -> bool {
    wk_contracts::valid_id(product)
        && (1..=50).contains(&services.len())
        && services.iter().all(|s| wk_contracts::valid_id(s))
}
fn valid_token_request(label: &str, hash: &str, days: u32) -> bool {
    valid_label(label) && valid_hash(hash) && (1..=365).contains(&days)
}
impl Knowledge {
    /// Is the caller an admin of this product?
    pub async fn is_product_admin(&self, a: &Actor, product: &str) -> Result<bool, Error> {
        if !wk_contracts::valid_id(product) {
            return Ok(false);
        }
        let mut tx = self.actor_tx(a).await?;
        sqlx::query_scalar("SELECT public.wk_is_admin($1)")
            .bind(product)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)
    }
    /// The admin's ceiling and every member inside it, with live token metadata for token-only accounts.
    pub async fn admin_users(&self, a: &Actor, product: &str) -> Result<AdminUsers, Error> {
        if !wk_contracts::valid_id(product) {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        let value: serde_json::Value = sqlx::query_scalar("SELECT public.wk_admin_users($1)")
            .bind(product)
            .fetch_one(&mut *tx)
            .await
            .map_err(failure)?;
        let raw: RawUsers = serde_json::from_value(value).map_err(|_| Error::Storage)?;
        let token_only: Vec<String> = raw
            .users
            .iter()
            .filter(|u| u.token_only)
            .map(|u| u.user.clone())
            .collect();
        let mut tokens: HashMap<String, Vec<PersonalToken>> = HashMap::new();
        for row in sqlx::query(&format!("SELECT {COLUMNS},user_id FROM personal_tokens WHERE org=$1 AND product=$2 AND user_id=ANY($3) AND revoked_at IS NULL AND expires_at>now() ORDER BY created_at DESC,id"))
            .bind(&a.org)
            .bind(product)
            .bind(&token_only)
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?
        {
            tokens
                .entry(row.try_get("user_id").map_err(db)?)
                .or_default()
                .push(token(&row)?);
        }
        tx.commit().await.map_err(db)?;
        Ok(AdminUsers {
            services: raw.services,
            users: raw
                .users
                .into_iter()
                .map(|u| AdminUser {
                    tokens: tokens.remove(&u.user).unwrap_or_default(),
                    user: u.user,
                    services: u.services,
                    can_review: u.can_review,
                    token_only: u.token_only,
                })
                .collect(),
        })
    }
    /// Create a token-only developer and their first token in one transaction: either both exist
    /// afterwards or neither does. `Conflict` when the name is already taken by anyone.
    #[allow(clippy::too_many_arguments)]
    pub async fn admin_create_user(
        &self,
        a: &Actor,
        product: &str,
        user: &str,
        services: &[String],
        can_review: bool,
        label: &str,
        hash: &str,
        days: u32,
    ) -> Result<PersonalToken, Error> {
        let label = label.trim();
        if !valid_scope(product, services) || !valid_token_request(label, hash, days) {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        sqlx::query("SELECT public.wk_admin_create_user($1,$2,$3,$4)")
            .bind(product)
            .bind(user)
            .bind(services)
            .bind(can_review)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        let created = Self::issue_in(&mut tx, a, product, user, label, hash, days, false).await?;
        tx.commit().await.map_err(db)?;
        Ok(created)
    }
    /// Replace a token-only developer's tokens with one new token (a lost token, or rotation).
    #[allow(clippy::too_many_arguments)]
    pub async fn admin_reissue_token(
        &self,
        a: &Actor,
        product: &str,
        user: &str,
        label: &str,
        hash: &str,
        days: u32,
    ) -> Result<PersonalToken, Error> {
        let label = label.trim();
        if !wk_contracts::valid_id(product) || !valid_token_request(label, hash, days) {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        let created = Self::issue_in(&mut tx, a, product, user, label, hash, days, true).await?;
        tx.commit().await.map_err(db)?;
        Ok(created)
    }
    /// Authorize the admin for this target (token-only account inside the ceiling), optionally
    /// revoke the target's live tokens for the product, insert the new one and audit it.
    #[allow(clippy::too_many_arguments)]
    async fn issue_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        admin: &Actor,
        product: &str,
        user: &str,
        label: &str,
        hash: &str,
        days: u32,
        replace: bool,
    ) -> Result<PersonalToken, Error> {
        sqlx::query("SELECT public.wk_admin_token_target($1,$2)")
            .bind(product)
            .bind(user)
            .execute(&mut **tx)
            .await
            .map_err(failure)?;
        let owner = Actor {
            org: admin.org.clone(),
            user: user.into(),
        };
        if replace {
            Self::revoke_personal_tokens_in(tx, &owner.org, user, product).await?;
        }
        let created = Self::insert_personal_token(tx, &owner, product, label, hash, days).await?;
        let action = if replace {
            "admin_token_reissue"
        } else {
            "admin_token_issue"
        };
        Self::token_audit(tx, admin, product, created.id, action, Some(user)).await?;
        Ok(created)
    }
    /// Remove a developer from the product (any member inside the ceiling). Their tokens are
    /// revoked, while the principal identity and name remain reserved so retained data cannot be
    /// adopted by a new account. All in one transaction.
    pub async fn admin_remove_user(
        &self,
        a: &Actor,
        product: &str,
        user: &str,
    ) -> Result<(), Error> {
        if !wk_contracts::valid_id(product) || user.is_empty() || user.len() > 256 {
            return Err(Error::Invalid);
        }
        let mut tx = self.actor_tx(a).await?;
        sqlx::query("SELECT public.wk_admin_member($1,$2,$3,false,true)")
            .bind(product)
            .bind(user)
            .bind(Vec::<String>::new())
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        Self::revoke_personal_tokens_in(&mut tx, &a.org, user, product).await?;
        sqlx::query("SELECT public.wk_admin_delete_user($1,$2)")
            .bind(product)
            .bind(user)
            .execute(&mut *tx)
            .await
            .map_err(failure)?;
        tx.commit().await.map_err(db)
    }
}
