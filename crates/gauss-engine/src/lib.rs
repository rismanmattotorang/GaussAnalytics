//! # gauss-engine
//!
//! Core of the GaussAnalytics platform — a Rust port of the GaussAnalytics user-aware
//! NL2SQL agent framework. Contains the data models, capability/extensibility
//! traits, the tool registry, the UI component system, and the agent loop.

pub mod agent;
pub mod components;
pub mod context;
pub mod dataframe;
pub mod defaults;
pub mod enhancer;
pub mod error;
pub mod model;
pub mod prompt;
pub mod recovery;
pub mod tool;
pub mod traits;
pub mod workflow;

// Convenient re-exports.
pub use context::{ToolContext, ToolResult};
pub use dataframe::DataFrame;
pub use error::{AgentError, Result};
pub use tool::{DynTool, Tool, ToolRegistry};

pub use model::agent_config::{AgentConfig, AuditConfig, UiFeatures};
pub use model::conversation::{Conversation, Message};
pub use model::llm::{LlmMessage, LlmRequest, LlmResponse, LlmStreamChunk};
pub use model::memory::{TextMemory, ToolMemory};
pub use model::tool::{ToolCall, ToolSchema};
pub use model::user::{RequestContext, User};
pub use traits::{CommandResult, FileSearchMatch};

pub use agent::Agent;
pub use components::{RichComponent, SimpleComponent, UiComponent};
