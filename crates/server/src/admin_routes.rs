//! Portal routes for a product admin to create token-only developer accounts and hand out their
//! personal token and MCP address. Like token management these need a signed-in portal session
//! (cookie, origin and CSRF checked) and the personal-token feature; no bearer credential of any kind
//! can reach them, so a leaked token or identity-provider token cannot create accounts.
use crate::{parse, personal_tokens, remote_mcp, Api, Failure};
use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use wk_knowledge::personal_tokens::PersonalToken;

pub(crate) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/admin/products/{product}/users", get(list).post(create))
        .route("/v1/admin/products/{product}/users/{user}", delete(remove))
        .route(
            "/v1/admin/products/{product}/users/{user}/token",
            post(reissue),
        )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    user: String,
    services: Vec<String>,
    #[serde(default)]
    can_review: bool,
    #[serde(default)]
    label: Option<String>,
    days: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reissue {
    #[serde(default)]
    label: Option<String>,
    days: u32,
}
fn days_allowed(api: &Api, days: u32) -> Result<(), Failure> {
    let max = api.personal_tokens.map_or(0, |c| c.max_days);
    if days == 0 || days > max {
        return Err(Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"));
    }
    Ok(())
}
/// The one response that carries a secret: shown once, never cached.
fn issued(
    api: &Api,
    status: StatusCode,
    product: &str,
    user: &str,
    secret: String,
    made: PersonalToken,
) -> Response {
    let mcp_url = remote_mcp::resource(api)
        .ok()
        .filter(|_| wk_contracts::valid_id(product))
        .map(|origin| format!("{origin}/mcp/{product}"));
    let mut response = (
        status,
        Json(
            json!({"user":user,"product":product,"token":secret,"id":made.id,"label":made.label,
            "created_ms":made.created_ms,"expires_ms":made.expires_ms,"mcp_url":mcp_url}),
        ),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
async fn list(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
) -> Result<Json<Value>, Failure> {
    let admin = personal_tokens::portal_actor(&api, &headers).await?;
    let users = api.knowledge.admin_users(&admin, &product).await?;
    let mcp_url = remote_mcp::resource(&api)
        .ok()
        .filter(|_| wk_contracts::valid_id(&product))
        .map(|origin| format!("{origin}/mcp/{product}"));
    Ok(Json(
        json!({"services":users.services,"users":users.users,"mcp_url":mcp_url}),
    ))
}
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
    body: Result<Json<Create>, JsonRejection>,
) -> Result<Response, Failure> {
    let admin = personal_tokens::portal_actor(&api, &headers).await?;
    let request = parse(body)?;
    days_allowed(&api, request.days)?;
    let (secret, hash) = personal_tokens::generate();
    let made = api
        .knowledge
        .admin_create_user(
            &admin,
            &product,
            &request.user,
            &request.services,
            request.can_review,
            request.label.as_deref().unwrap_or("issued by admin"),
            &hash,
            request.days,
        )
        .await?;
    Ok(issued(
        &api,
        StatusCode::CREATED,
        &product,
        &request.user,
        secret,
        made,
    ))
}
async fn reissue(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((product, user)): Path<(String, String)>,
    body: Result<Json<Reissue>, JsonRejection>,
) -> Result<Response, Failure> {
    let admin = personal_tokens::portal_actor(&api, &headers).await?;
    let request = parse(body)?;
    days_allowed(&api, request.days)?;
    let (secret, hash) = personal_tokens::generate();
    let made = api
        .knowledge
        .admin_reissue_token(
            &admin,
            &product,
            &user,
            request.label.as_deref().unwrap_or("reissued by admin"),
            &hash,
            request.days,
        )
        .await?;
    Ok(issued(&api, StatusCode::OK, &product, &user, secret, made))
}
async fn remove(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((product, user)): Path<(String, String)>,
) -> Result<Json<Value>, Failure> {
    let admin = personal_tokens::portal_actor(&api, &headers).await?;
    api.knowledge
        .admin_remove_user(&admin, &product, &user)
        .await?;
    Ok(Json(json!({"removed":true})))
}
