//! Google Gemini (Generative Language API) `LlmService`.
//!
//! Gemini differs from OpenAI in several ways this conversion handles:
//! - the system prompt is a top-level `system_instruction`;
//! - turns use `contents[].parts[]` with `text` / `functionCall` / `functionResponse`;
//! - the assistant role is `model`;
//! - tool results are `functionResponse` parts (which need the function *name*,
//!   recovered here by matching `tool_call_id` back to the call that produced it);
//! - tool parameter schemas are cleaned of keys Gemini rejects.

use async_trait::async_trait;
use gauss_engine::error::{AgentError, Result};
use gauss_engine::model::llm::{LlmRequest, LlmResponse};
use gauss_engine::model::tool::ToolCall;
use gauss_engine::traits::LlmService;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use uuid::Uuid;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct GeminiLlmService {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl GeminiLlmService {
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

/// Remove JSON-Schema keys Gemini's function declarations reject.
fn clean_schema(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, val) in map {
                if matches!(
                    k.as_str(),
                    "$schema" | "title" | "additionalProperties" | "definitions" | "$defs" | "$id"
                ) {
                    continue;
                }
                out.insert(k.clone(), clean_schema(val));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(clean_schema).collect()),
        other => other.clone(),
    }
}

pub(crate) fn build_payload(request: &LlmRequest) -> Value {
    // Resolve tool_call_id → function name from prior assistant tool calls so
    // tool results can be expressed as Gemini functionResponse parts.
    let mut id_to_name: HashMap<String, String> = HashMap::new();
    for m in &request.messages {
        if let Some(tcs) = &m.tool_calls {
            for tc in tcs {
                id_to_name.insert(tc.id.clone(), tc.name.clone());
            }
        }
    }

    let mut contents: Vec<Value> = Vec::new();
    for m in &request.messages {
        match m.role.as_str() {
            "assistant" => {
                let mut parts: Vec<Value> = Vec::new();
                if !m.content.is_empty() {
                    parts.push(json!({ "text": m.content }));
                }
                if let Some(tcs) = &m.tool_calls {
                    for tc in tcs {
                        parts.push(json!({
                            "functionCall": { "name": tc.name, "args": tc.arguments }
                        }));
                    }
                }
                contents.push(json!({ "role": "model", "parts": parts }));
            }
            "tool" => {
                let name = m
                    .tool_call_id
                    .as_ref()
                    .and_then(|id| id_to_name.get(id))
                    .cloned()
                    .unwrap_or_else(|| "tool".to_string());
                contents.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": name,
                            "response": { "content": m.content }
                        }
                    }]
                }));
            }
            _ => contents.push(json!({
                "role": "user",
                "parts": [{ "text": m.content }],
            })),
        }
    }

    let mut payload = json!({
        "contents": contents,
        "generationConfig": { "temperature": request.temperature },
    });
    if let Some(max) = request.max_tokens {
        payload["generationConfig"]["maxOutputTokens"] = json!(max);
    }
    if let Some(sp) = &request.system_prompt {
        payload["system_instruction"] = json!({ "parts": [{ "text": sp }] });
    }
    if let Some(tools) = &request.tools {
        if !tools.is_empty() {
            let decls: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": clean_schema(&t.parameters),
                    })
                })
                .collect();
            payload["tools"] = json!([{ "function_declarations": decls }]);
        }
    }
    payload
}

pub(crate) fn parse_response(body: &Value) -> Result<LlmResponse> {
    let parts = body
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .ok_or_else(|| AgentError::LlmService("gemini: missing candidates/content/parts".into()))?;

    let mut text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    for part in parts {
        if let Some(t) = part.get("text").and_then(Value::as_str) {
            text.push_str(t);
        }
        if let Some(fc) = part.get("functionCall") {
            let name = fc
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = fc
                .get("args")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_else(Map::new);
            tool_calls.push(ToolCall {
                id: Uuid::new_v4().to_string(),
                name,
                arguments,
            });
        }
    }

    Ok(LlmResponse {
        content: (!text.is_empty()).then_some(text),
        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
        finish_reason: body
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    })
}

#[async_trait]
impl LlmService for GeminiLlmService {
    async fn send_request(&self, request: LlmRequest) -> Result<LlmResponse> {
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, self.model, self.api_key
        );
        let resp = self
            .client
            .post(&url)
            .json(&build_payload(&request))
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
                "Gemini API error {status}: {body}"
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
    use gauss_engine::model::tool::ToolSchema;
    use gauss_engine::model::user::User;

    fn req(messages: Vec<LlmMessage>, tools: Option<Vec<ToolSchema>>) -> LlmRequest {
        LlmRequest {
            messages,
            tools,
            user: User::new("u"),
            stream: false,
            temperature: 0.2,
            max_tokens: Some(256),
            system_prompt: Some("be precise".into()),
            metadata: Default::default(),
        }
    }

    #[test]
    fn payload_maps_roles_and_system_and_cleans_schema() {
        let tools = vec![ToolSchema {
            name: "run_sql".into(),
            description: "run".into(),
            parameters: json!({"$schema":"x","title":"Args","type":"object","properties":{"sql":{"type":"string"}}}),
            access_groups: vec![],
        }];
        let p = build_payload(&req(vec![LlmMessage::new("user", "hi")], Some(tools)));
        assert_eq!(p["system_instruction"]["parts"][0]["text"], "be precise");
        assert_eq!(p["contents"][0]["role"], "user");
        assert_eq!(p["contents"][0]["parts"][0]["text"], "hi");
        assert_eq!(p["generationConfig"]["temperature"], 0.2);
        let params = &p["tools"][0]["function_declarations"][0]["parameters"];
        assert!(params.get("$schema").is_none());
        assert!(params.get("title").is_none());
        assert_eq!(params["properties"]["sql"]["type"], "string");
    }

    #[test]
    fn tool_result_uses_resolved_function_name() {
        let assistant = LlmMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".into(),
                name: "run_sql".into(),
                arguments: Map::new(),
            }]),
            tool_call_id: None,
        };
        let tool = LlmMessage {
            role: "tool".into(),
            content: "rows".into(),
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
        };
        let p = build_payload(&req(vec![assistant, tool], None));
        // contents: [model(functionCall), user(functionResponse)]
        let fr = &p["contents"][1]["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "run_sql");
        assert_eq!(fr["response"]["content"], "rows");
    }

    #[test]
    fn parse_text_and_function_call() {
        let body = json!({
            "candidates": [{
                "content": { "parts": [
                    { "text": "sure" },
                    { "functionCall": { "name": "run_sql", "args": { "sql": "SELECT 1" } } }
                ]},
                "finishReason": "STOP"
            }]
        });
        let r = parse_response(&body).unwrap();
        assert_eq!(r.content.as_deref(), Some("sure"));
        let tc = &r.tool_calls.unwrap()[0];
        assert_eq!(tc.name, "run_sql");
        assert_eq!(tc.arguments["sql"], "SELECT 1");
    }
}
