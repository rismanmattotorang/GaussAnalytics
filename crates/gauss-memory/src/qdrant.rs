//! Qdrant-backed [`AgentMemory`] over the Qdrant REST API.
//!
//! Two collections hold tool-usage and text memories; vectors come from the
//! injected [`Embedder`] and Qdrant performs cosine search. Compile-checked
//! here; a live round-trip test runs only when `QDRANT_URL` is set.

use async_trait::async_trait;
use gauss_embed::Embedder;
use gauss_engine::context::ToolContext;
use gauss_engine::error::{AgentError, Result};
use gauss_engine::model::memory::{
    TextMemory, TextMemorySearchResult, ToolMemory, ToolMemorySearchResult,
};
use gauss_engine::traits::AgentMemory;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use uuid::Uuid;

pub struct QdrantAgentMemory {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    embedder: Arc<dyn Embedder>,
    tool_collection: String,
    text_collection: String,
}

impl QdrantAgentMemory {
    /// Build a memory backed by Qdrant at `base_url` (e.g. `http://localhost:6333`).
    /// `prefix` namespaces the two collections (`{prefix}_tool`, `{prefix}_text`).
    pub fn new(base_url: impl Into<String>, prefix: &str, embedder: Arc<dyn Embedder>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: None,
            embedder,
            tool_collection: format!("{prefix}_tool"),
            text_collection: format!("{prefix}_text"),
        }
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Create both collections if they do not already exist.
    pub async fn ensure_collections(&self) -> Result<()> {
        let dim = self.embedder.dimension();
        for name in [&self.tool_collection, &self.text_collection] {
            let body = json!({ "vectors": { "size": dim, "distance": "Cosine" } });
            // PUT is idempotent-ish; ignore "already exists" style errors.
            let _ = self
                .request(
                    reqwest::Method::PUT,
                    &format!("/collections/{name}"),
                    Some(body),
                )
                .await;
        }
        Ok(())
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base_url));
        if let Some(key) = &self.api_key {
            req = req.header("api-key", key);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AgentError::other(format!("qdrant request: {e}")))?;
        let status = resp.status();
        let value: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(AgentError::other(format!("qdrant {status}: {value}")));
        }
        Ok(value)
    }

    async fn upsert(
        &self,
        collection: &str,
        id: &str,
        vector: Vec<f32>,
        payload: Value,
    ) -> Result<()> {
        let body = json!({ "points": [{ "id": id, "vector": vector, "payload": payload }] });
        self.request(
            reqwest::Method::PUT,
            &format!("/collections/{collection}/points?wait=true"),
            Some(body),
        )
        .await
        .map(|_| ())
    }
}

fn tool_memory_from_payload(id: &str, payload: &Value) -> ToolMemory {
    let args = payload
        .get("args_json")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    ToolMemory {
        memory_id: Some(id.to_string()),
        question: payload
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tool_name: payload
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        args,
        timestamp: payload
            .get("timestamp")
            .and_then(Value::as_str)
            .map(str::to_string),
        success: payload
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        metadata: None,
    }
}

#[async_trait]
impl AgentMemory for QdrantAgentMemory {
    async fn save_tool_usage(
        &self,
        question: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        _context: &ToolContext,
        success: bool,
        _metadata: Option<&Map<String, Value>>,
    ) -> Result<()> {
        let vector = self.embedder.embed_one(question).await?;
        let id = Uuid::new_v4().to_string();
        let payload = json!({
            "memory_id": id,
            "question": question,
            "tool_name": tool_name,
            "args_json": Value::Object(args.clone()).to_string(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "success": success,
        });
        self.upsert(&self.tool_collection, &id, vector, payload)
            .await
    }

    async fn save_text_memory(&self, content: &str, _context: &ToolContext) -> Result<TextMemory> {
        let vector = self.embedder.embed_one(content).await?;
        let id = Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().to_rfc3339();
        let payload = json!({ "memory_id": id, "content": content, "timestamp": timestamp });
        self.upsert(&self.text_collection, &id, vector, payload)
            .await?;
        Ok(TextMemory {
            memory_id: Some(id),
            content: content.to_string(),
            timestamp: Some(timestamp),
        })
    }

    async fn search_similar_usage(
        &self,
        question: &str,
        _context: &ToolContext,
        limit: usize,
        similarity_threshold: f32,
        tool_name_filter: Option<&str>,
    ) -> Result<Vec<ToolMemorySearchResult>> {
        let vector = self.embedder.embed_one(question).await?;
        let mut must = vec![json!({ "key": "success", "match": { "value": true } })];
        if let Some(name) = tool_name_filter {
            must.push(json!({ "key": "tool_name", "match": { "value": name } }));
        }
        let body = json!({
            "vector": vector,
            "limit": limit,
            "with_payload": true,
            "score_threshold": similarity_threshold,
            "filter": { "must": must },
        });
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/collections/{}/points/search", self.tool_collection),
                Some(body),
            )
            .await?;
        let results = resp
            .get("result")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(results
            .iter()
            .enumerate()
            .map(|(rank, hit)| {
                let id = hit.get("id").and_then(Value::as_str).unwrap_or("");
                let payload = hit.get("payload").cloned().unwrap_or(Value::Null);
                ToolMemorySearchResult {
                    memory: tool_memory_from_payload(id, &payload),
                    similarity_score: hit.get("score").and_then(Value::as_f64).unwrap_or(0.0)
                        as f32,
                    rank: rank as u32,
                }
            })
            .collect())
    }

