//! Anthropic (Claude) `LlmService`.
//!
//! The Messages API differs from OpenAI in three ways the conversion handles:
//! - the system prompt is a top-level field, not a message;
//! - assistant tool calls are `tool_use` content blocks;
//! - tool results are `tool_result` blocks that must live inside a *user* turn,
//!   so consecutive tool messages are grouped into one user turn.

use async_trait::async_trait;
use gauss_engine::error::{AgentError, Result};
use gauss_engine::model::llm::{LlmRequest, LlmResponse};
use gauss_engine::model::tool::ToolCall;
use gauss_engine::traits::LlmService;
use serde_json::{json, Map, Value};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Anthropic Messages LLM service.
pub struct AnthropicLlmService {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl AnthropicLlmService {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

pub(crate) fn build_payload(model: &str, request: &LlmRequest) -> Value {
    let mut turns: Vec<Value> = Vec::new();
    let mut tool_buf: Vec<Value> = Vec::new();

    for m in &request.messages {
        if m.role == "tool" {
            tool_buf.push(json!({
                "type": "tool_result",
                "tool_use_id": m.tool_call_id,
                "content": m.content,
            }));
            continue;
        }
        // Flush any pending tool results as a single user turn.
        if !tool_buf.is_empty() {
            turns.push(json!({ "role": "user", "content": std::mem::take(&mut tool_buf) }));
        }
        match m.role.as_str() {
            "assistant" => {
                let mut blocks: Vec<Value> = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(json!({ "type": "text", "text": m.content }));
                }
                if let Some(tcs) = &m.tool_calls {
                    for tc in tcs {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments,
                        }));
                    }
                }
                turns.push(json!({ "role": "assistant", "content": blocks }));
            }
            _ => turns.push(json!({
                "role": "user",
                "content": [{ "type": "text", "text": m.content }],
            })),
        }
    }
    if !tool_buf.is_empty() {
        turns.push(json!({ "role": "user", "content": tool_buf }));
    }

    let mut payload = json!({
        "model": model,
        "max_tokens": request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "temperature": request.temperature,
        "messages": turns,
    });
    if let Some(sp) = &request.system_prompt {
        payload["system"] = json!(sp);
    }
    if let Some(tools) = &request.tools {
        if !tools.is_empty() {
            payload["tools"] = Value::Array(
                tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "input_schema": t.parameters,
                        })
                    })
                    .collect(),
            );
        }
    }
    payload
}

pub(crate) fn parse_response(body: &Value) -> Result<LlmResponse> {
    let blocks = body
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| AgentError::LlmService("response missing content".into()))?;

    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
            Some("tool_use") => {
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let arguments = block
                    .get("input")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_else(Map::new);
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            _ => {}
        }
    }

    Ok(LlmResponse {
        content: (!text.is_empty()).then_some(text),
        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
        finish_reason: body
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    })
}

#[async_trait]
impl LlmService for AnthropicLlmService {
    async fn send_request(&self, request: LlmRequest) -> Result<LlmResponse> {
        let payload = build_payload(&self.model, &request);
        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&payload)
            .send()
            .await
            .map_err(|e| AgentError::LlmService(format!("request failed: {e}")))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AgentError::LlmService(format!("invalid JSON response: {e}")))?;
        if !status.is_success() {
            return Err(AgentError::LlmService(format!(
                "Anthropic API error {status}: {body}"
            )));
        }
        parse_response(&body)
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_engine::model::llm::LlmMessage;
    use gauss_engine::model::tool::ToolCall;
    use gauss_engine::model::user::User;

    #[test]
    fn system_is_top_level_and_tool_results_grouped() {
        let assistant = LlmMessage {
            role: "assistant".into(),
            content: "let me check".into(),
            tool_calls: Some(vec![ToolCall {
                id: "tu_1".into(),
                name: "run_sql".into(),
                arguments: serde_json::Map::new(),
            }]),
            tool_call_id: None,
        };
        let tool1 = LlmMessage {
            role: "tool".into(),
            content: "rows".into(),
            tool_calls: None,
            tool_call_id: Some("tu_1".into()),
        };
        let request = LlmRequest {
            messages: vec![LlmMessage::new("user", "hi"), assistant, tool1],
            tools: None,
            user: User::new("u"),
            stream: false,
            temperature: 0.2,
            max_tokens: None,
            system_prompt: Some("sys".into()),
            metadata: Default::default(),
        };
        let p = build_payload("claude-sonnet-4-5", &request);
        assert_eq!(p["system"], "sys");
        assert_eq!(p["max_tokens"], DEFAULT_MAX_TOKENS);
        // user, assistant(tool_use), user(tool_result)
        let turns = p["messages"].as_array().unwrap();
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[1]["role"], "assistant");
        assert_eq!(turns[1]["content"][1]["type"], "tool_use");
        assert_eq!(turns[2]["role"], "user");
        assert_eq!(turns[2]["content"][0]["type"], "tool_result");
        assert_eq!(turns[2]["content"][0]["tool_use_id"], "tu_1");
    }

    #[test]
    fn parse_text_and_tool_use_blocks() {
        let body = json!({
            "content": [
                { "type": "text", "text": "sure" },
                { "type": "tool_use", "id": "tu_9", "name": "run_sql", "input": { "sql": "SELECT 1" } }
            ],
            "stop_reason": "tool_use"
        });
        let r = parse_response(&body).unwrap();
        assert_eq!(r.content.as_deref(), Some("sure"));
        let tc = &r.tool_calls.unwrap()[0];
        assert_eq!(tc.name, "run_sql");
        assert_eq!(tc.arguments["sql"], "SELECT 1");
    }
}
