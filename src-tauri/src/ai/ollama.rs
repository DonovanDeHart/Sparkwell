//! Minimal Ollama HTTP client. Localhost only, no proxy, short timeouts.
//! Every failure is contained and reported as an [`OllamaError`].

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::models::InstalledModel;

/// Sparkwell only ever talks to a local Ollama instance.
pub const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
/// Keep the (small) embedding model resident so searches stay fast between
/// invocations; a cold load is what makes the first search slow.
const EMBED_KEEP_ALIVE: &str = "30m";
/// Smart Add is occasional but often comes in runs (adding several Sparks):
/// keep the chat model for a few minutes, then release its memory.
const CHAT_KEEP_ALIVE: &str = "5m";
/// Context for drafting: the system prompt, up to ~8,000 characters of the
/// Spark and a short JSON answer fit comfortably. Fixed, so a preloaded model
/// is reused as is (a different context would make Ollama reload it) and its
/// memory use is predictable (gemma4:12b: ~8 GB).
pub const DRAFT_CONTEXT: u32 = 8192;
/// A title, summary and tags take ~100 tokens; this bounds a runaway answer.
const DRAFT_MAX_TOKENS: u32 = 512;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum OllamaError {
    #[error("local intelligence is offline")]
    Unreachable,
    #[error("local intelligence timed out")]
    Timeout,
    #[error("the model is not installed")]
    ModelMissing,
    #[error("unexpected response from local intelligence: {0}")]
    BadResponse(String),
}

impl OllamaError {
    fn from_reqwest(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            OllamaError::Timeout
        } else if e.is_connect() || e.is_request() {
            OllamaError::Unreachable
        } else if e.is_decode() {
            OllamaError::BadResponse(e.to_string())
        } else {
            OllamaError::Unreachable
        }
    }
}

#[derive(Clone)]
pub struct OllamaClient {
    http: reqwest::Client,
    base: String,
}

#[derive(Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
    #[serde(default)]
    remote_host: Option<String>,
    #[serde(default)]
    remote_model: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    details: Option<TagDetails>,
}

#[derive(Deserialize)]
struct TagDetails {
    #[serde(default)]
    parameter_size: Option<String>,
}

#[derive(Deserialize)]
struct ShowResponse {
    #[serde(default)]
    capabilities: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct PsResponse {
    #[serde(default)]
    models: Vec<PsModel>,
}

#[derive(Deserialize)]
struct PsModel {
    name: String,
    #[serde(default)]
    context_length: Option<u32>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: Option<ChatMessage>,
}

#[derive(Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: String,
}

impl Default for OllamaClient {
    fn default() -> Self {
        Self::new(OLLAMA_BASE_URL)
    }
}

