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
use serde_json::{json, Value};
use wk_knowledge::{team::TeamQuery, team_work::TeamWorkQuery};
pub(crate) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/products/{product}/context", get(context))
        .route("/v1/products/{product}/team-work", post(team_work))
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
