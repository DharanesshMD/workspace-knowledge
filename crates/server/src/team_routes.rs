use crate::{actor, Api, Failure};
use axum::{
    extract::{rejection::QueryRejection, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use wk_knowledge::team::TeamQuery;
pub(crate) fn routes() -> Router<Api> {
    Router::new().route("/v1/products/{product}/context", get(context))
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
