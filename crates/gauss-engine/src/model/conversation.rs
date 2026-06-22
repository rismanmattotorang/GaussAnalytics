//! Conversation & message models.
//! Mirrors `gauss/core/storage/models.py`.

use super::tool::ToolCall;
use super::user::User;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A single message in a stored conversation. `role` is one of
/// `user` | `assistant` | `system` | `tool`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(default = "Utc::now")]
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", content)
    }

    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            timestamp: Utc::now(),
            metadata: Map::new(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn with_tool_calls(mut self, calls: Vec<ToolCall>) -> Self {
        self.tool_calls = Some(calls);
        self
    }

    pub fn tool_response(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        let mut m = Self::new("tool", content);
        m.tool_call_id = Some(tool_call_id.into());
        m
    }
}

/// A conversation: an ordered list of messages owned by a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub user: User,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

impl Conversation {
    pub fn new(id: impl Into<String>, user: User) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            user,
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: Map::new(),
        }
    }

    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
        self.updated_at = Utc::now();
    }
}
