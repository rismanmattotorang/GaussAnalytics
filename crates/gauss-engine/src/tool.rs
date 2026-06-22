//! The tool system: a typed `Tool` trait, an object-safe `DynTool` bridge, and
//! the `ToolRegistry`.
//!
//! This is the linchpin of the port (GAUSSANALYTICS_PORTING_PLAN.md §6.1). Tool
//! authors implement the ergonomic, strongly-typed `Tool` trait. A blanket impl
//! turns any `Tool` into a `DynTool` that accepts `serde_json` arguments and is
//! object-safe, so the registry can hold heterogeneous tools in one map and
//! validate-then-dispatch exactly like the Python `ToolRegistry.execute`.

use crate::context::{ToolContext, ToolResult};
use crate::model::agent_config::AuditConfig;
use crate::model::tool::{ToolCall, ToolSchema};
use crate::model::user::User;
use crate::traits::AuditLogger;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// A strongly-typed tool. `Args` is deserialized from the LLM's JSON arguments
/// and its JSON schema is advertised to the LLM.
#[async_trait]
pub trait Tool: Send + Sync {
    type Args: DeserializeOwned + JsonSchema + Send;

    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn access_groups(&self) -> Vec<String> {
        Vec::new()
    }
    async fn execute(&self, context: &ToolContext, args: Self::Args) -> ToolResult;
}

/// Object-safe tool interface stored by the registry: JSON in, `ToolResult` out.
#[async_trait]
pub trait DynTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn access_groups(&self) -> Vec<String>;
    fn schema(&self) -> ToolSchema;
    async fn execute_dyn(&self, context: &ToolContext, args: Map<String, Value>) -> ToolResult;
}

/// Blanket bridge: every typed `Tool` is a `DynTool`.
#[async_trait]
impl<T> DynTool for T
where
    T: Tool,
{
    fn name(&self) -> &str {
        Tool::name(self)
    }

    fn description(&self) -> &str {
        Tool::description(self)
    }

    fn access_groups(&self) -> Vec<String> {
        Tool::access_groups(self)
    }

    fn schema(&self) -> ToolSchema {
        let parameters = serde_json::to_value(schemars::schema_for!(T::Args)).unwrap_or(json!({
            "type": "object",
            "properties": {}
        }));
        ToolSchema {
            name: Tool::name(self).to_string(),
            description: Tool::description(self).to_string(),
            parameters,
            access_groups: Tool::access_groups(self),
        }
    }

    async fn execute_dyn(&self, context: &ToolContext, args: Map<String, Value>) -> ToolResult {
        match serde_json::from_value::<T::Args>(Value::Object(args)) {
            Ok(parsed) => self.execute(context, parsed).await,
            Err(e) => ToolResult::error(format!("Invalid arguments: {e}")),
        }
    }
}

/// Registry of available tools, keyed by name.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn DynTool>>,
    audit_logger: Option<Arc<dyn AuditLogger>>,
    audit_config: AuditConfig,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach an audit logger so tool access checks, invocations, and results
    /// are recorded.
    pub fn set_audit(&mut self, logger: Arc<dyn AuditLogger>, config: AuditConfig) {
        self.audit_logger = Some(logger);
        self.audit_config = config;
    }

    /// Register a typed tool. The tool advertises its own access groups.
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let arc: Arc<dyn DynTool> = Arc::new(tool);
        self.tools.insert(arc.name().to_string(), arc);
    }

    /// Register an already-boxed dynamic tool.
    pub fn register_arc(&mut self, tool: Arc<dyn DynTool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get_tool(&self, name: &str) -> Option<Arc<dyn DynTool>> {
        self.tools.get(name).cloned()
    }

    pub fn list_tools(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Tool schemas visible to `user` (filtered by group access).
    pub fn get_schemas(&self, user: &User) -> Vec<ToolSchema> {
        self.tools
            .values()
            .filter(|t| user.can_access(&t.access_groups()))
            .map(|t| t.schema())
            .collect()
    }

    /// Validate-and-execute a tool call. Mirrors `ToolRegistry.execute`:
    /// permission check → arg validation (inside `execute_dyn`) → execution.
    pub async fn execute(&self, tool_call: &ToolCall, context: &ToolContext) -> ToolResult {
        let Some(tool) = self.get_tool(&tool_call.name) else {
            return ToolResult::error(format!("Tool not found: {}", tool_call.name));
        };

        let groups = tool.access_groups();
        let granted = context.user.can_access(&groups);

        // Audit: access check.
        if let Some(audit) = self.audit_logger.as_ref() {
            if self.audit_config.enabled && self.audit_config.log_tool_access_checks {
                audit
                    .log_tool_access_check(
                        &context.user,
                        &tool_call.name,
                        granted,
                        &groups,
                        &context.conversation_id,
                        &context.request_id,
                    )
                    .await;
            }
        }

        if !granted {
            return ToolResult::error(format!(
                "Access denied: user lacks permission for tool '{}'",
                tool_call.name
            ));
        }

        // Audit: invocation.
        if let Some(audit) = self.audit_logger.as_ref() {
            if self.audit_config.enabled && self.audit_config.log_tool_invocations {
                audit
                    .log_tool_invocation(
                        &context.user,
                        tool_call,
                        &context.conversation_id,
                        &context.request_id,
                        self.audit_config.sanitize_tool_parameters,
                    )
                    .await;
            }
        }

        let result = tool.execute_dyn(context, tool_call.arguments.clone()).await;

        // Audit: result.
        if let Some(audit) = self.audit_logger.as_ref() {
            if self.audit_config.enabled && self.audit_config.log_tool_results {
                audit
                    .log_tool_result(
                        &context.user,
                        tool_call,
                        &result,
                        &context.conversation_id,
                        &context.request_id,
                    )
                    .await;
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::user::User;
    use schemars::JsonSchema;
    use serde::Deserialize;

    #[derive(Deserialize, JsonSchema)]
    struct EchoArgs {
        /// The text to echo back.
        text: String,
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        type Args = EchoArgs;
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echo text"
        }
        fn access_groups(&self) -> Vec<String> {
            vec!["admin".into()]
        }
        async fn execute(&self, _ctx: &ToolContext, args: EchoArgs) -> ToolResult {
            ToolResult::success(args.text)
        }
    }

    #[test]
    fn schema_derives_from_args() {
        let tool = EchoTool;
        let schema = DynTool::schema(&tool);
        assert_eq!(schema.name, "echo");
        // The derived JSON schema advertises the `text` property.
        assert!(schema.parameters["properties"]["text"].is_object());
        assert_eq!(schema.access_groups, vec!["admin".to_string()]);
    }

    #[test]
    fn get_schemas_filters_by_access() {
        let mut reg = ToolRegistry::new();
        reg.register(EchoTool);
        let admin = User::new("a").with_groups(["admin"]);
        let plain = User::new("b").with_groups(["user"]);
        assert_eq!(reg.get_schemas(&admin).len(), 1);
        assert_eq!(reg.get_schemas(&plain).len(), 0);
    }
}
