use crate::{Api, Failure};
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde_json::{json, Value};
pub(crate) fn routes() -> Router<Api> {
    Router::new()
        .route("/health/live", get(|| async { Json(json!({"live":true})) }))
        .route("/health/ready", get(ready))
}
async fn ready(State(api): State<Api>) -> Result<Json<Value>, Failure> {
    let _permit = api
        .health_slots
        .try_acquire()
        .map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable"))?;
    match tokio::time::timeout(std::time::Duration::from_secs(1), api.knowledge.ready()).await {
        Ok(Ok(())) => Ok(Json(json!({"ready":true}))),
        _ => Err(Failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable")),
    }
}
