use crate::{actor, Api, Failure};
use axum::{
    extract::{
        rejection::{JsonRejection, QueryRejection},
        Path, Query, State,
    },
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use wk_knowledge::{team::TeamQuery, team_work::TeamWorkQuery};
pub(crate) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/products/{product}/context", get(context))
        .route("/v1/products/{product}/team-work", post(team_work))
        .route("/v1/products/{product}/agent-setup", get(agent_setup))
}
async fn context(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
    query: Result<Query<TeamQuery>, QueryRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    let Query(query) =
        query.map_err(|_| Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"))?;
    let entries = api.knowledge.team_context(&who, &product, query).await?;
    let next = if entries.len() == 50 {
        entries.last().map(|e| e.card.id)
    } else {
        None
    };
    Ok(Json(json!({"entries":entries,"next_cursor":next})))
}
pub(crate) async fn activity(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    Ok(Json(
        json!({"activity":api.knowledge.team_activity(&who,&product).await?}),
    ))
}
/// What teammates did: by developer, file or module, time and keywords, packed to a size cap.
async fn team_work(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
    body: Result<Json<TeamWorkQuery>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    let query = crate::parse(body)?;
    Ok(Json(api.knowledge.team_work(&who, &product, query).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupQuery {
    /// Standing instructions that share finished work automatically (default: only when asked).
    #[serde(default)]
    auto_share: bool,
}
/// How each supported coding client connects to this product and the standing instructions it should
/// be given, with this product's MCP address filled in. Contains no token.
async fn agent_setup(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
    query: Result<Query<SetupQuery>, QueryRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    let Query(query) =
        query.map_err(|_| Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"))?;
    // A non-member is denied, as for every other product read.
    api.knowledge
        .policy(&who, &product)
        .await
        .map_err(|e| match e {
            wk_knowledge::Error::NotFound => wk_knowledge::Error::Denied,
            other => other,
        })?;
    let mcp_url = crate::remote_mcp::resource(&api)
        .ok()
        .filter(|_| wk_contracts::valid_id(&product))
        .map(|origin| format!("{origin}/mcp/{product}"));
    let clients = mcp_url
        .as_deref()
        .map(|url| wk_contracts::agent_setup::clients(url, query.auto_share))
        .unwrap_or_default();
    Ok(Json(json!({
        "product": product,
        "mcp_url": mcp_url,
        "token_env": wk_contracts::agent_setup::TOKEN_ENV,
        "token_placeholder": wk_contracts::agent_setup::TOKEN_PLACEHOLDER,
        "rules": wk_contracts::agent_setup::rules(query.auto_share),
        "clients": clients,
    })))
}
