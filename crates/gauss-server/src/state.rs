//! Shared application state injected into every handler.

use std::sync::Arc;

use gauss_config::AppConfig;
use gauss_db::Store;
use gauss_mcp_gateway::McpGateway;
use gauss_nl2sql::{HttpNl2Sql, Nl2SqlPipeline};

/// Cloneable handle to all shared services. `Clone` is cheap — everything is an
/// `Arc` or `Option<Arc<_>>`.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub store: Arc<dyn Store>,
    /// Present only when the MCP integration is enabled in config.
    pub mcp: Option<Arc<dyn McpGateway>>,
    /// Present only when the NL2SQL integration is enabled in config.
    pub nl2sql: Option<Arc<Nl2SqlPipeline<HttpNl2Sql>>>,
}

impl AppState {
    /// Build state from config, wiring integrations that are enabled.
    pub fn new(config: AppConfig, store: Arc<dyn Store>) -> gauss_core::CoreResult<Self> {
        let mcp: Option<Arc<dyn McpGateway>> = if config.mcp.enabled {
            let gateway = gauss_mcp_gateway::HttpMcpGateway::new(
                config.mcp.base_url.clone(),
                config.mcp.timeout_ms,
                gauss_mcp_gateway::ToolPolicy::deny_all(),
                Box::new(gauss_mcp_gateway::NoopAuditSink),
            )?;
            Some(Arc::new(gateway))
        } else {
            None
        };

        let nl2sql = if config.nl2sql.enabled {
            let client = HttpNl2Sql::new(config.nl2sql.base_url.clone(), config.nl2sql.timeout_ms)?;
            Some(Arc::new(Nl2SqlPipeline::new(client)))
        } else {
            None
        };

        Ok(Self {
            config: Arc::new(config),
            store,
            mcp,
            nl2sql,
        })
    }
}
