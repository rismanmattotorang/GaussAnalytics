//! OpenAI (and OpenAI-compatible / Azure) `LlmService`.
//!
//! Uses the Chat Completions API over `reqwest`. Payload construction and
//! response parsing are factored into pure functions so they can be unit-tested
//! without a live endpoint.

use crate::accumulator::ToolCallAccumulator;
use async_trait::async_trait;
use futures::StreamExt;
use gauss_engine::error::{AgentError, Result};
use gauss_engine::model::llm::{LlmRequest, LlmResponse, LlmStreamChunk};
use gauss_engine::model::tool::ToolCall;
use gauss_engine::traits::{LlmChunkStream, LlmService};
use serde_json::{json, Map, Value};
use uuid::Uuid;

/// A single streamed tool-call delta: (index, id?, name?, args fragment?).
type ToolCallDelta = (usize, Option<String>, Option<String>, Option<String>);

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// OpenAI Chat Completions LLM service.
pub struct OpenAiLlmService {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAiLlmService {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// Point at an OpenAI-compatible endpoint (Azure OpenAI, local gateways…).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

/// Build the Chat Completions request body. Pure + testable.
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
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                // OpenAI expects arguments as a JSON string.
                                "arguments": Value::Object(tc.arguments.clone()).to_string(),
                            }
                        })
                    })
                    .collect();
                messages.push(json!({
                    "role": "assistant",
                    "content": if m.content.is_empty() { Value::Null } else { json!(m.content) },
                    "tool_calls": calls,
                }));
            }
            "tool" => messages.push(json!({
                "role": "tool",
                "tool_call_id": m.tool_call_id,
                "content": m.content,
            })),
            _ => messages.push(json!({ "role": m.role, "content": m.content })),
        }
    }

    let mut payload = json!({
        "model": model,
        "messages": messages,
        "temperature": request.temperature,
        "stream": false,
    });

    if let Some(max) = request.max_tokens {
        payload["max_tokens"] = json!(max);
    }
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
            payload["tool_choice"] = json!("auto");
        }
    }

    payload
}

/// Parse a Chat Completions response body. Pure + testable.
pub(crate) fn parse_response(body: &Value) -> Result<LlmResponse> {
    let choice = body
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| AgentError::LlmService("response missing choices".into()))?;
    let message = choice
        .get("message")
        .ok_or_else(|| AgentError::LlmService("choice missing message".into()))?;

    let content = message
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(parse_tool_call)
                .collect::<Vec<ToolCall>>()
        })
        .filter(|v| !v.is_empty());

    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(LlmResponse {
        content,
        tool_calls,
        finish_reason,
        ..Default::default()
    })
}

/// Parse one streaming chunk's delta into (content, tool-call deltas).
pub(crate) fn parse_stream_chunk(v: &Value) -> (Option<String>, Vec<ToolCallDelta>) {
    let Some(delta) = v.pointer("/choices/0/delta") else {
        return (None, Vec::new());
    };
    let content = delta
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut deltas = Vec::new();
    if let Some(arr) = delta.get("tool_calls").and_then(Value::as_array) {
        for tc in arr {
            let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let id = tc.get("id").and_then(Value::as_str).map(str::to_string);
            let func = tc.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let args = func
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .map(str::to_string);
            deltas.push((index, id, name, args));
        }
    }
    (content, deltas)
}

fn parse_tool_call(v: &Value) -> Option<ToolCall> {
    let func = v.get("function")?;
    let name = func.get("name")?.as_str()?.to_string();
    // Arguments arrive as a JSON-encoded string.
    let arguments = func
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_else(Map::new);
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .map_or_else(|| Uuid::new_v4().to_string(), str::to_string);
    Some(ToolCall {
        id,
        name,
        arguments,
    })
}

