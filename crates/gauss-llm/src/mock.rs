//! A deterministic mock LLM that exercises the full tool loop.
//!
//! Behavior:
//! - If the most recent message is a tool result, it produces a final natural
//!   language answer (ending the loop).
//! - Otherwise, if a `run_sql` tool is available, it emits a `run_sql` tool
//!   call. The SQL is taken from the user's message when it looks like SQL,
//!   else it defaults to listing the database's tables.
//! - With no tools, it echoes the user's message.

use async_trait::async_trait;
use gauss_engine::error::Result;
use gauss_engine::model::llm::{LlmRequest, LlmResponse};
use gauss_engine::model::tool::ToolCall;
use gauss_engine::traits::LlmService;
use serde_json::json;
use uuid::Uuid;

#[derive(Default)]
pub struct MockLlmService;

impl MockLlmService {
    pub fn new() -> Self {
        Self
    }

    fn latest_user_message(req: &LlmRequest) -> Option<&str> {
        req.messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
    }

    fn last_role(req: &LlmRequest) -> Option<&str> {
        req.messages.last().map(|m| m.role.as_str())
    }

    fn has_tool(req: &LlmRequest, name: &str) -> bool {
        req.tools
            .as_ref()
            .is_some_and(|ts| ts.iter().any(|t| t.name == name))
    }

    /// Extract table names from any `CREATE TABLE ...` statements that the
    /// schema-context enhancer injected into the system prompt. Handles both
    /// quoted (`"t"`) and bare identifiers.
    fn schema_tables(req: &LlmRequest) -> Vec<String> {
        let Some(sp) = req.system_prompt.as_deref() else {
            return Vec::new();
        };
        let bytes = sp.as_bytes();
        let lower = sp.to_lowercase();
        let needle = "create table ";
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(p) = lower[from..].find(needle) {
            let mut i = from + p + needle.len();
            while i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
            }
            let start = i;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c.is_ascii_alphanumeric() || c == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            if i > start {
                out.push(sp[start..i].to_string());
            }
            from = (from + p + needle.len()).max(i);
        }
        out
    }

    /// Choose SQL for a natural-language question, grounded in the injected
    /// schema: run a pasted SELECT verbatim; else preview a table named in the
    /// question (or the first table for a generic ask); else list the tables.
    fn choose_sql(req: &LlmRequest, user_msg: &str) -> String {
        let lower = user_msg.to_lowercase();
        if lower.contains("select ") {
            return user_msg.trim().to_string();
        }
        let tables = Self::schema_tables(req);
        if let Some(t) = tables.iter().find(|t| lower.contains(&t.to_lowercase())) {
            return format!("SELECT * FROM \"{t}\" LIMIT 50;");
        }
        let generic = [
            "show", "list", "all", "data", "rows", "preview", "sample", "what", "how many",
            "count", "top", "average", "sum", "total",
        ]
        .iter()
        .any(|k| lower.contains(k));
        if let Some(first) = tables.first().filter(|_| generic) {
            return format!("SELECT * FROM \"{first}\" LIMIT 50;");
        }
        "SELECT name FROM sqlite_master WHERE type='table';".to_string()
    }
}

#[async_trait]
impl LlmService for MockLlmService {
    async fn send_request(&self, request: LlmRequest) -> Result<LlmResponse> {
        // After a tool has run, summarize and finish.
        if Self::last_role(&request) == Some("tool") {
            let tool_output = request
                .messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            return Ok(LlmResponse {
                content: Some(format!(
                    "Here are the results of your query:\n\n```\n{}\n```",
                    tool_output.trim()
                )),
                finish_reason: Some("stop".into()),
                ..Default::default()
            });
        }

        // Otherwise, drive a run_sql tool call if available.
        if Self::has_tool(&request, "run_sql") {
            let user_msg = Self::latest_user_message(&request).unwrap_or("");
            let sql = Self::choose_sql(&request, user_msg);
            return Ok(LlmResponse {
                content: Some("Let me query the database for that.".into()),
                tool_calls: Some(vec![ToolCall {
                    id: Uuid::new_v4().to_string(),
                    name: "run_sql".into(),
                    arguments: json!({ "sql": sql })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                }]),
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            });
        }

        // No tools: echo.
        let user_msg = Self::latest_user_message(&request).unwrap_or("(no message)");
        Ok(LlmResponse {
            content: Some(format!("You said: {user_msg}")),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }

    fn model(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_engine::model::llm::LlmMessage;
    use gauss_engine::model::tool::ToolSchema;
    use gauss_engine::model::user::User;

    fn req(system_prompt: Option<&str>, user: &str, with_run_sql: bool) -> LlmRequest {
        LlmRequest {
            messages: vec![LlmMessage::new("user", user)],
            tools: with_run_sql.then(|| {
                vec![ToolSchema {
                    name: "run_sql".into(),
                    description: "run sql".into(),
                    parameters: serde_json::json!({}),
                    access_groups: vec![],
                }]
            }),
            user: User::new("u"),
            stream: false,
            temperature: 0.0,
            max_tokens: None,
            system_prompt: system_prompt.map(str::to_string),
            metadata: serde_json::Map::new(),
        }
    }

    const SCHEMA: &str = "## Database schema\n```sql\nCREATE TABLE \"sales_csv\" \
        (\"region\" TEXT, \"amount\" REAL);\nCREATE TABLE customers (id INTEGER);\n```";

    #[tokio::test]
    async fn previews_table_named_in_question() {
        let r = MockLlmService::new()
            .send_request(req(
                Some(SCHEMA),
                "what is the total amount in sales_csv?",
                true,
            ))
            .await
            .unwrap();
        let call = &r.tool_calls.unwrap()[0];
        assert_eq!(call.name, "run_sql");
        let sql = call.arguments["sql"].as_str().unwrap();
        assert!(sql.contains("sales_csv"), "got: {sql}");
    }

    #[tokio::test]
    async fn generic_ask_previews_first_table() {
        let r = MockLlmService::new()
            .send_request(req(Some(SCHEMA), "show me the data", true))
            .await
            .unwrap();
        let sql = r.tool_calls.unwrap()[0].arguments["sql"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(sql.contains("sales_csv"), "got: {sql}");
    }

    #[tokio::test]
    async fn no_schema_lists_tables() {
        let r = MockLlmService::new()
            .send_request(req(None, "hello there", true))
            .await
            .unwrap();
        let sql = r.tool_calls.unwrap()[0].arguments["sql"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(sql.contains("sqlite_master"), "got: {sql}");
    }
}
