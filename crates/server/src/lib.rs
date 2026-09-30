//! Development HTTP surface. Production OIDC and deployment hardening are pending.
use axum::{
    extract::{rejection::JsonRejection, DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use wk_knowledge::{Actor, Error, Knowledge};
#[derive(Clone)]
struct Api {
    knowledge: Knowledge,
    tokens: Arc<HashMap<[u8; 32], Actor>>,
}
struct Failure(StatusCode, &'static str);
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        match e {
            Error::Denied => Self(StatusCode::FORBIDDEN, "forbidden"),
            Error::NotFound => Self(StatusCode::NOT_FOUND, "not_found"),
            Error::Invalid => Self(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
            Error::Conflict => Self(StatusCode::CONFLICT, "conflict"),
            Error::Storage => Self(StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        }
    }
}
fn actor(api: &Api, headers: &HeaderMap) -> Result<Actor, Failure> {
    if headers.contains_key("origin") {
        return Err(Failure(StatusCode::FORBIDDEN, "browser_access_disabled"));
    }
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(Failure(StatusCode::UNAUTHORIZED, "unauthenticated"))?;
    let key: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    api.tokens
        .get(&key)
        .cloned()
        .ok_or(Failure(StatusCode::UNAUTHORIZED, "unauthenticated"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    product: String,
    query: String,
    limit: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    product: String,
    card: wk_contracts::CardSubmission,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Product {
    product: String,
}
fn parse<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Failure> {
    body.map(|Json(v)| v)
        .map_err(|_| Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"))
}
async fn search(
    State(api): State<Api>,
    headers: HeaderMap,
    body: Result<Json<Search>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers)?;
    let request = parse(body)?;
    let cards = api
        .knowledge
        .search(&who, &request.product, &request.query, request.limit)
        .await?;
    Ok(Json(json!({"cards":cards})))
}
async fn submit(
    State(api): State<Api>,
    headers: HeaderMap,
    body: Result<Json<Submission>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers)?;
    let request = parse(body)?;
    let card = api
        .knowledge
        .submit(&who, &request.product, request.card)
        .await?;
    Ok(Json(json!({"card":card})))
}
async fn publish(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<Product>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers)?;
    let request = parse(body)?;
    let id = uuid::Uuid::parse_str(&id).map_err(|_| Failure(StatusCode::NOT_FOUND, "not_found"))?;
    api.knowledge.publish(&who, &request.product, id).await?;
    Ok(Json(json!({"published":true})))
}
async fn card(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((product, id)): Path<(String, String)>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers)?;
    let id = uuid::Uuid::parse_str(&id).map_err(|_| Failure(StatusCode::NOT_FOUND, "not_found"))?;
    Ok(Json(
        json!({"card":api.knowledge.get(&who,&product,id).await?}),
    ))
}
pub fn router(knowledge: Knowledge, tokens: Vec<(String, Actor)>) -> Router {
    let tokens = tokens
        .into_iter()
        .map(|(token, actor)| (Sha256::digest(token.as_bytes()).into(), actor))
        .collect();
    Router::new()
        .route(
            "/health",
            get(|| async { Json(json!({"status":"ok","mode":"development"})) }),
        )
        .route("/v1/search", post(search))
        .route("/v1/cards", post(submit))
        .route("/v1/cards/{id}/publish", post(publish))
        .route("/v1/products/{product}/cards/{id}", get(card))
        .layer(DefaultBodyLimit::max(65536))
        .with_state(Api {
            knowledge,
            tokens: Arc::new(tokens),
        })
}
