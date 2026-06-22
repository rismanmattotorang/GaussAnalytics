//! Capability and extensibility traits.
//!
//! Each trait is the Rust equivalent of a Python ABC in the GaussAnalytics source.
//! Implementations live in sibling crates (`gauss-llm`, `gauss-sql`, …) or in
//! `crate::defaults`.

use crate::context::{ToolContext, ToolResult};
use crate::dataframe::DataFrame;
use crate::error::Result;
use crate::model::audit::{AuditEvent, AuditEventType};
use crate::model::conversation::{Conversation, Message};
use crate::model::llm::{LlmMessage, LlmRequest, LlmResponse, LlmStreamChunk};
use crate::model::memory::{
    TextMemory, TextMemorySearchResult, ToolMemory, ToolMemorySearchResult,
};
use crate::model::observability::Span;
use crate::model::tool::ToolSchema;
use crate::model::user::{RequestContext, User};
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::{Map, Value};

/// Streaming output of an LLM service.
pub type LlmChunkStream<'a> = BoxStream<'a, Result<LlmStreamChunk>>;

/// A large-language-model backend (Anthropic, OpenAI, Ollama, mock, …).
#[async_trait]
pub trait LlmService: Send + Sync {
    async fn send_request(&self, request: LlmRequest) -> Result<LlmResponse>;

    /// Stream a response as incremental chunks. Default implementation falls
    /// back to a single chunk built from `send_request`.
    async fn stream_request<'a>(&'a self, request: LlmRequest) -> LlmChunkStream<'a> {
        let res = self.send_request(request).await;
        Box::pin(async_stream::stream! {
            match res {
                Ok(r) => yield Ok(LlmStreamChunk {
                    content: r.content,
                    tool_calls: r.tool_calls,
                    finish_reason: r.finish_reason,
                    metadata: r.metadata,
                }),
                Err(e) => yield Err(e),
            }
        })
    }

    /// Optional model identifier (used for observability tagging).
    fn model(&self) -> &str {
        "unknown"
    }
}

/// Executes SQL against a database and returns a tabular result.
#[async_trait]
pub trait SqlRunner: Send + Sync {
    async fn run_sql(&self, sql: &str, context: &ToolContext) -> Result<DataFrame>;
}

/// Result of running a shell command.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// A search hit: a file path and an optional matching snippet.
#[derive(Debug, Clone)]
pub struct FileSearchMatch {
    pub path: String,
    pub snippet: Option<String>,
}

/// A user-aware file system abstraction used by data tools (write query CSVs,
/// read them back for charting, run scripts, search files, etc.).
#[async_trait]
pub trait FileSystem: Send + Sync {
    async fn read_file(&self, path: &str, context: &ToolContext) -> Result<String>;
    async fn write_file(
        &self,
        path: &str,
        content: &str,
        context: &ToolContext,
        overwrite: bool,
    ) -> Result<()>;
    async fn list_files(&self, directory: &str, context: &ToolContext) -> Result<Vec<String>>;
    async fn exists(&self, path: &str, context: &ToolContext) -> Result<bool>;

    /// Search files under the sandbox for `query` (substring match). Default
    /// implementation returns an empty result; backends may override.
    async fn search_files(
        &self,
        _query: &str,
        _context: &ToolContext,
        _max_results: usize,
    ) -> Result<Vec<FileSearchMatch>> {
        Ok(Vec::new())
    }

    /// Run a shell command within the sandbox. Default implementation refuses;
    /// backends that permit execution override it.
    async fn run_bash(
        &self,
        _command: &str,
        _context: &ToolContext,
        _timeout_secs: Option<u64>,
    ) -> Result<CommandResult> {
        Err(crate::error::AgentError::other(
            "command execution is not supported by this file system",
        ))
    }
}

/// Stores and retrieves agent memories (tool-usage patterns + text knowledge).
#[async_trait]
pub trait AgentMemory: Send + Sync {
    async fn save_tool_usage(
        &self,
        question: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &ToolContext,
        success: bool,
        metadata: Option<&Map<String, Value>>,
    ) -> Result<()>;

    async fn save_text_memory(&self, content: &str, context: &ToolContext) -> Result<TextMemory>;

    async fn search_similar_usage(
        &self,
        question: &str,
        context: &ToolContext,
        limit: usize,
        similarity_threshold: f32,
        tool_name_filter: Option<&str>,
    ) -> Result<Vec<ToolMemorySearchResult>>;

    async fn search_text_memories(
        &self,
        query: &str,
        context: &ToolContext,
        limit: usize,
        similarity_threshold: f32,
    ) -> Result<Vec<TextMemorySearchResult>>;

    async fn get_recent_memories(
        &self,
        context: &ToolContext,
        limit: usize,
    ) -> Result<Vec<ToolMemory>>;

    async fn get_recent_text_memories(
        &self,
        context: &ToolContext,
        limit: usize,
    ) -> Result<Vec<TextMemory>>;

    async fn delete_by_id(&self, context: &ToolContext, memory_id: &str) -> Result<bool>;

    async fn delete_text_memory(&self, context: &ToolContext, memory_id: &str) -> Result<bool>;

    async fn clear_memories(
        &self,
        context: &ToolContext,
        tool_name: Option<&str>,
        before_date: Option<&str>,
    ) -> Result<usize>;
}

/// Persists conversations per user.
#[async_trait]
pub trait ConversationStore: Send + Sync {
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user: &User,
    ) -> Result<Option<Conversation>>;

    async fn update_conversation(&self, conversation: &Conversation) -> Result<()>;

    async fn delete_conversation(&self, conversation_id: &str, user: &User) -> Result<bool>;

    async fn list_conversations(
        &self,
        user: &User,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Conversation>>;
}

