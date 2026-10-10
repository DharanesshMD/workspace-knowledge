use crate::{actor, parse, Api, Failure};
use axum::{
    extract::{rejection::JsonRejection, Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use wk_knowledge::personal::PersonalInput;
pub(crate) fn routes() -> Router<Api> {
    Router::new()
        .route("/v1/products/{product}/personal", get(list).post(put))
        .route(
            "/v1/products/{product}/personal/{id}",
            get(detail).patch(edit).delete(remove),
        )
        .route("/v1/products/{product}/personal/{id}/submit", post(submit))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    before: Option<uuid::Uuid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    expected_revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    expected_revision: i64,
    body: wk_contracts::CardSubmission,
}
async fn list(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
    query: Result<Query<Page>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    let Query(page) = query.map_err(|_| {
        Failure(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_request",
        )
    })?;
    let notes = api
        .knowledge
        .personal_list(&who, &product, page.before)
        .await?;
    let next = if notes.len() == 50 {
        notes.last().map(|n| n.id)
    } else {
        None
    };
    Ok(Json(json!({"summaries":notes,"next_cursor":next})))
}
async fn put(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
    input: Result<Json<PersonalInput>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    let note = api
        .knowledge
        .personal_put(&who, &product, parse(input)?)
        .await?;
    Ok(Json(json!({"summary":note})))
}
async fn detail(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((product, id)): Path<(String, uuid::Uuid)>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    Ok(Json(
        json!({"summary":api.knowledge.personal_get(&who,&product,id).await?}),
    ))
}
async fn edit(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((product, id)): Path<(String, uuid::Uuid)>,
    input: Result<Json<Edit>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    let input = parse(input)?;
    Ok(Json(
        json!({"summary":api.knowledge.personal_edit(&who,&product,id,input.expected_revision,input.body).await?}),
    ))
}
async fn remove(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((product, id)): Path<(String, uuid::Uuid)>,
    input: Result<Json<Revision>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    api.knowledge
        .personal_delete(&who, &product, id, parse(input)?.expected_revision)
        .await?;
    Ok(Json(json!({"deleted":true})))
}
async fn submit(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((product, id)): Path<(String, uuid::Uuid)>,
    input: Result<Json<Revision>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    Ok(Json(
        json!({"card":api.knowledge.personal_submit(&who,&product,id,parse(input)?.expected_revision).await?}),
    ))
}
