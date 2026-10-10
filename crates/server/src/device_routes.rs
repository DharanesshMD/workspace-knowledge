//! Narrow, signed collector API and separately authenticated owner device controls.
use crate::{actor, auth, bearer_actor, parse, Api, Failure};
use axum::{
    body::Bytes,
    extract::{rejection::JsonRejection, OriginalUri, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use wk_contracts::device::*;
use wk_knowledge::{devices::DevicePermission, Actor};
#[derive(Clone)]
pub struct CollectorAuth {
    pub origin: String,
    pub oidc: Option<auth::Oidc>,
    pub tokens: Vec<(String, Actor)>,
    pub client_id: Option<String>,
}
impl CollectorAuth {
    pub(crate) fn validate(
        &self,
        ordinary: Option<&auth::Oidc>,
        origin: Option<&str>,
        tokens: &[(String, Actor)],
    ) -> Result<(), String> {
        let u = reqwest::Url::parse(&self.origin).map_err(|_| "invalid collector origin")?;
        let loopback =
            u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
        if u.origin().ascii_serialization() != self.origin
            || (!loopback && u.scheme() != "https")
            || !u.username().is_empty()
            || u.password().is_some()
            || origin.is_some_and(|v| v != self.origin)
        {
            return Err("collector requires exact hosted HTTPS or test loopback origin".into());
        }
        if let Some(o) = ordinary {
            let c = self
                .oidc
                .as_ref()
                .ok_or("production collector OIDC required")?;
            if c.audience() == o.audience()
                || c.issuer() != o.issuer()
                || !self.tokens.is_empty()
                || self.client_id.as_ref().is_none_or(|id| id.is_empty())
            {
                return Err(
                    "collector requires distinct audience and upstream public client".into(),
                );
            }
        } else if !loopback {
            return Err("development collector is loopback only".into());
        }
        if self
            .tokens
            .iter()
            .any(|(t, _)| tokens.iter().any(|(o, _)| o == t))
        {
            return Err("collector token must differ from ordinary credentials".into());
        }
        Ok(())
    }
}
pub(crate) fn routes() -> Router<Api> {
    Router::new()
        .route("/collector/config", get(config))
        .route("/collector/{operation}", post(collect))
        .route("/v1/products/{product}/devices", get(list))
        .route(
            "/v1/products/{product}/devices/{id}/settings",
            axum::routing::patch(change),
        )
        .route("/v1/products/{product}/devices/{id}/revoke", post(revoke))
}
async fn config(State(api): State<Api>) -> Json<Value> {
    Json(match api.collector {
        Some(c) => {
            json!({"enabled":true,"issuer":c.oidc.as_ref().map(|o|o.issuer()),"audience":c.oidc.as_ref().map(|o|o.audience()),"client_id":c.client_id})
        }
        None => json!({"enabled":false}),
    })
}
fn unauthorized() -> Failure {
    Failure(StatusCode::UNAUTHORIZED, "unauthenticated")
}
fn invalid() -> Failure {
    Failure(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request")
}
fn header<'a>(h: &'a HeaderMap, name: &str) -> Result<&'a str, Failure> {
    if h.get_all(name).iter().count() != 1 {
        return Err(unauthorized());
    }
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(unauthorized)
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Failure> {
    serde_json::from_slice(bytes).map_err(|_| invalid())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    product: String,
    approval_hash: String,
    request: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
async fn collect(
    State(api): State<Api>,
    Path(operation): Path<String>,
    OriginalUri(uri): OriginalUri,
    h: HeaderMap,
    bytes: Bytes,
) -> Result<Json<Value>, Failure> {
    let cfg = api.collector.as_ref().ok_or_else(unauthorized)?;
    if h.contains_key("cookie") || h.contains_key("origin") || uri.query().is_some() {
        return Err(unauthorized());
    }
    let token = header(&h, "authorization")?
        .strip_prefix("Bearer ")
        .ok_or_else(unauthorized)?;
    let mut authenticated = api.clone();
    authenticated.oidc = cfg.oidc.clone();
    authenticated.tokens = Arc::new(
        cfg.tokens
            .iter()
            .map(|(t, a)| (Sha256::digest(t.as_bytes()).into(), a.clone()))
            .collect(),
    );
    let who = bearer_actor(&authenticated, &h).await?;
    let id: uuid::Uuid = header(&h, "x-wk-device")?
        .parse()
        .map_err(|_| unauthorized())?;
    let nonce: uuid::Uuid = header(&h, "x-wk-nonce")?
        .parse()
        .map_err(|_| unauthorized())?;
    let time: i64 = header(&h, "x-wk-time")?
        .parse()
        .map_err(|_| unauthorized())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| unauthorized())?
        .as_secs() as i64;
    if time.abs_diff(now) > 60 || id.is_nil() || nonce.is_nil() {
        return Err(unauthorized());
    }
    let signature = ed25519_dalek::Signature::from_slice(
        &hex::decode(header(&h, "x-wk-signature")?).map_err(|_| unauthorized())?,
    )
    .map_err(|_| unauthorized())?;
    let (registration, envelope) = if operation == "register" {
        (Some(decode::<DeviceRegistration>(&bytes)?), None)
    } else {
        (None, Some(decode::<Envelope>(&bytes)?))
    };
    let public = if let Some(r) = &registration {
        if r.id != id {
            return Err(unauthorized());
        }
        r.public_key.clone()
    } else {
        api.knowledge
            .device_get(&who, &envelope.as_ref().ok_or_else(invalid)?.product, id)
            .await
            .map_err(|_| unauthorized())?
            .public_key
    };
    let key_bytes: [u8; 32] = hex::decode(&public)
        .map_err(|_| unauthorized())?
        .try_into()
        .map_err(|_| unauthorized())?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes).map_err(|_| unauthorized())?;
    if key.is_weak()
        || key
            .verify_strict(
                &signing_bytes(
                    &cfg.origin,
                    "POST",
                    uri.path(),
                    time,
                    nonce,
                    id,
                    token,
                    &bytes,
                ),
                &signature,
            )
            .is_err()
    {
        return Err(unauthorized());
    }
    if let Some(r) = registration {
        let d = api.knowledge.device_register(&who, r).await?;
        api.knowledge
            .device_nonce(&who, &d, nonce)
            .await
            .map_err(|_| unauthorized())?;
        return Ok(Json(json!({"device":d})));
    }
    let e = envelope.ok_or_else(invalid)?;
    let d = api
        .knowledge
        .device_get(&who, &e.product, id)
        .await
        .map_err(|_| unauthorized())?;
    if d.approval_hash != e.approval_hash {
        return Err(unauthorized());
    }
    api.knowledge
        .device_nonce(&who, &d, nonce)
        .await
        .map_err(|_| unauthorized())?;
    let permission = match operation.as_str() {
        "cards" => DevicePermission::SharedUpload,
        "personal" => DevicePermission::OwnerUpload,
        "activity" => DevicePermission::ActivityUpload,
        "policy" | "report" | "overlaps" => DevicePermission::Read,
        _ => return Err(Failure(StatusCode::NOT_FOUND, "not_found")),
    };
    let k = api.knowledge.device_bound(&d, permission);
    let value = match operation.as_str() {
        "policy" => {
            let _: Empty = serde_json::from_value(e.request).map_err(|_| invalid())?;
            json!({"policy":k.policy(&who,&e.product).await?,"device":d})
        }
        "cards" => {
            let r: crate::Submission = serde_json::from_value(e.request).map_err(|_| invalid())?;
            if r.product != e.product {
                return Err(invalid());
            }
            json!({"card":k.submit(&who,&e.product,r.card).await?})
        }
        "personal" => {
            json!({"summary":k.personal_put(&who,&e.product,serde_json::from_value(e.request).map_err(|_|invalid())?).await?})
        }
        "activity" => {
            k.record_activity(
                &who,
                &e.product,
                serde_json::from_value(e.request).map_err(|_| invalid())?,
            )
            .await?;
            json!({"accepted":true})
        }
        "overlaps" => {
            let n = k
                .overlaps(
                    &who,
                    &e.product,
                    serde_json::from_value(e.request).map_err(|_| invalid())?,
                )
                .await?;
            json!({"text":wk_knowledge::activity::render(&n),"notices":n})
        }
        "report" => {
            api.knowledge
                .device_report(
                    &who,
                    &d,
                    serde_json::from_value(e.request).map_err(|_| invalid())?,
                )
                .await?;
            json!({"accepted":true})
        }
        _ => unreachable!(),
    };
    Ok(Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    expected_revision: i64,
    settings: DeviceSettings,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    expected_revision: i64,
}
async fn list(
    State(api): State<Api>,
    h: HeaderMap,
    Path(p): Path<String>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &h).await?;
    Ok(Json(
        json!({"devices":api.knowledge.device_list(&who,&p).await?}),
    ))
}
async fn change(
    State(api): State<Api>,
    h: HeaderMap,
    Path((p, id)): Path<(String, uuid::Uuid)>,
    body: Result<Json<Change>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &h).await?;
    let r = parse(body)?;
    Ok(Json(
        json!({"device":api.knowledge.device_change(&who,&p,id,r.expected_revision,r.settings).await?}),
    ))
}
async fn revoke(
    State(api): State<Api>,
    h: HeaderMap,
    Path((p, id)): Path<(String, uuid::Uuid)>,
    body: Result<Json<Revision>, JsonRejection>,
) -> Result<Json<Value>, Failure> {
    let who = actor(&api, &h).await?;
    let r = parse(body)?;
    Ok(Json(
        json!({"device":api.knowledge.device_revoke(&who,&p,id,r.expected_revision).await?}),
    ))
}
