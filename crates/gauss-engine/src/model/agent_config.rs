//! Agent configuration, UI-feature gating, and audit config.
//! Mirrors `gauss/core/agent/config.py`.

use super::user::User;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Named UI features that can be gated per user group. The string values are
/// the canonical wire identifiers (kept identical to the Python source).
pub mod ui_feature {
    pub const SHOW_TOOL_NAMES: &str = "tool_names";
    pub const SHOW_TOOL_ARGUMENTS: &str = "tool_arguments";
    pub const SHOW_TOOL_ERROR: &str = "tool_error";
    pub const SHOW_TOOL_INVOCATION_MESSAGE_IN_CHAT: &str = "tool_invocation_message_in_chat";
    pub const SHOW_MEMORY_DETAILED_RESULTS: &str = "memory_detailed_results";
}

/// Maps UI-feature name → the access groups allowed to see it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiFeatures {
    #[serde(default)]
    pub feature_group_access: HashMap<String, Vec<String>>,
}

impl Default for UiFeatures {
    fn default() -> Self {
        // By default, tool internals are visible only to the `admin` group;
        // the in-chat tool invocation message is visible to everyone ("").
        let admin = || vec!["admin".to_string()];
        let mut m = HashMap::new();
        m.insert(ui_feature::SHOW_TOOL_NAMES.to_string(), admin());
        m.insert(ui_feature::SHOW_TOOL_ARGUMENTS.to_string(), admin());
        m.insert(ui_feature::SHOW_TOOL_ERROR.to_string(), admin());
        m.insert(
            ui_feature::SHOW_TOOL_INVOCATION_MESSAGE_IN_CHAT.to_string(),
            vec![],
        );
        m.insert(
            ui_feature::SHOW_MEMORY_DETAILED_RESULTS.to_string(),
            admin(),
        );
        Self {
            feature_group_access: m,
        }
    }
}

impl UiFeatures {
    /// Whether `user` may access `feature_name`. Unknown features and empty
    /// group lists are treated as "allowed for everyone".
    pub fn can_user_access_feature(&self, feature_name: &str, user: &User) -> bool {
        match self.feature_group_access.get(feature_name) {
            None => true,
            Some(groups) => user.can_access(groups),
        }
    }

    pub fn register_feature(&mut self, name: impl Into<String>, access_groups: Vec<String>) {
        self.feature_group_access.insert(name.into(), access_groups);
    }
}

/// Controls which events the audit logger records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditConfig {
    #[serde(default = "tru")]
    pub enabled: bool,
    #[serde(default = "tru")]
    pub log_tool_access_checks: bool,
    #[serde(default = "tru")]
    pub log_tool_invocations: bool,
    #[serde(default = "tru")]
    pub log_tool_results: bool,
    #[serde(default)]
    pub log_ui_feature_checks: bool,
    #[serde(default = "tru")]
    pub log_ai_responses: bool,
    #[serde(default)]
    pub include_full_ai_responses: bool,
    #[serde(default = "tru")]
    pub sanitize_tool_parameters: bool,
}

fn tru() -> bool {
    true
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            log_tool_access_checks: true,
            log_tool_invocations: true,
            log_tool_results: true,
            log_ui_feature_checks: false,
            log_ai_responses: true,
            include_full_ai_responses: false,
            sanitize_tool_parameters: true,
        }
    }
}

/// Top-level agent behavior configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_max_iters")]
    pub max_tool_iterations: u32,
    #[serde(default = "tru")]
    pub stream_responses: bool,
    #[serde(default = "tru")]
    pub auto_save_conversations: bool,
    #[serde(default = "tru")]
    pub include_thinking_indicators: bool,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub ui_features: UiFeatures,
    #[serde(default)]
    pub audit_config: AuditConfig,
}

fn default_max_iters() -> u32 {
    10
}
fn default_temperature() -> f64 {
    0.7
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_tool_iterations: 10,
            stream_responses: true,
            auto_save_conversations: true,
            include_thinking_indicators: true,
            temperature: 0.7,
            max_tokens: None,
            ui_features: UiFeatures::default(),
            audit_config: AuditConfig::default(),
        }
    }
}
