//! Audit event model. Mirrors `gauss/core/audit/models.py`.
//!
//! A single `AuditEvent` struct carries the common fields plus a `details` blob
//! (rather than a class hierarchy); helper constructors on `AuditLogger`
//! populate `details` for each event kind. Audit JSON is for compliance sinks,
//! not the frontend, so it is not a wire contract.

use crate::model::user::User;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    ToolAccessCheck,
    UiFeatureAccessCheck,
    ToolInvocation,
    ToolResult,
    MessageReceived,
    AiResponseGenerated,
    ConversationCreated,
    AccessDenied,
    AuthenticationAttempt,
}

/// A single audit record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub event_type: AuditEventType,
    pub timestamp: DateTime<Utc>,
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
    pub user_groups: Vec<String>,
    pub conversation_id: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_addr: Option<String>,
    pub details: Map<String, Value>,
    pub contains_pii: bool,
    pub redacted_fields: Vec<String>,
}

impl AuditEvent {
    pub fn new(
        event_type: AuditEventType,
        user: &User,
        conversation_id: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4().to_string(),
            event_type,
            timestamp: Utc::now(),
            user_id: user.id.clone(),
            username: user.username.clone(),
            user_email: user.email.clone(),
            user_groups: user.group_memberships.clone(),
            conversation_id: conversation_id.into(),
            request_id: request_id.into(),
            remote_addr: None,
            details: Map::new(),
            contains_pii: false,
            redacted_fields: Vec::new(),
        }
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: Value) -> Self {
        self.details.insert(key.into(), value);
        self
    }
}

/// Mask values whose key names suggest secrets. Returns the sanitized map and
/// the list of redacted field names.
pub fn sanitize_parameters(params: &Map<String, Value>) -> (Map<String, Value>, Vec<String>) {
    const SENSITIVE: &[&str] = &[
        "password",
        "secret",
        "token",
        "api_key",
        "apikey",
        "credential",
        "auth",
        "private_key",
        "access_key",
    ];
    let mut out = Map::new();
    let mut redacted = Vec::new();
    for (k, v) in params {
        let lk = k.to_lowercase();
        if SENSITIVE.iter().any(|s| lk.contains(s)) {
            out.insert(k.clone(), Value::String("***REDACTED***".into()));
            redacted.push(k.clone());
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    (out, redacted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_type_wire_values() {
        assert_eq!(
            serde_json::to_value(AuditEventType::AiResponseGenerated).unwrap(),
            json!("ai_response_generated")
        );
        assert_eq!(
            serde_json::to_value(AuditEventType::ToolAccessCheck).unwrap(),
            json!("tool_access_check")
        );
    }

    #[test]
    fn sanitization_masks_secrets() {
        let mut p = Map::new();
        p.insert("sql".into(), json!("SELECT 1"));
        p.insert("api_key".into(), json!("sk-123"));
        p.insert("PASSWORD".into(), json!("hunter2"));
        let (clean, redacted) = sanitize_parameters(&p);
        assert_eq!(clean["sql"], json!("SELECT 1"));
        assert_eq!(clean["api_key"], json!("***REDACTED***"));
        assert_eq!(clean["PASSWORD"], json!("***REDACTED***"));
        assert_eq!(redacted.len(), 2);
    }
}