    async fn search_text_memories(
        &self,
        query: &str,
        _context: &ToolContext,
        limit: usize,
        similarity_threshold: f32,
    ) -> Result<Vec<TextMemorySearchResult>> {
        let vector = self.embedder.embed_one(query).await?;
        let body = json!({
            "vector": vector,
            "limit": limit,
            "with_payload": true,
            "score_threshold": similarity_threshold,
        });
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/collections/{}/points/search", self.text_collection),
                Some(body),
            )
            .await?;
        let results = resp
            .get("result")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(results
            .iter()
            .enumerate()
            .map(|(rank, hit)| {
                let id = hit
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let payload = hit.get("payload").cloned().unwrap_or(Value::Null);
                TextMemorySearchResult {
                    memory: TextMemory {
                        memory_id: Some(id),
                        content: payload
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        timestamp: payload
                            .get("timestamp")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    },
                    similarity_score: hit.get("score").and_then(Value::as_f64).unwrap_or(0.0)
                        as f32,
                    rank: rank as u32,
                }
            })
            .collect())
    }

    async fn get_recent_memories(
        &self,
        _context: &ToolContext,
        limit: usize,
    ) -> Result<Vec<ToolMemory>> {
        let body = json!({ "limit": limit, "with_payload": true });
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/collections/{}/points/scroll", self.tool_collection),
                Some(body),
            )
            .await?;
        let points = resp
            .pointer("/result/points")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(points
            .iter()
            .map(|p| {
                let id = p.get("id").and_then(Value::as_str).unwrap_or("");
                tool_memory_from_payload(id, &p.get("payload").cloned().unwrap_or(Value::Null))
            })
            .collect())
    }

    async fn get_recent_text_memories(
        &self,
        _context: &ToolContext,
        limit: usize,
    ) -> Result<Vec<TextMemory>> {
        let body = json!({ "limit": limit, "with_payload": true });
        let resp = self
            .request(
                reqwest::Method::POST,
                &format!("/collections/{}/points/scroll", self.text_collection),
                Some(body),
            )
            .await?;
        let points = resp
            .pointer("/result/points")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(points
            .iter()
            .map(|p| {
                let payload = p.get("payload").cloned().unwrap_or(Value::Null);
                TextMemory {
                    memory_id: p.get("id").and_then(Value::as_str).map(str::to_string),
                    content: payload
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    timestamp: payload
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                }
            })
            .collect())
    }

    async fn delete_by_id(&self, _context: &ToolContext, memory_id: &str) -> Result<bool> {
        let body = json!({ "points": [memory_id] });
        self.request(
            reqwest::Method::POST,
            &format!(
                "/collections/{}/points/delete?wait=true",
                self.tool_collection
            ),
            Some(body),
        )
        .await
        .map(|_| true)
    }

    async fn delete_text_memory(&self, _context: &ToolContext, memory_id: &str) -> Result<bool> {
        let body = json!({ "points": [memory_id] });
        self.request(
            reqwest::Method::POST,
            &format!(
                "/collections/{}/points/delete?wait=true",
                self.text_collection
            ),
            Some(body),
        )
        .await
        .map(|_| true)
    }

    async fn clear_memories(
        &self,
        _context: &ToolContext,
        tool_name: Option<&str>,
        _before_date: Option<&str>,
    ) -> Result<usize> {
        let filter = match tool_name {
            Some(name) => json!({ "must": [{ "key": "tool_name", "match": { "value": name } }] }),
            // Match everything by filtering on the always-present `success` field.
            None => json!({ "should": [
                { "key": "success", "match": { "value": true } },
                { "key": "success", "match": { "value": false } }
            ]}),
        };
        self.request(
            reqwest::Method::POST,
            &format!(
                "/collections/{}/points/delete?wait=true",
                self.tool_collection
            ),
            Some(json!({ "filter": filter })),
        )
        .await
        .map(|_| 0)
    }
}
