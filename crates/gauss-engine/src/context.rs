//! `ToolContext` and `ToolResult` — the execution context handed to every tool
//! and the structured result it returns.
//! Mirrors `gauss/core/tool/models.py` (ToolContext, ToolResult).

use crate::components::UiComponent;
use crate::model::user::User;
use crate::traits::{AgentMemory, ObservabilityProvider};
use serde_json::{Map, Value};
use std::sync::Arc;

/// Context injected into a tool when it executes. Carries the user (for
/// permission-aware behavior), tracing IDs, the agent memory, and arbitrary
/// enricher-supplied metadata.
#[derive(Clone)]
pub struct ToolContext {
    pub user: User,
    pub conversation_id: String,
    pub request_id: String,
    pub agent_memory: Arc<dyn AgentMemory>,
    pub observability_provider: Option<Arc<dyn ObservabilityProvider>>,
    pub metadata: Map<String, Value>,
}

impl ToolContext {
    pub fn new(
        user: User,
        conversation_id: impl Into<String>,
        request_id: impl Into<String>,
        agent_memory: Arc<dyn AgentMemory>,
    ) -> Self {
        Self {
            user,
            conversation_id: conversation_id.into(),
            request_id: request_id.into(),
            agent_memory,
            observability_provider: None,
            metadata: Map::new(),
        }
    }
}

/// The structured outcome of a tool execution.
#[derive(Clone)]
pub struct ToolResult {
    pub success: bool,
    /// String content fed back to the LLM as the tool's `tool` message.
    pub result_for_llm: String,
    /// Optional rich component streamed to the UI.
    pub ui_component: Option<UiComponent>,
    pub error: Option<String>,
    pub metadata: Map<String, Value>,
}

impl ToolResult {
    pub fn success(result_for_llm: impl Into<String>) -> Self {
        Self {
            success: true,
            result_for_llm: result_for_llm.into(),
            ui_component: None,
            error: None,
            metadata: Map::new(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            success: false,
            result_for_llm: message.clone(),
            ui_component: None,
            error: Some(message),
            metadata: Map::new(),
        }
    }

    pub fn with_ui(mut self, component: UiComponent) -> Self {
        self.ui_component = Some(component);
        self
    }
}
