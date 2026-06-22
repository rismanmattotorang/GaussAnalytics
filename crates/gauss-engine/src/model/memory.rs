//! Agent-memory data models.
//! Mirrors `gauss/capabilities/agent_memory/models.py`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A recorded "question → tool invocation" pair, used to teach the agent
/// which tool/args worked for similar questions in the past.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMemory {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
    pub question: String,
    pub tool_name: String,
    #[serde(default)]
    pub args: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default = "default_true")]
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

fn default_true() -> bool {
    true
}

/// A free-form text memory (domain knowledge, schema notes, terminology).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextMemory {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMemorySearchResult {
    pub memory: ToolMemory,
    pub similarity_score: f32,
    pub rank: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextMemorySearchResult {
    pub memory: TextMemory,
    pub similarity_score: f32,
    pub rank: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total_memories: u32,
    pub unique_tools: u32,
    pub unique_questions: u32,
    pub success_rate: f32,
    pub most_used_tools: Map<String, Value>,
}
