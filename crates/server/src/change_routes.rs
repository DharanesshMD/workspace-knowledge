//! Thin bounded transports. Human confirmation is session/CSRF protected and absent from MCP.
use crate::{actor, parse, Api, Failure};
use axum::{
    extract::{rejection::JsonRejection, DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use wk_contracts::change_context::ChangeContextRequest;
use wk_knowledge::{
    code_provenance::{AnchorInput, ContributionInput, ProfileInput},
    Actor,
};
pub(crate) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/products/{product}/change-context", post(context))
        .route("/v1/products/{product}/code-attribution", get(attribution))
        .route("/v1/products/{product}/code-profile", post(profile))
        .route(
            "/v1/products/{product}/code-contributions",
            post(contribution),
        )
        .route(
            "/v1/products/{product}/code-contributions/{id}/confirm",
            post(confirm_contribution),
        )
        .route(
            "/v1/products/{product}/code-contributions/{id}/review",
            post(review_contribution),
        )
        .route("/v1/products/{product}/code-anchors", post(anchor))
        .route(
            "/v1/products/{product}/code-anchors/{id}/confirm",
            post(confirm_anchor),
        )
        .route(
            "/v1/products/{product}/code-anchors/{id}/review",
            post(review_anchor),
        )
        .route("/v1/products/{product}/preflights/{id}", get(brief))
        .route(
            "/v1/products/{product}/preflights/{id}/acknowledge",
            post(acknowledge),
        )
        .layer(DefaultBodyLimit::max(65536))
        .layer(axum::middleware::from_fn(deadline))
}
pub(crate) async fn deadline(req: Request, next: Next) -> Response {
    match tokio::time::timeout(std::time::Duration::from_secs(5), next.run(req)).await {
        Ok(r) => r,
        Err(_) => Failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response(),
    }
}
async fn browser(api: &Api, h: &HeaderMap) -> Result<Actor, Failure> {
    if h.contains_key("authorization")
        || api.browser.is_none()
        || crate::browser_routes::session_cookie(api, h)?.is_none()
    {
        return Err(Failure(StatusCode::UNAUTHORIZED, "unauthenticated"));
    }
    actor(api, h).await
}
fn bounded<T: serde::Serialize>(v: T) -> Result<Json<Value>, Failure> {
    let value = serde_json::to_value(v)
        .map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable"))?;
    if serde_json::to_vec(&value)
        .map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable"))?
        .len()
        > 16384
    {
        return Err(Failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable"));
    }
    Ok(Json(value))
}
fn parse_body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Failure> {
    if body
        .as_ref()
        .is_err_and(|e| e.status() == StatusCode::PAYLOAD_TOO_LARGE)
    {
        return Err(Failure(StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"));
    }
    parse(body)
}
fn id(value: &str) -> Result<uuid::Uuid, Failure> {
    value
        .parse()
        .ok()
        .filter(|id: &uuid::Uuid| !id.is_nil())
        .ok_or(Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"))
}
async fn context(
    State(api): State<Api>,
    h: HeaderMap,
    Path(p): Path<String>,
    body: Result<Json<ChangeContextRequest>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &h).await?;
    bounded(
        api.knowledge
            .change_context(&who, &p, parse_body(body)?)
            .await?,
    )
}
async fn attribution(
    State(api): State<Api>,
    h: HeaderMap,
    Path(p): Path<String>,
) -> Result<Json<Value>, Failure> {
    let who = browser(&api, &h).await?;
    bounded(api.knowledge.code_attribution(&who, &p).await?)
}
async fn profile(
    State(api): State<Api>,
    h: HeaderMap,
    Path(p): Path<String>,
    body: Result<Json<ProfileInput>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = browser(&api, &h).await?;
    let out = api
        .knowledge
        .put_code_profile(&who, &p, parse_body(body)?)
        .await?;
    bounded(json!({"revision":out.revision}))
}
async fn contribution(
    State(api): State<Api>,
    h: HeaderMap,
    Path(p): Path<String>,
    body: Result<Json<ContributionInput>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &h).await?;
    let out = api
        .knowledge
        .propose_contribution(&who, &p, parse_body(body)?)
        .await?;
    bounded(json!({"id":out.id,"revision":out.revision}))
}
async fn anchor(
    State(api): State<Api>,
    h: HeaderMap,
    Path(p): Path<String>,
    body: Result<Json<AnchorInput>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &h).await?;
    let out = api
        .knowledge
        .propose_rationale_anchor(&who, &p, parse_body(body)?)
        .await?;
    bounded(json!({"id":out.id,"revision":out.revision}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    expected_revision: i64,
    confirmed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    expected_revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Acknowledgement {
    confirmed: bool,
}
async fn confirm_contribution(
    State(api): State<Api>,
    h: HeaderMap,
    Path((p, value)): Path<(String, String)>,
    body: Result<Json<Confirmation>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = browser(&api, &h).await?;
    let r = parse_body(body)?;
    if !r.confirmed {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            "confirmation_required",
        ));
    }
    let out = api
        .knowledge
        .confirm_contribution(&who, &p, id(&value)?, r.expected_revision)
        .await?;
    bounded(json!({"id":out.id,"revision":out.revision}))
}
async fn confirm_anchor(
    State(api): State<Api>,
    h: HeaderMap,
    Path((p, value)): Path<(String, String)>,
    body: Result<Json<Confirmation>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = browser(&api, &h).await?;
    let r = parse_body(body)?;
    if !r.confirmed {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            "confirmation_required",
        ));
    }
    let out = api
        .knowledge
        .confirm_rationale_anchor(&who, &p, id(&value)?, r.expected_revision)
        .await?;
    bounded(json!({"id":out.id,"revision":out.revision}))
}
async fn review_contribution(
    State(api): State<Api>,
    h: HeaderMap,
    Path((p, value)): Path<(String, String)>,
    body: Result<Json<Revision>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = browser(&api, &h).await?;
    let r = parse_body(body)?;
    let out = api
        .knowledge
        .review_contribution(&who, &p, id(&value)?, r.expected_revision)
        .await?;
    bounded(json!({"id":out.id,"revision":out.revision}))
}
async fn review_anchor(
    State(api): State<Api>,
    h: HeaderMap,
    Path((p, value)): Path<(String, String)>,
    body: Result<Json<Revision>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = browser(&api, &h).await?;
    let r = parse_body(body)?;
    let out = api
        .knowledge
        .review_rationale_anchor(&who, &p, id(&value)?, r.expected_revision)
        .await?;
    bounded(json!({"id":out.id,"revision":out.revision}))
}
async fn brief(
    State(api): State<Api>,
    h: HeaderMap,
    Path((p, value)): Path<(String, String)>,
) -> Result<Json<Value>, Failure> {
    let who = browser(&api, &h).await?;
    bounded(api.knowledge.preflight_view(&who, &p, id(&value)?).await?)
}
async fn acknowledge(
    State(api): State<Api>,
    h: HeaderMap,
    Path((p, value)): Path<(String, String)>,
    body: Result<Json<Acknowledgement>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = browser(&api, &h).await?;
    if !parse_body(body)?.confirmed {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            "confirmation_required",
        ));
    }
    api.knowledge
        .acknowledge_preflight(&who, &p, id(&value)?)
        .await?;
    bounded(json!({"acknowledged":true}))
}
