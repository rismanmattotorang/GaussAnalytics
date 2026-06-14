//! Tool-invocation policy.
//!
//! GaussAnalytics governs which MCP tools may be called, independent of what
//! the upstream servers expose. The default posture is deny-by-default once any
//! allow-list entry exists, so enabling one server does not implicitly enable
//! the rest.

use std::collections::HashSet;

use gauss_core::error::{CoreError, CoreResult};

use crate::ToolInvocation;

/// An allow-list of `server` and `server:tool` identifiers.
#[derive(Debug, Clone, Default)]
pub struct ToolPolicy {
    /// Allowed server names.
    servers: HashSet<String>,
    /// Allowed fully-qualified `server:tool` names.
    tools: HashSet<String>,
    /// If true, allow everything (development convenience; not for production).
    allow_all: bool,
}

impl ToolPolicy {
    /// A policy that allows everything. Intended for local development only.
    pub fn allow_all() -> Self {
        Self {
            allow_all: true,
            ..Default::default()
        }
    }

    /// A deny-by-default policy; grant access with [`Self::allow_server`] /
    /// [`Self::allow_tool`].
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// Allow every tool on `server`.
    pub fn allow_server(mut self, server: impl Into<String>) -> Self {
        self.servers.insert(server.into());
        self
    }

    /// Allow a specific `tool` on `server`.
    pub fn allow_tool(mut self, server: impl Into<String>, tool: impl Into<String>) -> Self {
        self.tools
            .insert(format!("{}:{}", server.into(), tool.into()));
        self
    }

    /// Authorize an invocation, or return [`CoreError::PermissionDenied`].
    pub fn enforce(&self, inv: &ToolInvocation) -> CoreResult<()> {
        if self.allow_all
            || self.servers.contains(&inv.server)
            || self.tools.contains(&format!("{}:{}", inv.server, inv.tool))
        {
            Ok(())
        } else {
            Err(CoreError::PermissionDenied(format!(
                "MCP tool not allowed by policy: {}:{}",
                inv.server, inv.tool
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv(server: &str, tool: &str) -> ToolInvocation {
        ToolInvocation {
            server: server.into(),
            tool: tool.into(),
            arguments: serde_json::Value::Null,
        }
    }

    #[test]
    fn deny_by_default() {
        let p = ToolPolicy::deny_all();
        assert!(p.enforce(&inv("fs", "read")).is_err());
    }

    #[test]
    fn server_grant_allows_all_its_tools() {
        let p = ToolPolicy::deny_all().allow_server("warehouse");
        assert!(p.enforce(&inv("warehouse", "query")).is_ok());
        assert!(p.enforce(&inv("fs", "read")).is_err());
    }

    #[test]
    fn tool_grant_is_specific() {
        let p = ToolPolicy::deny_all().allow_tool("fs", "read");
        assert!(p.enforce(&inv("fs", "read")).is_ok());
        assert!(p.enforce(&inv("fs", "write")).is_err());
    }
}
