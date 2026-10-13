//! Explicit loopback-only Ollama embeddings; no application credentials or cache.
use serde_json::json;
use std::time::Duration;
use tokio::sync::Semaphore;
use wk_knowledge::semantic::Namespace;
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
    pub namespace: Namespace,
    endpoint: reqwest::Url,
    client: reqwest::Client,
    slots: Semaphore,
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
        })
    }
    pub fn from_env() -> Result<Option<Self>, String> {
        Config::parse(|name| std::env::var(name).ok())?
            .map(Self::new)
            .transpose()
    }
    pub async fn embed_query(&self, text: &str) -> Result<Vec<f32>, &'static str> {
        self.bounded_embed(text, true, Duration::from_secs(3)).await
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
        if text.trim().is_empty()
            || text.len() > if query { 1024 } else { 16384 }
            || wk_summarize::sensitive(text)
        {
            return Err("input_rejected");
        }
        let _permit = self.slots.try_acquire().map_err(|_| "embedding_busy")?;
        tokio::time::timeout(deadline, self.embed(text, query))
            .await
            .map_err(|_| "embedding_timeout")?
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
        let response=self.client.post(self.endpoint.clone()).json(&json!({"model":self.namespace.model,"input":format!("{prefix}: {text}"),"truncate":false,"keep_alive":"5m"}))
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
