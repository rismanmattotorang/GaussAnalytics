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

/// The LLM providers this build can drive for NL2SQL. OpenRouter, LiteLLM, and
/// vLLM are OpenAI-compatible and reuse the OpenAI client with a base URL.
/// `bedrock` is recognized but is reached via an OpenAI-compatible gateway
/// (e.g. LiteLLM) rather than bundling the AWS SDK.
pub const NL2SQL_PROVIDERS: &[&str] = &[
    "mock",
    "openai",
    "anthropic",
    "ollama",
    "gemini",
    "openrouter",
    "litellm",
    "vllm",
    "bedrock",
];

/// Default OpenAI-compatible base URL for OpenRouter.
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Build an OpenAI-compatible client (OpenAI, OpenRouter, LiteLLM, vLLM). All
/// speak the Chat Completions wire format; they differ only in base URL.
fn openai_compatible(
    cfg: &Nl2SqlConfig,
    default_base: Option<&str>,
    base_required: bool,
    provider: &str,
) -> gauss_core::CoreResult<Arc<dyn LlmService>> {
    let base = if !cfg.base_url.is_empty() {
        Some(cfg.base_url.clone())
    } else {
        default_base.map(str::to_string)
    };
    if base_required && base.is_none() {
        return Err(CoreError::Config(format!(
            "NL2SQL provider {provider:?} requires GAUSS_NL2SQL_BASE_URL (its OpenAI-compatible endpoint)"
        )));
    }
    let mut svc = OpenAiLlmService::new(cfg.api_key.clone(), cfg.model.clone());
    if let Some(b) = base {
        svc = svc.with_base_url(b);
    }
    Ok(Arc::new(svc))
}

/// Build the in-process LLM backend for NL2SQL from configuration.
///
/// Replaces the former external, credentialed NL2SQL service: GaussAnalytics
/// drives a configured LLM provider directly, in-process.
fn build_nl2sql_llm(cfg: &Nl2SqlConfig) -> gauss_core::CoreResult<Arc<dyn LlmService>> {
    let model = cfg.model.clone();
    let llm: Arc<dyn LlmService> = match cfg.provider.to_ascii_lowercase().as_str() {
        "mock" | "" => Arc::new(MockLlmService::new()),
        "openai" => openai_compatible(cfg, None, false, "openai")?,
        // OpenRouter and LiteLLM are OpenAI-compatible gateways. OpenRouter has
        // a well-known endpoint; LiteLLM is self-hosted, so its URL is required.
        "openrouter" => openai_compatible(cfg, Some(OPENROUTER_BASE_URL), false, "openrouter")?,
        "litellm" => openai_compatible(cfg, None, true, "litellm")?,
        // vLLM exposes an OpenAI-compatible server; its base URL is required.
        "vllm" => openai_compatible(cfg, None, true, "vllm")?,
        // AWS Bedrock is reached through an OpenAI-compatible gateway (LiteLLM or
        // bedrock-access-gateway) rather than bundling the AWS SDK.
        "bedrock" => {
            return Err(CoreError::Config(
                "provider \"bedrock\": point GaussAnalytics at a Bedrock OpenAI-compatible \
                 gateway — set provider \"litellm\" (or \"openai\") and GAUSS_NL2SQL_BASE_URL \
                 to the gateway URL"
                    .into(),
            ))
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
                "unknown NL2SQL provider {other:?} (expected one of: {})",
                NL2SQL_PROVIDERS.join(", ")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(provider: &str, base_url: &str) -> Nl2SqlConfig {
        Nl2SqlConfig {
            enabled: true,
            provider: provider.into(),
            model: "m".into(),
            api_key: "k".into(),
            base_url: base_url.into(),
            timeout_ms: 30_000,
        }
    }

    #[test]
    fn openai_compatible_providers_build() {
        // mock + the OpenAI-compatible providers resolve to a client.
        assert!(build_nl2sql_llm(&cfg("mock", "")).is_ok());
        assert!(build_nl2sql_llm(&cfg("openai", "")).is_ok());
        // OpenRouter has a default endpoint, so no base URL is required.
        assert!(build_nl2sql_llm(&cfg("openrouter", "")).is_ok());
        // vLLM / LiteLLM are self-hosted: base URL required.
        assert!(build_nl2sql_llm(&cfg("vllm", "http://localhost:8000/v1")).is_ok());
        assert!(build_nl2sql_llm(&cfg("vllm", "")).is_err());
        assert!(build_nl2sql_llm(&cfg("litellm", "http://localhost:4000")).is_ok());
        assert!(build_nl2sql_llm(&cfg("litellm", "")).is_err());
    }

    #[test]
    fn bedrock_directs_to_gateway_and_unknown_errors() {
        let err = match build_nl2sql_llm(&cfg("bedrock", "")) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("bedrock should not build a direct client"),
        };
        assert!(err.contains("gateway"), "{err}");
        assert!(build_nl2sql_llm(&cfg("does-not-exist", "")).is_err());
    }
}
