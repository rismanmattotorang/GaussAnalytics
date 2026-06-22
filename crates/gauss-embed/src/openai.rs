//! OpenAI embedding provider (`/embeddings`).

use crate::Embedder;
use async_trait::async_trait;
use gauss_engine::error::{AgentError, Result};
use serde_json::{json, Value};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

pub struct OpenAiEmbedder {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    dim: usize,
}

impl OpenAiEmbedder {
    /// `dim` is the model's output dimension (1536 for `text-embedding-3-small`).
    pub fn new(api_key: impl Into<String>, model: impl Into<String>, dim: usize) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: api_key.into(),
            model: model.into(),
            dim,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

pub(crate) fn build_payload(model: &str, texts: &[String]) -> Value {
    json!({ "model": model, "input": texts })
}

pub(crate) fn parse_embeddings(body: &Value) -> Result<Vec<Vec<f32>>> {
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| AgentError::other("openai embeddings: missing 'data'"))?;
    Ok(data
        .iter()
        .map(|item| {
            item.get("embedding")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect())
}

#[async_trait]
impl Embedder for OpenAiEmbedder {
    fn dimension(&self) -> usize {
        self.dim
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let resp = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&build_payload(&self.model, texts))
            .send()
            .await
            .map_err(|e| AgentError::other(format!("openai embed request: {e}")))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AgentError::other(format!("openai embed json: {e}")))?;
        if !status.is_success() {
            return Err(AgentError::other(format!(
                "openai embeddings error {status}: {body}"
            )));
        }
        parse_embeddings(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_and_parse() {
        let p = build_payload("text-embedding-3-small", &["a".into(), "b".into()]);
        assert_eq!(p["input"][1], "b");
        let body = json!({ "data": [
            { "embedding": [0.1, 0.2] },
            { "embedding": [0.3, 0.4] }
        ]});
        let v = parse_embeddings(&body).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1], vec![0.3f32, 0.4]);
    }
}
