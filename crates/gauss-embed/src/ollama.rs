//! Ollama embedding provider (`/api/embeddings`).

use crate::Embedder;
use async_trait::async_trait;
use gauss_engine::error::{AgentError, Result};
use serde_json::{json, Value};

const DEFAULT_BASE_URL: &str = "http://localhost:11434";

pub struct OllamaEmbedder {
    client: reqwest::Client,
    base_url: String,
    model: String,
    dim: usize,
}

impl OllamaEmbedder {
    /// `dim` is the model's output dimension (e.g. 768 for `nomic-embed-text`).
    pub fn new(model: impl Into<String>, dim: usize) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: model.into(),
            dim,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

pub(crate) fn parse_embedding(body: &Value) -> Result<Vec<f32>> {
    body.get("embedding")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect()
        })
        .ok_or_else(|| AgentError::other("ollama embeddings: missing 'embedding'"))
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    fn dimension(&self) -> usize {
        self.dim
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            let resp = self
                .client
                .post(format!("{}/api/embeddings", self.base_url))
                .json(&json!({ "model": self.model, "prompt": text }))
                .send()
                .await
                .map_err(|e| AgentError::other(format!("ollama embed request: {e}")))?;
            let body: Value = resp
                .json()
                .await
                .map_err(|e| AgentError::other(format!("ollama embed json: {e}")))?;
            out.push(parse_embedding(&body)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_embedding_vector() {
        let body = json!({ "embedding": [0.1, 0.2, 0.3] });
        assert_eq!(parse_embedding(&body).unwrap(), vec![0.1f32, 0.2, 0.3]);
    }
}
