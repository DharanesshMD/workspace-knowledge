//! Explicit loopback-only Ollama embeddings; query vectors are cached in memory only.
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use wk_knowledge::semantic::Namespace;
const QUERY_CACHE: usize = 1024;
const QUERY_TTL: Duration = Duration::from_secs(600);
/// Recent query vectors keyed by organisation, product and whitespace-normalized text, so
/// one tenant's query never times as a hit for another; never persisted or logged.
#[derive(Default)]
struct QueryCache {
    entries: HashMap<String, (Vec<f32>, Instant, u64)>,
    tick: u64,
}
impl QueryCache {
    fn get(&mut self, key: &str, now: Instant) -> Option<Vec<f32>> {
        self.tick += 1;
        let tick = self.tick;
        match self.entries.get_mut(key) {
            Some((vector, at, used)) if now.duration_since(*at) < QUERY_TTL => {
                *used = tick;
                Some(vector.clone())
            }
            Some(_) => {
                self.entries.remove(key);
                None
            }
            None => None,
        }
    }
    fn put(&mut self, key: String, vector: Vec<f32>, now: Instant) {
        self.tick += 1;
        if self.entries.len() >= QUERY_CACHE && !self.entries.contains_key(&key) {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (vector, now, self.tick));
    }
}
/// Ollama `keep_alive`: an integer with `s`, `m` or `h`, at most 24 hours.
pub fn parse_keep_alive(value: &str) -> Result<String, String> {
    let invalid = || "invalid embedding keep_alive".to_string();
    let Some(unit) = value.chars().last() else {
        return Err(invalid());
    };
    let number = &value[..value.len() - unit.len_utf8()];
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let n: u64 = number.parse().map_err(|_| invalid())?;
    let seconds = match unit {
        's' => n,
        'm' => n.saturating_mul(60),
        'h' => n.saturating_mul(3600),
        _ => return Err(invalid()),
    };
    if seconds > 86_400 {
        return Err("embedding keep_alive exceeds 24h".into());
    }
    Ok(value.to_string())
}
/// Default cosine similarity floor for prompt-time meaning matches. Keyword prompts embed less
/// sharply than full questions, so this sits below search's 0.6; calibrated with
/// `tests/prompt_context_eval.rs`.
pub const PROMPT_FLOOR: f32 = 0.55;
/// `WK_PROMPT_SIMILARITY_FLOOR`: a cosine similarity from 0.3 to 0.95.
pub fn parse_prompt_floor(value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .ok()
        .filter(|v| (0.3..=0.95).contains(v))
        .ok_or_else(|| "invalid prompt similarity floor".to_string())
}
#[derive(Clone)]
pub struct Config {
    pub endpoint: String,
    pub namespace: Namespace,
}
impl Config {
    pub fn parse(mut get: impl FnMut(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let values = [
            "WK_EMBEDDING_URL",
            "WK_EMBEDDING_MODEL",
            "WK_EMBEDDING_DIGEST",
            "WK_EMBEDDING_DIMENSIONS",
        ]
        .map(&mut get);
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let [Some(endpoint), Some(model), Some(digest), Some(dimensions)] = values else {
            return Err("incomplete embedding configuration".into());
        };
        let dimensions = dimensions
            .parse()
            .map_err(|_| "invalid embedding dimensions")?;
        let config = Self {
            endpoint,
            namespace: Namespace {
                model,
                digest,
                dimensions,
            },
        };
        config.validate()?;
        Ok(Some(config))
    }
    fn validate(&self) -> Result<reqwest::Url, String> {
        self.namespace
            .validate()
            .map_err(|_| "invalid embedding namespace")?;
        let endpoint =
            reqwest::Url::parse(&self.endpoint).map_err(|_| "invalid embedding endpoint")?;
        if endpoint.scheme() != "http"
            || !matches!(endpoint.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))
            || endpoint.path() != "/api/embed"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !self.namespace.model.starts_with("nomic-embed-text:")
        {
            return Err(
                "embedding endpoint must be literal loopback HTTP and use the Nomic search profile"
                    .into(),
            );
        }
        Ok(endpoint)
    }
}
pub struct Embeddings {
    /// Fixed for the life of the instance; the query cache relies on it.
    pub namespace: Namespace,
    endpoint: reqwest::Url,
    client: reqwest::Client,
    slots: Semaphore,
    keep_alive: String,
    prompt_floor: f32,
    cache: Mutex<QueryCache>,
}
impl Embeddings {
    pub fn new(config: Config) -> Result<Self, String> {
        let endpoint = config.validate()?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "embedding client unavailable")?;
        Ok(Self {
            namespace: config.namespace,
            endpoint,
            client,
            slots: Semaphore::new(2),
            keep_alive: "30m".into(),
            prompt_floor: PROMPT_FLOOR,
            cache: Mutex::default(),
        })
    }
    /// Use an Ollama `keep_alive` already checked by [`parse_keep_alive`].
    pub fn with_keep_alive(mut self, value: String) -> Self {
        self.keep_alive = value;
        self
    }
    /// Use a prompt similarity floor already checked by [`parse_prompt_floor`].
    pub fn with_prompt_floor(mut self, value: f32) -> Self {
        self.prompt_floor = value;
        self
    }
    /// Minimum cosine similarity for prompt-time meaning matches.
    pub fn prompt_floor(&self) -> f32 {
        self.prompt_floor
    }
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_vars(|name| std::env::var(name).ok())
    }
    /// [`Self::from_env`] over any variable source. Optional settings are checked even when no
    /// model is configured, so a bad value is always a startup error.
    pub fn from_vars(mut get: impl FnMut(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let keep_alive = get("WK_EMBEDDING_KEEP_ALIVE")
            .map(|value| parse_keep_alive(&value))
            .transpose()?;
        let prompt_floor = get("WK_PROMPT_SIMILARITY_FLOOR")
            .map(|value| parse_prompt_floor(&value))
            .transpose()?;
        let Some(mut embeddings) = Config::parse(&mut get)?.map(Self::new).transpose()? else {
            return Ok(None);
        };
        if let Some(value) = keep_alive {
            embeddings = embeddings.with_keep_alive(value);
        }
        if let Some(value) = prompt_floor {
            embeddings = embeddings.with_prompt_floor(value);
        }
        Ok(Some(embeddings))
    }
    pub async fn embed_query(
        &self,
        org: &str,
        product: &str,
        text: &str,
    ) -> Result<Vec<f32>, &'static str> {
        self.embed_query_within(org, product, text, Duration::from_secs(3))
            .await
    }
    /// Cached query embedding within `deadline`. The cache is in memory only, keyed by
    /// `org`, `product` and the whitespace-normalized text.
    ///
    /// Cache hits skip the provider digest check for up to the 10-minute TTL; any provider
    /// call that detects a model change clears the cache.
    pub async fn embed_query_within(
        &self,
        org: &str,
        product: &str,
        text: &str,
        deadline: Duration,
    ) -> Result<Vec<f32>, &'static str> {
        screen(text, true)?;
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let key = format!("{org}\0{product}\0{normalized}");
        if let Some(vector) = self.cache().get(&key, Instant::now()) {
            return Ok(vector);
        }
        let vector = self.bounded_embed(text, true, deadline).await?;
        self.cache().put(key, vector.clone(), Instant::now());
        Ok(vector)
    }
    fn cache(&self) -> std::sync::MutexGuard<'_, QueryCache> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Load the model before the first prompt; never cached.
    pub async fn warm(&self) -> Result<(), &'static str> {
        self.bounded_embed("warm up", true, Duration::from_secs(30))
            .await
            .map(|_| ())
    }
    pub async fn embed_document(&self, text: &str) -> Result<Vec<f32>, &'static str> {
        self.bounded_embed(text, false, Duration::from_secs(30))
            .await
    }
    async fn bounded_embed(
        &self,
        text: &str,
        query: bool,
        deadline: Duration,
    ) -> Result<Vec<f32>, &'static str> {
        screen(text, query)?;
        let _permit = self.slots.try_acquire().map_err(|_| "embedding_busy")?;
        let result = tokio::time::timeout(deadline, self.embed(text, query))
            .await
            .map_err(|_| "embedding_timeout")?;
        if result == Err("embedding_model_changed") {
            *self.cache() = QueryCache::default();
        }
        result
    }
    async fn digest_current(&self) -> Result<(), &'static str> {
        let mut url = self.endpoint.clone();
        url.set_path("/api/tags");
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| "embedding_unavailable")?;
        let value = wk_summarize::bounded_json(response, 1024 * 1024)
            .await
            .map_err(|_| "embedding_invalid_response")?;
        let models = value["models"]
            .as_array()
            .ok_or("embedding_invalid_response")?;
        let matches = models
            .iter()
            .filter(|m| m["name"] == self.namespace.model)
            .collect::<Vec<_>>();
        if matches.len() != 1 || matches[0]["digest"] != self.namespace.digest {
            return Err("embedding_model_changed");
        }
        Ok(())
    }
    async fn embed(&self, text: &str, query: bool) -> Result<Vec<f32>, &'static str> {
        self.digest_current().await?;
        let prefix = if query {
            "search_query"
        } else {
            "search_document"
        };
        let response=self.client.post(self.endpoint.clone()).json(&json!({"model":self.namespace.model,"input":format!("{prefix}: {text}"),"truncate":false,"keep_alive":self.keep_alive}))
            .send().await.map_err(|_|"embedding_unavailable")?;
        let value = wk_summarize::bounded_json(response, 1024 * 1024)
            .await
            .map_err(|_| "embedding_invalid_response")?;
        if value["model"] != self.namespace.model {
            return Err("embedding_model_changed");
        }
        let vectors: Vec<Vec<f32>> = serde_json::from_value(value["embeddings"].clone())
            .map_err(|_| "embedding_invalid_response")?;
        if vectors.len() != 1 {
            return Err("embedding_invalid_response");
        }
        let vector = vectors
            .into_iter()
            .next()
            .ok_or("embedding_invalid_response")?;
        self.namespace
            .vector(&vector)
            .map_err(|_| "embedding_invalid_response")?;
        self.digest_current().await?;
        Ok(vector)
    }
}
fn screen(text: &str, query: bool) -> Result<(), &'static str> {
    if text.trim().is_empty()
        || text.len() > if query { 1024 } else { 16384 }
        || wk_summarize::sensitive(text)
    {
        return Err("input_rejected");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_cache_expires_and_evicts_least_recently_used() {
        let mut cache = QueryCache::default();
        let start = Instant::now();
        for i in 0..QUERY_CACHE {
            cache.put(i.to_string(), vec![i as f32], start);
        }
        assert!(cache.get("0", start).is_some());
        cache.put("new".into(), vec![0.], start);
        assert_eq!(cache.entries.len(), QUERY_CACHE);
        assert!(cache.get("0", start).is_some(), "recently used entry kept");
        assert!(
            cache.get("1", start).is_none(),
            "least recently used evicted"
        );
        assert!(cache.get("new", start + QUERY_TTL).is_none(), "expired");
        assert!(!cache.entries.contains_key("new"));
    }
    #[tokio::test]
    async fn a_poisoned_cache_lock_still_serves_queries() {
        let e = Embeddings::new(Config {
            endpoint: "http://127.0.0.1:9/api/embed".into(),
            namespace: Namespace {
                model: "nomic-embed-text:v1.5".into(),
                digest: "a".repeat(64),
                dimensions: 3,
            },
        })
        .unwrap();
        e.cache.lock().unwrap().put(
            "org\0demo\0rate limiter".into(),
            vec![1., 0., 0.],
            Instant::now(),
        );
        let _ = std::thread::scope(|s| {
            s.spawn(|| {
                let _guard = e.cache.lock().unwrap();
                panic!("poison the cache lock");
            })
            .join()
        });
        assert!(e.cache.is_poisoned());
        assert_eq!(
            e.embed_query("org", "demo", "rate limiter").await,
            Ok(vec![1., 0., 0.])
        );
    }
}
