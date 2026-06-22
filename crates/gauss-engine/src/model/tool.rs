//! Tool-related data models (the data side; the `Tool` trait lives in `crate::tool`).
//! Mirrors `gauss/core/tool/models.py`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A request from the LLM to invoke a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw arguments produced by the LLM, validated against the tool's schema
    /// at execution time.
    #[serde(default)]
    pub arguments: Map<String, Value>,
}

/// The JSON-schema description of a tool exposed to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON Schema of the tool's parameters.
    pub parameters: Value,
    #[serde(default)]
    pub access_groups: Vec<String>,
}

/// Returned by argument-transformation hooks to reject a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRejection {
    pub reason: String,
}
