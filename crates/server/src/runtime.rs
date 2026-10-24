//! Per-process request shedding and bounded complete JSON responses.
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use wk_knowledge::AutoPublish;
#[derive(Clone, Debug)]
pub struct Limits {
    pub pool: wk_knowledge::PoolConfig,
    pub inflight: usize,
    pub http_timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            pool: Default::default(),
            inflight: 64,
            http_timeout: Duration::from_secs(30),
        }
    }
}
impl Limits {
    pub fn parse(mut get: impl FnMut(&str) -> Option<String>) -> Result<Self, String> {
        fn number(
            get: &mut impl FnMut(&str) -> Option<String>,
            name: &str,
            default: u64,
            min: u64,
            max: u64,
        ) -> Result<u64, String> {
            let n = get(name)
                .map(|v| v.parse::<u64>())
                .transpose()
                .map_err(|_| format!("invalid {name}"))?
                .unwrap_or(default);
            if !(min..=max).contains(&n) {
                return Err(format!("invalid {name}"));
            }
            Ok(n)
        }
        let pool = wk_knowledge::PoolConfig {
            max_connections: number(&mut get, "WK_POOL_MAX", 10, 1, 32)? as u32,
            acquire_timeout: Duration::from_millis(number(
                &mut get,
                "WK_DB_ACQUIRE_MS",
                5000,
                100,
                10000,
            )?),
            statement_timeout: Duration::from_millis(number(
                &mut get,
                "WK_DB_STATEMENT_MS",
                5000,
                100,
                15000,
            )?),
            lock_timeout: Duration::from_millis(number(
                &mut get,
                "WK_DB_LOCK_MS",
                3000,
                100,
                5000,
            )?),
        };
        pool.validate()
            .map_err(|_| "invalid database timeout relationship".to_string())?;
        let inflight = number(&mut get, "WK_MAX_INFLIGHT", 64, 1, 64)? as usize;
        let http_timeout =
            Duration::from_secs(number(&mut get, "WK_HTTP_TIMEOUT_SECONDS", 30, 1, 30)?);
        if pool.acquire_timeout + pool.statement_timeout > http_timeout {
            return Err("database timeouts exceed HTTP deadline".into());
        }
        Ok(Self {
            pool,
            inflight,
            http_timeout,
        })
    }
    pub fn from_env() -> Result<Self, String> {
        Self::parse(|key| std::env::var(key).ok())
    }
}
/// `WK_AUTO_PUBLISH_PRODUCTS`: unset or blank enables publish on submit for every product
/// (the collector must still ask per card); `none` turns it off; otherwise a comma-separated
/// list of product ids enables only those. Any malformed entry (including `none` mixed with
/// ids) fails startup rather than silently widening or dropping the list.
pub fn auto_publish_products(
    mut get: impl FnMut(&str) -> Option<String>,
) -> Result<AutoPublish, String> {
    const NAME: &str = "WK_AUTO_PUBLISH_PRODUCTS";
    let value = get(NAME).unwrap_or_default();
    let value = value.trim();
    if value.is_empty() {
        return Ok(AutoPublish::All);
    }
    if value.eq_ignore_ascii_case("none") {
        return Ok(AutoPublish::Off);
    }
    let mut products: Vec<String> = Vec::new();
    for entry in value.split(',') {
        let entry = entry.trim();
        if !wk_contracts::valid_id(entry) || entry.eq_ignore_ascii_case("none") {
            return Err(format!(
                "invalid {NAME}: expected `none` or comma-separated product ids"
            ));
        }
        if !products.iter().any(|p| p == entry) {
            products.push(entry.into());
        }
    }
    if products.len() > 100 {
        return Err(format!("invalid {NAME}: at most 100 products"));
    }
    Ok(AutoPublish::Only(products))
}
#[derive(Clone)]
struct Gate {
    slots: Arc<Semaphore>,
    timeout: Duration,
}
fn unavailable() -> Response {
    crate::Failure(axum::http::StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response()
}
async fn enforce(State(gate): State<Gate>, request: Request, next: Next) -> Response {
    let Ok(_permit) = gate.slots.try_acquire() else {
        return unavailable();
    };
    // Stateless MCP uses JSON; retain the slot and deadline until its body completes.
    match tokio::time::timeout(gate.timeout, async move {
        let response = next.run(request).await;
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, 8 * 1024 * 1024).await.map_err(|_| ())?;
        Ok::<_, ()>(Response::from_parts(parts, Body::from(bytes)))
    })
    .await
    {
        Ok(Ok(response)) => response,
        _ => unavailable(),
    }
}
pub fn bound(router: Router, limits: Limits) -> Router {
    router.layer(middleware::from_fn_with_state(
        Gate {
            slots: Arc::new(Semaphore::new(limits.inflight)),
            timeout: limits.http_timeout,
        },
        enforce,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse(value: Option<&str>) -> Result<AutoPublish, String> {
        auto_publish_products(|_| value.map(String::from))
    }
    fn only(ids: &[&str]) -> Result<AutoPublish, String> {
        Ok(AutoPublish::Only(
            ids.iter().map(|s| s.to_string()).collect(),
        ))
    }
    #[test]
    fn auto_publish_defaults_to_every_product_when_unset_or_blank() {
        assert_eq!(parse(None), Ok(AutoPublish::All));
        assert_eq!(parse(Some("")), Ok(AutoPublish::All));
        assert_eq!(parse(Some("  ")), Ok(AutoPublish::All));
    }
    #[test]
    fn auto_publish_none_turns_it_off_case_insensitively() {
        for off in ["none", "NONE", " None ", "\tnone\n"] {
            assert_eq!(parse(Some(off)), Ok(AutoPublish::Off), "{off:?}");
        }
    }
    #[test]
    fn auto_publish_accepts_product_ids_trimmed_and_deduplicated() {
        assert_eq!(parse(Some("demo")), only(&["demo"]));
        assert_eq!(
            parse(Some(" demo , team_2,demo ")),
            only(&["demo", "team_2"])
        );
    }
    #[test]
    fn auto_publish_fails_closed_on_any_malformed_entry() {
        for bad in [
            "none,a",
            "a,none",
            "a,NONE",
            "none,",
            "none none",
            "demo,",
            ",demo",
            "a,,b",
            "*",
            "all products",
            "demo;x",
            "a/b",
            "é",
        ] {
            assert!(
                parse(Some(bad))
                    .unwrap_err()
                    .contains("WK_AUTO_PUBLISH_PRODUCTS"),
                "{bad}"
            );
        }
        assert!(parse(Some(&"p".repeat(129))).is_err());
        let many = (0..101)
            .map(|i| format!("p{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse(Some(&many)).is_err());
    }
}