/// Resolves an authenticated `User` from request-level context.
#[async_trait]
pub trait UserResolver: Send + Sync {
    async fn resolve_user(&self, request_context: &RequestContext) -> Result<User>;
}

/// Builds the system prompt for a request, given the user and available tools.
#[async_trait]
pub trait SystemPromptBuilder: Send + Sync {
    async fn build_system_prompt(
        &self,
        user: &User,
        tools: &[ToolSchema],
    ) -> Result<Option<String>>;
}

/// Augments the system prompt and message list with retrieved context (RAG).
#[async_trait]
pub trait LlmContextEnhancer: Send + Sync {
    async fn enhance_system_prompt(
        &self,
        system_prompt: String,
        _user_message: &str,
        _user: &User,
    ) -> String {
        system_prompt
    }

    async fn enhance_user_messages(
        &self,
        messages: Vec<LlmMessage>,
        _user: &User,
    ) -> Vec<LlmMessage> {
        messages
    }
}

/// Intercepts and transforms LLM requests/responses.
#[async_trait]
pub trait LlmMiddleware: Send + Sync {
    async fn before_llm_request(&self, request: LlmRequest) -> LlmRequest {
        request
    }
    async fn after_llm_response(
        &self,
        _request: &LlmRequest,
        response: LlmResponse,
    ) -> LlmResponse {
        response
    }
}

/// Hooks into message and tool execution lifecycle.
#[async_trait]
pub trait LifecycleHook: Send + Sync {
    async fn before_message(&self, _user: &User, _message: &str) -> Option<String> {
        None
    }
    async fn after_message(&self, _conversation: &Conversation) {}
    async fn before_tool(&self, _tool_name: &str, _context: &ToolContext) -> Result<()> {
        Ok(())
    }
    async fn after_tool(&self, _result: ToolResult) -> Option<ToolResult> {
        None
    }
}

/// Filters conversation history before it is sent to the LLM.
#[async_trait]
pub trait ConversationFilter: Send + Sync {
    async fn filter_messages(&self, messages: Vec<Message>) -> Vec<Message>;
}

/// Enriches the tool execution context with extra metadata.
#[async_trait]
pub trait ToolContextEnricher: Send + Sync {
    async fn enrich_context(&self, context: ToolContext) -> ToolContext;
}

/// Collects telemetry. Default methods are no-ops; `create_span` returns a span.
#[async_trait]
pub trait ObservabilityProvider: Send + Sync {
    async fn record_metric(
        &self,
        _name: &str,
        _value: f64,
        _unit: &str,
        _tags: Option<&std::collections::HashMap<String, String>>,
    ) {
    }
    async fn create_span(&self, name: &str) -> Span {
        Span::new(name)
    }
    async fn end_span(&self, span: &mut Span) {
        span.end();
    }
}

/// Records audit events. Implementations only need `log_event`; the convenience
/// helpers build the appropriate `AuditEvent` and delegate to it.
#[async_trait]
pub trait AuditLogger: Send + Sync {
    async fn log_event(&self, event: AuditEvent) -> Result<()>;

    async fn log_tool_access_check(
        &self,
        user: &User,
        tool_name: &str,
        access_granted: bool,
        required_groups: &[String],
        conversation_id: &str,
        request_id: &str,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ToolAccessCheck,
            user,
            conversation_id,
            request_id,
        )
        .with_detail("tool_name", Value::String(tool_name.to_string()))
        .with_detail("access_granted", Value::Bool(access_granted))
        .with_detail("required_groups", serde_json::json!(required_groups));
        let _ = self.log_event(event).await;
    }

    async fn log_tool_invocation(
        &self,
        user: &User,
        tool_call: &crate::model::tool::ToolCall,
        conversation_id: &str,
        request_id: &str,
        sanitize: bool,
    ) {
        let (params, redacted) = if sanitize {
            crate::model::audit::sanitize_parameters(&tool_call.arguments)
        } else {
            (tool_call.arguments.clone(), Vec::new())
        };
        let mut event = AuditEvent::new(
            AuditEventType::ToolInvocation,
            user,
            conversation_id,
            request_id,
        )
        .with_detail("tool_call_id", Value::String(tool_call.id.clone()))
        .with_detail("tool_name", Value::String(tool_call.name.clone()))
        .with_detail("parameters", Value::Object(params));
        event.redacted_fields = redacted;
        let _ = self.log_event(event).await;
    }

    async fn log_tool_result(
        &self,
        user: &User,
        tool_call: &crate::model::tool::ToolCall,
        result: &ToolResult,
        conversation_id: &str,
        request_id: &str,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ToolResult,
            user,
            conversation_id,
            request_id,
        )
        .with_detail("tool_call_id", Value::String(tool_call.id.clone()))
        .with_detail("tool_name", Value::String(tool_call.name.clone()))
        .with_detail("success", Value::Bool(result.success))
        .with_detail(
            "error",
            result
                .error
                .as_ref()
                .map_or(Value::Null, |e| Value::String(e.clone())),
        );
        let _ = self.log_event(event).await;
    }

    async fn log_ai_response(
        &self,
        user: &User,
        conversation_id: &str,
        request_id: &str,
        response_text: &str,
        tool_call_count: usize,
        include_full_text: bool,
    ) {
        let mut event = AuditEvent::new(
            AuditEventType::AiResponseGenerated,
            user,
            conversation_id,
            request_id,
        )
        .with_detail(
            "response_length_chars",
            serde_json::json!(response_text.chars().count()),
        )
        .with_detail("tool_calls_count", serde_json::json!(tool_call_count));
        if include_full_text {
            event = event.with_detail("response_text", Value::String(response_text.to_string()));
        }
        let _ = self.log_event(event).await;
    }
}
