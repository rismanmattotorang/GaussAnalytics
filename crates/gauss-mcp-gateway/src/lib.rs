//! `gauss-mcp-gateway` — integration layer to Gaussian's MCP Servers.
//!
//! Gaussian Technologies operates the MCP Servers; this crate does **not**
//! reimplement them. It provides a typed client plus the GaussAnalytics-owned
//! governance around tool discovery and invocation: a [`policy::ToolPolicy`]
//! allow-list and an [`audit::AuditSink`] hook. The server composes these so
//! that every agentic tool call is policy-checked and recorded.

#![forbid(unsafe_code)]

pub mod audit;
pub mod policy;

use async_trait::async_trait;
use gauss_core::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

pub use audit::{AuditEvent, AuditSink, NoopAuditSink};
pub use policy::ToolPolicy;

/// A registered MCP server exposed by Gaussian's control plane.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// A tool offered by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// A request to invoke a tool on a server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolInvocation {
    pub server: String,
    pub tool: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// The result of a tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub output: serde_json::Value,
}

/// The capability surface of the MCP gateway.
#[async_trait]
pub trait McpGateway: Send + Sync {
    /// Discover available MCP servers.
    async fn list_servers(&self) -> CoreResult<Vec<McpServer>>;
    /// Discover tools offered by `server`.
    async fn list_tools(&self, server: &str) -> CoreResult<Vec<McpTool>>;
    /// Invoke a tool (already policy-checked by the caller or by this impl).
    async fn invoke(&self, invocation: ToolInvocation) -> CoreResult<ToolResult>;
}

/// HTTP-backed gateway that talks to Gaussian's MCP control plane, applying a
/// [`ToolPolicy`] and recording to an [`AuditSink`] around each invocation.
pub struct HttpMcpGateway {
    client: reqwest::Client,
    base_url: String,
    policy: ToolPolicy,
    audit: Box<dyn AuditSink>,
}

impl HttpMcpGateway {
    /// Construct a gateway pointed at `base_url` with the given request timeout.
    pub fn new(
        base_url: impl Into<String>,
        timeout_ms: u64,
        policy: ToolPolicy,
        audit: Box<dyn AuditSink>,
    ) -> CoreResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .map_err(|e| CoreError::Integration(format!("mcp client init failed: {e}")))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            policy,
            audit,
        })
    }
}

fn integ<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Integration(e.to_string())
}

#[async_trait]
impl McpGateway for HttpMcpGateway {
    async fn list_servers(&self) -> CoreResult<Vec<McpServer>> {
        let url = format!("{}/servers", self.base_url);
        let resp = self.client.get(url).send().await.map_err(integ)?;
        resp.error_for_status()
            .map_err(integ)?
            .json::<Vec<McpServer>>()
            .await
            .map_err(integ)
    }

    async fn list_tools(&self, server: &str) -> CoreResult<Vec<McpTool>> {
        let url = format!("{}/servers/{}/tools", self.base_url, server);
        let resp = self.client.get(url).send().await.map_err(integ)?;
        resp.error_for_status()
            .map_err(integ)?
            .json::<Vec<McpTool>>()
            .await
            .map_err(integ)
    }

    async fn invoke(&self, invocation: ToolInvocation) -> CoreResult<ToolResult> {
        // Governance first: refuse anything the policy does not allow.
        self.policy.enforce(&invocation)?;
        self.audit.record(AuditEvent::tool_requested(&invocation));

        let url = format!("{}/invoke", self.base_url);
        let outcome = async {
            let resp = self
                .client
                .post(url)
                .json(&invocation)
                .send()
                .await
                .map_err(integ)?;
            resp.error_for_status()
                .map_err(integ)?
                .json::<ToolResult>()
                .await
                .map_err(integ)
        }
        .await;

        self.audit
            .record(AuditEvent::tool_completed(&invocation, outcome.is_ok()));
        outcome
    }
}