#[async_trait]
impl LlmService for OpenAiLlmService {
    async fn send_request(&self, request: LlmRequest) -> Result<LlmResponse> {
        let payload = build_payload(&self.model, &request);
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
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
                "OpenAI API error {status}: {body}"
            )));
        }
        parse_response(&body)
    }

    async fn stream_request<'a>(&'a self, request: LlmRequest) -> LlmChunkStream<'a> {
        let mut payload = build_payload(&self.model, &request);
        payload["stream"] = json!(true);
        // Own everything so the returned stream is self-contained.
        let client = self.client.clone();
        let url = format!("{}/chat/completions", self.base_url);
        let api_key = self.api_key.clone();

        Box::pin(async_stream::stream! {
            let resp = match client.post(&url).bearer_auth(&api_key).json(&payload).send().await {
                Ok(r) => r,
                Err(e) => {
                    yield Err(AgentError::LlmService(format!("stream request failed: {e}")));
                    return;
                }
            };
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                yield Err(AgentError::LlmService(format!("OpenAI API error {status}: {body}")));
                return;
            }

            let mut acc = ToolCallAccumulator::new();
            let mut buf: Vec<u8> = Vec::new();
            let mut byte_stream = resp.bytes_stream();
            while let Some(item) = byte_stream.next().await {
                let bytes = match item {
                    Ok(b) => b,
                    Err(e) => {
                        yield Err(AgentError::LlmService(format!("stream read error: {e}")));
                        return;
                    }
                };
                buf.extend_from_slice(&bytes);
                // Process complete newline-terminated lines (SSE frames).
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line);
                    let line = line.trim();
                    let Some(data) = line.strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data == "[DONE]" {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        let (content, deltas) = parse_stream_chunk(&v);
                        for (idx, id, name, args) in deltas {
                            acc.push_delta(idx, id.as_deref(), name.as_deref(), args.as_deref());
                        }
                        if let Some(c) = content {
                            yield Ok(LlmStreamChunk { content: Some(c), ..Default::default() });
                        }
                    }
                }
            }
            // Emit any assembled tool calls at the end of the stream.
            let calls = acc.finish();
            if !calls.is_empty() {
                yield Ok(LlmStreamChunk { tool_calls: Some(calls), ..Default::default() });
            }
        })
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_engine::model::llm::LlmMessage;
    use gauss_engine::model::tool::ToolSchema;
    use gauss_engine::model::user::User;

    fn req() -> LlmRequest {
        LlmRequest {
            messages: vec![LlmMessage::new("user", "hi")],
            tools: Some(vec![ToolSchema {
                name: "run_sql".into(),
                description: "run sql".into(),
                parameters: json!({"type": "object"}),
                access_groups: vec![],
            }]),
            user: User::new("u"),
            stream: false,
            temperature: 0.5,
            max_tokens: Some(100),
            system_prompt: Some("be helpful".into()),
            metadata: Default::default(),
        }
    }

    #[test]
    fn payload_has_system_tools_and_params() {
        let p = build_payload("gpt-4o", &req());
        assert_eq!(p["model"], "gpt-4o");
        assert_eq!(p["messages"][0]["role"], "system");
        assert_eq!(p["messages"][1]["content"], "hi");
        assert_eq!(p["tools"][0]["function"]["name"], "run_sql");
        assert_eq!(p["tool_choice"], "auto");
        assert_eq!(p["temperature"], 0.5);
        assert_eq!(p["max_tokens"], 100);
    }

    #[test]
    fn parse_text_response() {
        let body = json!({
            "choices": [{ "message": { "content": "hello" }, "finish_reason": "stop" }]
        });
        let r = parse_response(&body).unwrap();
        assert_eq!(r.content.as_deref(), Some("hello"));
        assert!(!r.is_tool_call());
    }

    #[test]
    fn parse_stream_chunk_content_and_tool_deltas() {
        let content_chunk = json!({ "choices": [{ "delta": { "content": "Hel" } }] });
        let (c, d) = parse_stream_chunk(&content_chunk);
        assert_eq!(c.as_deref(), Some("Hel"));
        assert!(d.is_empty());

        let tool_chunk = json!({ "choices": [{ "delta": { "tool_calls": [{
            "index": 0, "id": "call_1",
            "function": { "name": "run_sql", "arguments": "{\"sql\":" }
        }]}}]});
        let (_c, d) = parse_stream_chunk(&tool_chunk);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].0, 0);
        assert_eq!(d[0].1.as_deref(), Some("call_1"));
        assert_eq!(d[0].2.as_deref(), Some("run_sql"));
        assert_eq!(d[0].3.as_deref(), Some("{\"sql\":"));
    }

    #[test]
    fn parse_tool_call_response() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "run_sql", "arguments": "{\"sql\":\"SELECT 1\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let r = parse_response(&body).unwrap();
        assert!(r.is_tool_call());
        let tc = &r.tool_calls.unwrap()[0];
        assert_eq!(tc.name, "run_sql");
        assert_eq!(tc.arguments["sql"], "SELECT 1");
        assert_eq!(tc.id, "call_1");
    }
}
