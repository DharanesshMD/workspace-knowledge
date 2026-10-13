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
