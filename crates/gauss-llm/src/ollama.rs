//! Ollama `LlmService` for local/self-hosted models.
//!
//! Talks to the Ollama `/api/chat` REST endpoint. Unlike OpenAI, Ollama returns
//! tool-call arguments as a JSON object (not a string).

use async_trait::async_trait;
use gauss_engine::error::{AgentError, Result};
use gauss_engine::model::llm::{LlmRequest, LlmResponse};
use gauss_engine::model::tool::ToolCall;
use gauss_engine::traits::LlmService;
use serde_json::{json, Map, Value};
use uuid::Uuid;

const DEFAULT_BASE_URL: &str = "http://localhost:11434";

/// Ollama chat LLM service.
pub struct OllamaLlmService {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaLlmService {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: model.into(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

pub(crate) fn build_payload(model: &str, request: &LlmRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(sp) = &request.system_prompt {
        messages.push(json!({ "role": "system", "content": sp }));
    }
    for m in &request.messages {
        match m.role.as_str() {
            "assistant" if m.tool_calls.is_some() => {
                let calls: Vec<Value> = m
                    .tool_calls
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|tc| json!({ "function": { "name": tc.name, "arguments": tc.arguments } }))
                    .collect();
                messages.push(json!({
                    "role": "assistant",
                    "content": m.content,
                    "tool_calls": calls,
                }));
            }
            "tool" => messages.push(json!({ "role": "tool", "content": m.content })),
            _ => messages.push(json!({ "role": m.role, "content": m.content })),
        }
    }

    let mut payload = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "options": { "temperature": request.temperature },
    });
    if let Some(tools) = &request.tools {
        if !tools.is_empty() {
            payload["tools"] = Value::Array(
                tools
                    .iter()
                    .map(|t| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": t.name,
                                "description": t.description,
                                "parameters": t.parameters,
                            }
                        })
                    })
                    .collect(),
            );
        }
    }
    payload
}

pub(crate) fn parse_response(body: &Value) -> Result<LlmResponse> {
    let message = body
        .get("message")
        .ok_or_else(|| AgentError::LlmService("response missing message".into()))?;

    let content = message
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_tool_call).collect::<Vec<_>>())
        .filter(|v| !v.is_empty());

    Ok(LlmResponse {
        content,
        tool_calls,
        finish_reason: body
            .get("done_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    })
}

fn parse_tool_call(v: &Value) -> Option<ToolCall> {
    let func = v.get("function")?;
    let name = func.get("name")?.as_str()?.to_string();
    // Ollama returns arguments as an object directly.
    let arguments = func
        .get("arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);
    Some(ToolCall {
        id: Uuid::new_v4().to_string(),
        name,
        arguments,
    })
}

#[async_trait]
impl LlmService for OllamaLlmService {
    async fn send_request(&self, request: LlmRequest) -> Result<LlmResponse> {
        let payload = build_payload(&self.model, &request);
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
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
                "Ollama API error {status}: {body}"
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
    use gauss_engine::model::user::User;

    #[test]
    fn payload_prepends_system_message() {
        let request = LlmRequest {
            messages: vec![LlmMessage::new("user", "hi")],
            tools: None,
            user: User::new("u"),
            stream: false,
            temperature: 0.3,
            max_tokens: None,
            system_prompt: Some("sys".into()),
            metadata: Default::default(),
        };
        let p = build_payload("llama3", &request);
        assert_eq!(p["messages"][0]["role"], "system");
        assert_eq!(p["messages"][1]["content"], "hi");
        assert_eq!(p["options"]["temperature"], 0.3);
    }

    #[test]
    fn parse_object_arguments() {
        let body = json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{ "function": { "name": "run_sql", "arguments": { "sql": "SELECT 1" } } }]
            },
            "done": true
        });
        let r = parse_response(&body).unwrap();
        let tc = &r.tool_calls.unwrap()[0];
        assert_eq!(tc.name, "run_sql");
        assert_eq!(tc.arguments["sql"], "SELECT 1");
    }
}
