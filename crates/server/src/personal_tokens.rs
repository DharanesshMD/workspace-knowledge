//! Personal MCP tokens: settings, secret handling and the portal routes that manage them. A token
//! authenticates one user to one product's `/mcp/{product}` endpoint only (see `remote_mcp`); every
//! other route keeps rejecting it because it is not a signed identity-provider token.
use crate::{actor, browser_auth, browser_routes, parse, Api, Failure};
use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use wk_knowledge::Actor;

pub const PREFIX: &str = "wkpt_";
const DEFAULT_MAX_DAYS: u32 = 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub max_days: u32,
}
/// `WK_PERSONAL_TOKENS=1` turns the feature on (off when unset); `WK_PERSONAL_TOKEN_MAX_DAYS` caps a
/// token's lifetime (default 90, at most 365). A malformed value fails startup rather than silently
/// enabling or widening anything.
pub fn config(mut get: impl FnMut(&str) -> Option<String>) -> Result<Option<Config>, String> {
    match get("WK_PERSONAL_TOKENS").as_deref().map(str::trim) {
        None | Some("") | Some("0") => return Ok(None),
        Some("1") => {}
        Some(_) => return Err("invalid WK_PERSONAL_TOKENS: expected 1 or 0".into()),
    }
    let max_days = match get("WK_PERSONAL_TOKEN_MAX_DAYS").as_deref().map(str::trim) {
        None | Some("") => DEFAULT_MAX_DAYS,
        Some(v) => v
            .parse::<u32>()
            .ok()
            .filter(|d| (1..=365).contains(d))
            .ok_or("invalid WK_PERSONAL_TOKEN_MAX_DAYS: expected 1 to 365")?,
    };
    Ok(Some(Config { max_days }))
}
/// A fresh token and the hash that is stored for it.
pub fn generate() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = format!("{PREFIX}{}", hex::encode(bytes));
    let hash = browser_auth::hash(&token);
    (token, hash)
}
/// True for text shaped like one of our tokens; says nothing about whether it is valid.
pub fn looks_like_token(value: &str) -> bool {
    value.strip_prefix(PREFIX).is_some_and(|rest| {
        rest.len() == 64
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
/// Who a bearer personal token belongs to for this product, or `None` when the request carries
/// none (so ordinary identity-provider verification proceeds). Unknown, expired, revoked and
/// other-product tokens all produce the same 401.
pub(crate) async fn bearer_actor(
    api: &Api,
    product: &str,
    headers: &HeaderMap,
) -> Option<Result<Actor, Failure>> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| looks_like_token(t))?;
    api.personal_tokens?;
    if let Err(refused) = crate::reject_browser_origin(api, headers) {
        return Some(Err(refused));
    }
    Some(
        api.knowledge
            .personal_token_actor(&browser_auth::hash(token), product)
            .await
            .map_err(|_| Failure(StatusCode::UNAUTHORIZED, "unauthenticated")),
    )
}

pub(crate) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/tokens", get(list).post(create))
        .route("/v1/tokens/{id}", delete(revoke))
}
/// Minting, listing and revoking need a signed-in portal session (cookie, with the origin and CSRF
/// checks `protect` applies). A short-lived identity-provider token or a personal token must not be
/// able to turn itself into a long-lived credential.
pub(crate) async fn portal_actor(api: &Api, headers: &HeaderMap) -> Result<Actor, Failure> {
    if api.personal_tokens.is_none() {
        return Err(Failure(StatusCode::NOT_FOUND, "not_found"));
    }
    if browser_routes::session_cookie(api, headers)?.is_none() {
        return Err(Failure(StatusCode::UNAUTHORIZED, "session_required"));
    }
    actor(api, headers).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    product: String,
    label: String,
    days: u32,
}
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    body: Result<Json<Create>, JsonRejection>,
) -> Result<Response, Failure> {
    let who = portal_actor(&api, &headers).await?;
    let request = parse(body)?;
    let max_days = api.personal_tokens.map_or(0, |c| c.max_days);
    if request.days == 0 || request.days > max_days {
        return Err(Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"));
    }
    let (secret, hash) = generate();
    let created = api
        .knowledge
        .create_personal_token(&who, &request.product, &request.label, &hash, request.days)
        .await?;
    let mut response = (
        StatusCode::CREATED,
        Json(
            json!({"token":secret,"id":created.id,"product":created.product,"label":created.label,
            "created_ms":created.created_ms,"expires_ms":created.expires_ms}),
        ),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
async fn list(State(api): State<Api>, headers: HeaderMap) -> Result<Json<Value>, Failure> {
    let who = portal_actor(&api, &headers).await?;
    Ok(Json(
        json!({"tokens":api.knowledge.list_personal_tokens(&who).await?}),
    ))
}
async fn revoke(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Failure> {
    let who = portal_actor(&api, &headers).await?;
    api.knowledge
        .revoke_personal_token(&who, crate::uuid(&id)?)
        .await?;
    Ok(Json(json!({"revoked":true})))
}