impl OllamaClient {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_millis(800))
            .build()
            .expect("HTTP client configuration is static and valid");
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    async fn check(resp: reqwest::Response) -> Result<reqwest::Response, OllamaError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let text = resp.text().await.unwrap_or_default();
        if status.as_u16() == 404 || text.contains("not found") {
            Err(OllamaError::ModelMissing)
        } else {
            Err(OllamaError::BadResponse(format!(
                "HTTP {status}: {}",
                text.chars().take(200).collect::<String>()
            )))
        }
    }

    /// Lists installed models. Doubles as the health check.
    pub async fn list_models(&self) -> Result<Vec<InstalledModel>, OllamaError> {
        let resp = self
            .http
            .get(format!("{}/api/tags", self.base))
            .timeout(Duration::from_millis(1500))
            .send()
            .await
            .map_err(OllamaError::from_reqwest)?;
        let tags: TagsResponse = Self::check(resp)
            .await?
            .json()
            .await
            .map_err(OllamaError::from_reqwest)?;
        Ok(tags
            .models
            .into_iter()
            .map(|m| InstalledModel {
                remote: m.remote_host.is_some() || m.remote_model.is_some(),
                size: m.size.filter(|s| *s > 0),
                parameters_b: m
                    .details
                    .and_then(|d| d.parameter_size)
                    .and_then(|p| super::models::parse_parameters_b(&p)),
                name: m.name,
            })
            .collect())
    }

    /// Embeds a batch of inputs via `/api/embed`.
    pub async fn embed(
        &self,
        model: &str,
        inputs: &[String],
        timeout: Duration,
    ) -> Result<Vec<Vec<f32>>, OllamaError> {
        let resp = self
            .http
            .post(format!("{}/api/embed", self.base))
            .timeout(timeout)
            .json(&json!({
                "model": model,
                "input": inputs,
                "truncate": true,
                "keep_alive": EMBED_KEEP_ALIVE,
            }))
            .send()
            .await
            .map_err(OllamaError::from_reqwest)?;
        let body: EmbedResponse = Self::check(resp)
            .await?
            .json()
            .await
            .map_err(OllamaError::from_reqwest)?;
        if body.embeddings.len() != inputs.len() || body.embeddings.iter().any(|v| v.is_empty()) {
            return Err(OllamaError::BadResponse("embedding count mismatch".into()));
        }
        Ok(body.embeddings)
    }

    /// What a model can do (`completion`, `embedding`, `thinking`, …), from
    /// `/api/show`. `None` when this Ollama doesn't report capabilities.
    pub async fn capabilities(&self, model: &str) -> Result<Option<Vec<String>>, OllamaError> {
        let resp = self
            .http
            .post(format!("{}/api/show", self.base))
            .timeout(Duration::from_secs(5))
            .json(&json!({ "model": model }))
            .send()
            .await
            .map_err(OllamaError::from_reqwest)?;
        let show: ShowResponse = Self::check(resp)
            .await?
            .json()
            .await
            .map_err(OllamaError::from_reqwest)?;
        Ok(show.capabilities)
    }

    /// Whether `model` is loaded with the drafting context, i.e. a draft
    /// request would not have to load it first.
    pub async fn chat_model_loaded(&self, model: &str) -> Result<bool, OllamaError> {
        let resp = self
            .http
            .get(format!("{}/api/ps", self.base))
            .timeout(Duration::from_millis(1500))
            .send()
            .await
            .map_err(OllamaError::from_reqwest)?;
        let ps: PsResponse = Self::check(resp)
            .await?
            .json()
            .await
            .map_err(OllamaError::from_reqwest)?;
        Ok(ps
            .models
            .iter()
            .any(|m| m.name == model && m.context_length.map_or(true, |c| c == DRAFT_CONTEXT)))
    }

    /// Loads the drafting model (an empty `/api/generate` only loads it), so
    /// loading and drafting can be reported separately.
    pub async fn load_chat_model(&self, model: &str, timeout: Duration) -> Result<(), OllamaError> {
        let resp = self
            .http
            .post(format!("{}/api/generate", self.base))
            .timeout(timeout)
            .json(&json!({
                "model": model,
                "keep_alive": CHAT_KEEP_ALIVE,
                "options": { "num_ctx": DRAFT_CONTEXT },
            }))
            .send()
            .await
            .map_err(OllamaError::from_reqwest)?;
        Self::check(resp).await.map(|_| ())
    }

    /// Non-streaming `/api/chat` constrained to a JSON schema. Returns the raw
    /// message content for the caller to validate. `thinking` models are asked
    /// not to think: left to itself gemma4:12b spent a minute and its whole
    /// context reasoning about a one-line title, and answered nothing.
    pub async fn chat_json(
        &self,
        model: &str,
        messages: Value,
        schema: Value,
        thinking: bool,
        timeout: Duration,
    ) -> Result<String, OllamaError> {
        let mut request = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "format": schema,
            "keep_alive": CHAT_KEEP_ALIVE,
            "options": {
                "temperature": 0.2,
                "num_ctx": DRAFT_CONTEXT,
                "num_predict": DRAFT_MAX_TOKENS,
            },
        });
        if thinking {
            request["think"] = json!(false);
        }
        let resp = self
            .http
            .post(format!("{}/api/chat", self.base))
            .timeout(timeout)
            .json(&request)
            .send()
            .await
            .map_err(OllamaError::from_reqwest)?;
        let body: ChatResponse = Self::check(resp)
            .await?
            .json()
            .await
            .map_err(OllamaError::from_reqwest)?;
        body.message
            .map(|m| m.content)
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| OllamaError::BadResponse("empty chat response".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_server_fails_fast_and_soft() {
        // Port 9 (discard) on localhost is essentially never an HTTP server.
        let client = OllamaClient::new("http://127.0.0.1:9");
        let started = std::time::Instant::now();
        let err = client.list_models().await.unwrap_err();
        assert!(matches!(
            err,
            OllamaError::Unreachable | OllamaError::Timeout
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
