//! Shared application state injected into every handler.

use std::sync::Arc;

use gauss_config::{AppConfig, Nl2SqlConfig};
use gauss_core::error::CoreError;
use gauss_db::Store;
use gauss_engine::traits::LlmService;
use gauss_llm::{
    AnthropicLlmService, GeminiLlmService, MockLlmService, OllamaLlmService, OpenAiLlmService,
};
use gauss_mcp_gateway::McpGateway;
use gauss_nl2sql::{LlmNl2Sql, Nl2SqlPipeline};

use crate::cache::ResultCache;

/// Lightweight usage analytics: how many queries the instance has executed.
#[derive(Default)]
pub struct UsageStats {
    queries: std::sync::atomic::AtomicU64,
}

impl UsageStats {
    pub fn record_query(&self) {
        self.queries
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn queries_run(&self) -> u64 {
        self.queries.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Cloneable handle to all shared services. `Clone` is cheap — everything is an
/// `Arc` or `Option<Arc<_>>`.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub store: Arc<dyn Store>,
    /// Query-result cache (no-op when the configured TTL is zero).
    pub cache: Arc<ResultCache>,
    /// Process-lifetime usage analytics.
    pub usage: Arc<UsageStats>,
    /// Present only when the MCP integration is enabled in config.
    pub mcp: Option<Arc<dyn McpGateway>>,
    /// Present only when the NL2SQL integration is enabled in config.
    pub nl2sql: Option<Arc<Nl2SqlPipeline<LlmNl2Sql>>>,
}

/// Build the in-process LLM backend for NL2SQL from configuration.
///
/// Replaces the former external, credentialed NL2SQL service: GaussAnalytics
/// drives a configured LLM provider directly, in-process.
fn build_nl2sql_llm(cfg: &Nl2SqlConfig) -> gauss_core::CoreResult<Arc<dyn LlmService>> {
    let model = cfg.model.clone();
    let llm: Arc<dyn LlmService> = match cfg.provider.to_ascii_lowercase().as_str() {
        "mock" | "" => Arc::new(MockLlmService::new()),
        "openai" => {
            let mut svc = OpenAiLlmService::new(cfg.api_key.clone(), model);
            if !cfg.base_url.is_empty() {
                svc = svc.with_base_url(cfg.base_url.clone());
            }
            Arc::new(svc)
        }
        "anthropic" => {
            let mut svc = AnthropicLlmService::new(cfg.api_key.clone(), model);
            if !cfg.base_url.is_empty() {
                svc = svc.with_base_url(cfg.base_url.clone());
            }
            Arc::new(svc)
        }
        "ollama" => {
            let mut svc = OllamaLlmService::new(model);
            if !cfg.base_url.is_empty() {
                svc = svc.with_base_url(cfg.base_url.clone());
            }
            Arc::new(svc)
        }
        "gemini" => {
            let mut svc = GeminiLlmService::new(cfg.api_key.clone(), model);
            if !cfg.base_url.is_empty() {
                svc = svc.with_base_url(cfg.base_url.clone());
            }
            Arc::new(svc)
        }
        other => {
            return Err(CoreError::Config(format!(
                "unknown NL2SQL provider {other:?} (expected one of: mock, openai, anthropic, ollama, gemini)"
            )))
        }
    };
    Ok(llm)
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
            let llm = build_nl2sql_llm(&config.nl2sql)?;
            Some(Arc::new(Nl2SqlPipeline::new(LlmNl2Sql::new(llm))))
        } else {
            None
        };

        let cache = Arc::new(ResultCache::new(config.server.cache_ttl_secs));

        Ok(Self {
            config: Arc::new(config),
            store,
            cache,
            usage: Arc::new(UsageStats::default()),
            mcp,
            nl2sql,
        })
    }
}
