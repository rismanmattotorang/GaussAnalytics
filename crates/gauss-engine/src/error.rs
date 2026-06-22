//! Error types for the GaussAnalytics core.
//!
//! Mirrors the Python exception hierarchy in `gauss/core/errors.py`
//! (`AgentError` and its subclasses) as a single `thiserror` enum.

use thiserror::Error;

/// Top-level error type for agent operations.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("tool execution error: {0}")]
    ToolExecution(String),

    #[error("tool not found: {0}")]
    ToolNotFound(String),

    #[error("permission denied: {0}")]
    Permission(String),

    #[error("conversation not found: {0}")]
    ConversationNotFound(String),

    #[error("LLM service error: {0}")]
    LlmService(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

/// Convenience result alias used across the crate.
pub type Result<T> = std::result::Result<T, AgentError>;

impl AgentError {
    pub fn other(msg: impl Into<String>) -> Self {
        AgentError::Other(msg.into())
    }
}
