//! Minimal Ollama HTTP client. Localhost only, no proxy, short timeouts.
//! Every failure is contained and reported as an [`OllamaError`].

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::models::InstalledModel;

/// Sparkwell only ever talks to a local Ollama instance.
pub const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
/// Keep models resident briefly so retrieval stays fast between invocations.
const KEEP_ALIVE: &str = "15m";

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
                "keep_alive": KEEP_ALIVE,
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

    /// Non-streaming `/api/chat` constrained to a JSON schema. Returns the raw
    /// message content for the caller to validate.
    pub async fn chat_json(
        &self,
        model: &str,
        messages: Value,
        schema: Value,
        timeout: Duration,
    ) -> Result<String, OllamaError> {
        let resp = self
            .http
            .post(format!("{}/api/chat", self.base))
            .timeout(timeout)
            .json(&json!({
                "model": model,
                "messages": messages,
                "stream": false,
                "format": schema,
                "keep_alive": "5m",
                "options": { "temperature": 0.2 },
            }))
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
