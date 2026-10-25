//! Authenticated shared knowledge HTTP API.
pub mod admin_cli;
mod admin_routes;
pub mod auth;
pub mod browser_auth;
mod browser_routes;
mod change_routes;
pub mod decisions;
pub mod device_routes;
pub mod embeddings;
pub mod environments;
mod health;
mod personal_routes;
pub mod personal_tokens;
mod prompt_context;
pub mod remote_mcp;
pub mod retention_worker;
pub mod runtime;
pub mod semantic;
mod team_routes;
use axum::{
    extract::{rejection::JsonRejection, DefaultBodyLimit, Path, Query, State},
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
    embeddings: Option<Arc<embeddings::Embeddings>>,
    health_slots: Arc<tokio::sync::Semaphore>,
    decisions: Arc<std::sync::OnceLock<Result<decisions::Jev, String>>>,
    knowledge: Knowledge,
    tokens: Arc<HashMap<[u8; 32], Actor>>,
    oidc: Option<auth::Oidc>,
    origin: Option<String>,
    browser: Option<browser_auth::BrowserAuth>,
    collector: Option<device_routes::CollectorAuth>,
    personal_tokens: Option<personal_tokens::Config>,
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
fn reject_browser_origin(api: &Api, headers: &HeaderMap) -> Result<(), Failure> {
    if headers
        .get("origin")
        .is_some_and(|v| v.to_str().ok() != api.origin.as_deref())
    {
        return Err(Failure(StatusCode::FORBIDDEN, "browser_access_disabled"));
    }
    Ok(())
}
async fn bearer_actor(api: &Api, headers: &HeaderMap) -> Result<Actor, Failure> {
    reject_browser_origin(api, headers)?;
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(Failure(StatusCode::UNAUTHORIZED, "unauthenticated"))?;
    if let Some(oidc) = &api.oidc {
        let (issuer, subject) = oidc
            .verify(token)
            .map_err(|_| Failure(StatusCode::UNAUTHORIZED, "unauthenticated"))?;
        return api
            .knowledge
            .identity(issuer, &subject)
            .await
            .map_err(|_| Failure(StatusCode::UNAUTHORIZED, "unauthenticated"));
    }
    let key: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    api.tokens
        .get(&key)
        .cloned()
        .ok_or(Failure(StatusCode::UNAUTHORIZED, "unauthenticated"))
}
async fn actor(api: &Api, headers: &HeaderMap) -> Result<Actor, Failure> {
    let cookie = browser_routes::session_cookie(api, headers)?;
    if let Some(secret) = cookie {
        if headers.contains_key("authorization") {
            return Err(Failure(StatusCode::UNAUTHORIZED, "ambiguous_identity"));
        }
        if api.browser.is_none() {
            return Err(Failure(StatusCode::UNAUTHORIZED, "unauthenticated"));
        }
        let session = api
            .knowledge
            .browser_session(&browser_auth::hash(&secret))
            .await
            .map_err(|_| Failure(StatusCode::UNAUTHORIZED, "unauthenticated"))?;
        if session.csrf_hash != browser_auth::hash(&browser_routes::csrf(&secret)) {
            return Err(Failure(StatusCode::UNAUTHORIZED, "unauthenticated"));
        }
        return api
            .knowledge
            .identity(&session.issuer, &session.subject)
            .await
            .map_err(|_| Failure(StatusCode::UNAUTHORIZED, "unauthenticated"));
    }
    bearer_actor(api, headers).await
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
    /// Honored only for an `assistant_written` card in a product the operator enabled;
    /// otherwise the card is stored unpublished and the response says so.
    #[serde(default)]
    publish_on_submit: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Correction {
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
    let who = actor(&api, &headers).await?;
    let request = parse(body)?;
    if wk_summarize::sensitive(&request.query) {
        return Err(Failure(StatusCode::UNPROCESSABLE_ENTITY, "sensitive_query"));
    }
    Ok(Json(
        semantic::search_for(&api, &who, &request.product, &request.query, request.limit).await?,
    ))
}
async fn submit(
    State(api): State<Api>,
    headers: HeaderMap,
    body: Result<Json<Submission>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
    let request = parse(body)?;
    if wk_summarize::sensitive(
        &serde_json::to_string(&request.card)
            .map_err(|_| Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"))?,
    ) {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            "sensitive_summary",
        ));
    }
    let card = api
        .knowledge
        .submit_publishing(
            &who,
            &request.product,
            request.card,
            request.publish_on_submit,
        )
        .await?;
    Ok(Json(json!({"card":card})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Share {
    product: String,
    #[serde(default)]
    service: Option<String>,
    title: String,
    intent: String,
    rationale: String,
    #[serde(default)]
    decisions: Vec<String>,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    open_questions: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
}
/// What a successful share tells the caller, over HTTP and from the hosted MCP tool alike.
fn share_outcome(o: &wk_knowledge::share::ShareOutcome) -> Value {
    json!({
        "card_id": o.card_id,
        "revision": o.revision,
        "published": o.published,
        "status": o.status,
        "withdraw": format!("Open the card in the portal (My context, Withdraw knowledge) or POST /v1/cards/{}/withdraw", o.card_id),
    })
}
fn share_failure(e: wk_knowledge::share::ShareError) -> Response {
    use wk_knowledge::share::ShareError;
    match e {
        ShareError::ServiceRequired(services) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"service_required","services":services})),
        )
            .into_response(),
        ShareError::Sensitive => {
            Failure(StatusCode::UNPROCESSABLE_ENTITY, "sensitive_summary").into_response()
        }
        ShareError::Failed(e) => Failure::from(e).into_response(),
    }
}
/// Share a session summary with only the developer's own sign-in: the server builds the card
/// (assistant-written, partial, no evidence) and applies the publish-on-submit rules.
async fn share(
    State(api): State<Api>,
    headers: HeaderMap,
    body: Result<Json<Share>, JsonRejection>,
) -> Result<Response, Failure> {
    let who = actor(&api, &headers).await?;
    let r = parse(body)?;
    let request = wk_knowledge::share::ShareRequest {
        service: r.service,
        title: r.title,
        intent: r.intent,
        rationale: r.rationale,
        decisions: r.decisions,
        constraints: r.constraints,
        open_questions: r.open_questions,
        files: r.files,
    };
    Ok(
        match api.knowledge.share_session(&who, &r.product, request).await {
            Ok(outcome) => Json(share_outcome(&outcome)).into_response(),
            Err(e) => share_failure(e),
        },
    )
}
async fn publish(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<Product>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &headers).await?;
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
    let who = actor(&api, &headers).await?;
    let id = uuid::Uuid::parse_str(&id).map_err(|_| Failure(StatusCode::NOT_FOUND, "not_found"))?;
    Ok(Json(
        json!({"card":api.knowledge.get(&who,&product,id).await?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    product: String,
    device_id: String,
    approval_hash: String,
    services: Vec<String>,
    policy_revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    product: String,
    revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Feedback {
    product: String,
    revision: i64,
    issue: String,
    suggestion: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphRequest {
    product: String,
    revision: i64,
    provenance: wk_knowledge::graph::Provenance,
}
fn uuid(id: &str) -> Result<uuid::Uuid, Failure> {
    uuid::Uuid::parse_str(id).map_err(|_| Failure(StatusCode::NOT_FOUND, "not_found"))
}
async fn policy(
    State(api): State<Api>,
    headers: HeaderMap,
    Query(r): Query<Product>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    Ok(Json(
        json!({"policy":api.knowledge.policy(&a,&r.product).await?}),
    ))
}
async fn binding(
    State(api): State<Api>,
    headers: HeaderMap,
    body: Result<Json<Binding>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    let r = parse(body)?;
    Ok(Json(
        json!({"binding_id":api.knowledge.bind_workspace(&a,&r.product,&r.device_id,&r.approval_hash,&r.services,r.policy_revision).await?}),
    ))
}
async fn correct(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<Correction>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    let r = parse(body)?;
    Ok(Json(
        json!({"card":api.knowledge.correct(&a,&r.product,uuid(&id)?,r.card).await?}),
    ))
}
async fn withdraw(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<Revision>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    let r = parse(body)?;
    api.knowledge
        .withdraw(&a, &r.product, uuid(&id)?, r.revision)
        .await?;
    Ok(Json(json!({"withdrawn":true})))
}
async fn feedback(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<Feedback>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    let r = parse(body)?;
    Ok(Json(
        json!({"feedback_id":api.knowledge.feedback(&a,&r.product,uuid(&id)?,r.revision,&r.issue,&r.suggestion).await?}),
    ))
}
async fn revisions(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((product, id)): Path<(String, String)>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    Ok(Json(
        json!({"revisions":api.knowledge.revisions(&a,&product,uuid(&id)?).await?}),
    ))
}
async fn review(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    Ok(Json(
        json!({"cards":api.knowledge.review_queue(&a,&product).await?}),
    ))
}
async fn graph(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    Ok(Json(api.knowledge.graph(&a, &product).await?))
}
async fn evidence(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((product, id)): Path<(String, String)>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    Ok(Json(
        json!({"evidence":api.knowledge.evidence(&a,&product,uuid(&id)?).await?}),
    ))
}
async fn activity(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
    body: Result<Json<wk_knowledge::activity::ActivityUpdate>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    let update = parse(body)?;
    if wk_summarize::sensitive(&update.intent) {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            "sensitive_intent",
        ));
    }
    api.knowledge.record_activity(&a, &product, update).await?;
    Ok(Json(json!({"recorded":true})))
}
async fn clear_activity(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    api.knowledge.clear_activity(&a, &product).await?;
    Ok(Json(json!({"cleared":true})))
}
pub(crate) async fn overlaps_for(
    api: &Api,
    a: &Actor,
    product: &str,
    q: wk_knowledge::activity::OverlapQuery,
) -> Result<Value, Failure> {
    prompt_context::overlaps_with(api, &api.knowledge, a, product, q).await
}
async fn overlaps(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
    body: Result<Json<wk_knowledge::activity::OverlapQuery>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    overlaps_for(&api, &a, &product, parse(body)?)
        .await
        .map(Json)
}
async fn relate(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
    body: Result<Json<wk_knowledge::graph::CardRelationship>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    api.knowledge
        .relate_cards(&a, &product, parse(body)?)
        .await?;
    Ok(Json(json!({"linked":true})))
}
async fn reconcile(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<GraphRequest>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    let r = parse(body)?;
    api.knowledge
        .reconcile_graph(&a, &r.product, uuid(&id)?, r.revision, r.provenance)
        .await?;
    Ok(Json(json!({"reconciled":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentQuery {
    product: String,
    service: String,
}
async fn schemas_for(api: &Api, a: &Actor, product: &str) -> Result<Vec<Value>, Failure> {
    let policy = api.knowledge.policy(a, product).await?;
    let schemas = environments::load()
        .map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, "schema_unavailable"))?;
    Ok(schemas.into_iter().filter(|s|s.org==a.org&&s.product==product&&s.service_ids.iter().all(|id|policy.services.contains(id))).map(|s|json!({"id":s.id,"name":s.name,"service_ids":s.service_ids,"variables":s.variables,"access":"schema_only"})).collect())
}
async fn environments(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(product): Path<String>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    Ok(Json(
        json!({"profiles":schemas_for(&api,&a,&product).await?}),
    ))
}
async fn environment(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(profile): Path<String>,
    Query(q): Query<EnvironmentQuery>,
) -> Result<Json<Value>, Failure> {
    let a = actor(&api, &headers).await?;
    let schemas = schemas_for(&api, &a, &q.product).await?;
    let schema = schemas
        .into_iter()
        .find(|s| {
            s["id"] == profile
                && s["service_ids"]
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|id| id == &q.service))
        })
        .ok_or(Failure(StatusCode::NOT_FOUND, "not_found"))?;
    Ok(Json(json!({"schema":schema})))
}
pub fn router(knowledge: Knowledge, tokens: Vec<(String, Actor)>) -> Router {
    configured_router(knowledge, tokens, None, None)
}
pub fn configured_router(
    knowledge: Knowledge,
    tokens: Vec<(String, Actor)>,
    oidc: Option<auth::Oidc>,
    origin: Option<String>,
) -> Router {
    portal_router(knowledge, tokens, oidc, origin, None)
}
pub fn portal_router(
    knowledge: Knowledge,
    tokens: Vec<(String, Actor)>,
    oidc: Option<auth::Oidc>,
    origin: Option<String>,
    browser: Option<browser_auth::BrowserAuth>,
) -> Router {
    portal_router_with_collector(knowledge, tokens, oidc, origin, browser, None)
        .expect("no collector configuration")
}
pub fn portal_router_with_collector(
    knowledge: Knowledge,
    tokens: Vec<(String, Actor)>,
    oidc: Option<auth::Oidc>,
    origin: Option<String>,
    browser: Option<browser_auth::BrowserAuth>,
    collector: Option<device_routes::CollectorAuth>,
) -> Result<Router, String> {
    portal_router_with_embeddings(knowledge, tokens, oidc, origin, browser, collector, None)
}
pub fn portal_router_with_embeddings(
    knowledge: Knowledge,
    tokens: Vec<(String, Actor)>,
    oidc: Option<auth::Oidc>,
    origin: Option<String>,
    browser: Option<browser_auth::BrowserAuth>,
    collector: Option<device_routes::CollectorAuth>,
    embeddings: Option<embeddings::Embeddings>,
) -> Result<Router, String> {
    if let Some(c) = &collector {
        c.validate(oidc.as_ref(), origin.as_deref(), &tokens)?;
    }
    let mode = if oidc.is_some() {
        "production"
    } else {
        "development"
    };
    let tokens = tokens
        .into_iter()
        .map(|(token, actor)| (Sha256::digest(token.as_bytes()).into(), actor))
        .collect();
    let personal_tokens = personal_tokens::config(|key| std::env::var(key).ok())?;
    let api = Api {
        embeddings: embeddings.map(Arc::new),
        health_slots: Arc::new(tokio::sync::Semaphore::new(1)),
        decisions: Arc::new(std::sync::OnceLock::new()),
        knowledge,
        tokens: Arc::new(tokens),
        oidc,
        origin,
        browser,
        collector,
        personal_tokens,
    };
    let limits = runtime::Limits::from_env()?;
    let router = Router::new()
        .merge(browser_routes::routes())
        .merge(personal_routes::routes())
        .merge(personal_tokens::routes())
        .merge(admin_routes::routes())
        .merge(team_routes::routes())
        .merge(change_routes::routes())
        .merge(device_routes::routes())
        .route(
            "/health",
            get(move || async move { Json(json!({"status":"ok","mode":mode})) }),
        )
        .route("/mcp/{product}", axum::routing::any(remote_mcp::transport))
        .route(
            "/.well-known/oauth-protected-resource",
            get(remote_mcp::metadata),
        )
        .route("/v1/search", post(search))
        .route("/v1/products/{product}/decisions", post(decisions::assess))
        .route("/v1/products/{product}/environments", get(environments))
        .route("/v1/environments/{profile}/schema", get(environment))
        .route("/v1/policy", get(policy))
        .route("/v1/workspace-bindings", post(binding))
        .route("/v1/cards", post(submit))
        .route("/v1/share", post(share))
        .route("/v1/cards/{id}/publish", post(publish))
        .route("/v1/cards/{id}/correct", post(correct))
        .route("/v1/cards/{id}/withdraw", post(withdraw))
        .route("/v1/cards/{id}/feedback", post(feedback))
        .route("/v1/cards/{id}/graph", post(reconcile))
        .route("/v1/products/{product}/cards/{id}", get(card))
        .route("/v1/products/{product}/cards/{id}/evidence", get(evidence))
        .route(
            "/v1/products/{product}/cards/{id}/revisions",
            get(revisions),
        )
        .route("/v1/products/{product}/review", get(review))
        .route("/v1/products/{product}/graph", get(graph))
        .route("/v1/products/{product}/relationships", post(relate))
        .route(
            "/v1/products/{product}/activity",
            post(activity)
                .get(team_routes::activity)
                .delete(clear_activity),
        )
        .route("/v1/products/{product}/overlaps", post(overlaps))
        .layer(DefaultBodyLimit::max(65536))
        .layer(axum::middleware::from_fn_with_state(
            api.clone(),
            browser_routes::protect,
        ))
        .with_state(api.clone());
    Ok(runtime::bound(router, limits).merge(health::routes().with_state(api)))
}
