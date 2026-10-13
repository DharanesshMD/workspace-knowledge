use crate::{
    actor,
    browser_auth::{hash, random_secret},
    Api, Failure,
};
use axum::{
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
fn dev(api: &Api) -> bool {
    api.browser
        .as_ref()
        .map(|b| b.development())
        .unwrap_or(api.oidc.is_none())
}
fn cookie_name(api: &Api, transaction: bool) -> &'static str {
    match (dev(api), transaction) {
        (true, true) => "wk_dev_login",
        (true, false) => "wk_dev_session",
        (false, true) => "__Host-wk_login",
        (false, false) => "__Host-wk_session",
    }
}
fn cookie(headers: &HeaderMap, name: &str) -> Result<Option<String>, Failure> {
    let mut found = None;
    for header in headers.get_all("cookie") {
        let text = header
            .to_str()
            .map_err(|_| Failure(StatusCode::UNAUTHORIZED, "invalid_cookie"))?;
        if text.len() > 8192 {
            return Err(Failure(StatusCode::UNAUTHORIZED, "invalid_cookie"));
        }
        for item in text.split(';') {
            if let Some((key, value)) = item.trim().split_once('=') {
                if key == name {
                    if found.is_some() {
                        return Err(Failure(StatusCode::UNAUTHORIZED, "invalid_cookie"));
                    }
                    found = Some(value.to_string());
                }
            }
        }
    }
    Ok(found)
}
pub(crate) fn session_cookie(api: &Api, headers: &HeaderMap) -> Result<Option<String>, Failure> {
    cookie(headers, cookie_name(api, false))
}
fn set_cookie(api: &Api, transaction: bool, value: &str, max_age: u32) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        cookie_name(api, transaction),
        value,
        max_age,
        if dev(api) { "" } else { "; Secure" }
    ))
    .expect("generated hexadecimal cookie")
}
pub(crate) fn csrf(secret: &str) -> String {
    hash(&format!("wk-browser-csrf-v1:{secret}"))
}
fn failure() -> Failure {
    Failure(StatusCode::UNAUTHORIZED, "unauthenticated")
}
pub(crate) fn routes() -> Router<Api> {
    Router::new()
        .route("/auth/config", get(config))
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/logout", post(logout))
        .route("/v1/me", get(me))
}
pub(crate) async fn protect(State(api): State<Api>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let guarded = path.starts_with("/v1/") || path == "/auth/logout";
    let validation = (|| -> Result<(), Failure> {
        if guarded {
            match session_cookie(&api, request.headers()) {
                Err(e) => return Err(e),
                Ok(Some(secret)) => {
                    if request.headers().contains_key("authorization") {
                        return Err(failure());
                    }
                    if !matches!(
                        *request.method(),
                        Method::GET | Method::HEAD | Method::OPTIONS
                    ) {
                        let origin = request
                            .headers()
                            .get("origin")
                            .and_then(|h| h.to_str().ok());
                        if origin.is_none() || origin != api.origin.as_deref() {
                            return Err(Failure(StatusCode::FORBIDDEN, "invalid_origin"));
                        }
                        let supplied = request
                            .headers()
                            .get("x-csrf-token")
                            .and_then(|h| h.to_str().ok())
                            .unwrap_or("");
                        if secret.len() != 64 || hash(supplied) != hash(&csrf(&secret)) {
                            return Err(Failure(StatusCode::FORBIDDEN, "invalid_csrf"));
                        }
                    }
                    if secret.len() != 64 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(failure());
                    }
                }
                Ok(None) => {}
            }
        }
        Ok(())
    })();
    let mut response = match validation {
        Ok(()) => next.run(request).await,
        Err(error) => error.into_response(),
    };
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
}
async fn config(State(api): State<Api>) -> Json<serde_json::Value> {
    Json(json!({"browser_login":api.browser.is_some(),"development":api.oidc.is_none()}))
}
async fn login(State(api): State<Api>) -> Result<Response, Failure> {
    let browser = api.browser.as_ref().ok_or(Failure(
        StatusCode::SERVICE_UNAVAILABLE,
        "browser_login_not_configured",
    ))?;
    let start = browser
        .begin()
        .await
        .map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, "login_unavailable"))?;
    let mut response = Redirect::to(&start.url).into_response();
    response.headers_mut().append(
        "set-cookie",
        set_cookie(&api, true, &start.transaction_cookie, 300),
    );
    Ok(response)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    iss: Option<String>,
    session_state: Option<String>,
}
async fn callback(
    State(api): State<Api>,
    headers: HeaderMap,
    query: Result<Query<Callback>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Failure> {
    let browser = api.browser.as_ref().ok_or(failure())?;
    let Query(q) = query.map_err(|_| failure())?;
    // Never echo provider error descriptions or query data.
    let _ = (q.error_description, q.iss, q.session_state);
    if q.error.is_some() {
        return Err(failure());
    }
    let binding = cookie(&headers, cookie_name(&api, true))?.ok_or(failure())?;
    let (issuer, subject) = browser
        .complete(
            &binding,
            q.state.as_deref().ok_or(failure())?,
            q.code.as_deref().ok_or(failure())?,
        )
        .await
        .map_err(|_| failure())?;
    api.knowledge
        .identity(&issuer, &subject)
        .await
        .map_err(|_| failure())?;
    let secret = random_secret();
    api.knowledge
        .create_browser_session(&hash(&secret), &issuer, &subject, &hash(&csrf(&secret)))
        .await
        .map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, "session_unavailable"))?;
    if let Some(old) = session_cookie(&api, &headers)? {
        api.knowledge
            .delete_browser_session(&hash(&old))
            .await
            .map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, "session_unavailable"))?;
    }
    let mut response = Redirect::to("/").into_response();
    response
        .headers_mut()
        .append("set-cookie", set_cookie(&api, false, &secret, 43200));
    response
        .headers_mut()
        .append("set-cookie", set_cookie(&api, true, "", 0));
    Ok(response)
}
async fn me(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Failure> {
    let who = actor(&api, &headers).await?;
    let products = api.knowledge.products_for(&who).await?;
    let token = session_cookie(&api, &headers)?.map(|s| csrf(&s));
    Ok(Json(
        json!({"user":who.user,"org":who.org,"products":products,"csrf_token":token}),
    ))
}
async fn logout(State(api): State<Api>, headers: HeaderMap) -> Result<Response, Failure> {
    actor(&api, &headers).await?;
    let secret = session_cookie(&api, &headers)?.ok_or(failure())?;
    api.knowledge.delete_browser_session(&hash(&secret)).await?;
    let mut response = Json(json!({"logged_out":true})).into_response();
    response
        .headers_mut()
        .append("set-cookie", set_cookie(&api, false, "", 0));
    Ok(response)
}
