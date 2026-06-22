//! HTTP request/response models and the SSE wire chunk.
//! Mirrors `gauss/servers/base/models.py`.

use gauss_engine::components::UiComponent;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Incoming chat request body.
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

/// One streamed chunk. `rich`/`simple` are the frontend-serialized components.
/// The shape is the contract consumed by the `<gauss-chat>` web component.
#[derive(Debug, Serialize)]
pub struct ChatStreamChunk {
    pub rich: Value,
    pub simple: Option<Value>,
    pub conversation_id: String,
    pub request_id: String,
    pub timestamp: f64,
}

fn now_secs() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

impl ChatStreamChunk {
    pub fn from_component(
        component: &UiComponent,
        conversation_id: &str,
        request_id: &str,
    ) -> Self {
        Self {
            rich: component.rich_component.serialize_for_frontend(),
            simple: component
                .simple_component
                .as_ref()
                .map(gauss_engine::SimpleComponent::serialize_for_frontend),
            conversation_id: conversation_id.to_string(),
            request_id: request_id.to_string(),
            timestamp: now_secs(),
        }
    }
}

/// Non-streaming (poll) response: all chunks collected.
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub chunks: Vec<ChatStreamChunk>,
    pub conversation_id: String,
    pub request_id: String,
    pub total_chunks: usize,
}
