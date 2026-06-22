//! Shared application state injected into every handler.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use gauss_config::{AppConfig, Nl2SqlConfig};
use gauss_core::domain::Database;
use gauss_core::error::{CoreError, CoreResult};
use gauss_db::{ContentRecord, Store};
use gauss_drivers::Driver;
use gauss_engine::traits::LlmService;
use gauss_llm::{
    AnthropicLlmService, GeminiLlmService, MockLlmService, OllamaLlmService, OpenAiLlmService,
};
use gauss_mcp_gateway::McpGateway;
use gauss_nl2sql::{LlmNl2Sql, Nl2SqlPipeline};
use gauss_notebook::KernelGateway;
use uuid::Uuid;

use crate::cache::ResultCache;

/// A live, reused connection to a data source. Reconnects automatically when the
/// stored connection URI changes (a different `uri` invalidates the cache entry).
struct CachedDriver {
    uri: String,
    driver: Arc<dyn Driver>,
}

/// Caches one live [`Driver`] per data source so connection pools (sqlx) and
/// HTTP clients (REST drivers) are established once and **reused** across
/// requests, instead of opening a fresh pool on every query. Evicted when a
/// source is deleted or its connection URI changes.
#[derive(Default)]
pub struct ConnectionRegistry {
    conns: RwLock<HashMap<Uuid, CachedDriver>>,
}

impl ConnectionRegistry {
    /// Return a reused driver for `db`, connecting (and caching) on first use or
    /// when the connection URI has changed since the cached entry.
    pub async fn driver_for(&self, db: &Database) -> CoreResult<Arc<dyn Driver>> {
        let uri = db.connection_uri.clone().ok_or_else(|| {
            CoreError::InvalidQuery(format!(
                "data source `{}` has no connection configured",
                db.name
            ))
        })?;
        // Fast path: a cached, still-valid connection.
        if let Some(c) = self.conns.read().expect("conn lock").get(&db.id) {
            if c.uri == uri {
                return Ok(c.driver.clone());
            }
        }
        // Slow path: connect without holding the lock across the await.
        let driver: Arc<dyn Driver> = Arc::from(gauss_drivers::connect(db.kind, &uri).await?);
        // Re-check under the write lock: if another task connected the same uri
        // concurrently, converge on its cached handle and drop ours, so every
        // caller shares one pool (no overwrite churn / abandoned connections).
        let mut conns = self.conns.write().expect("conn lock");
        if let Some(existing) = conns.get(&db.id) {
            if existing.uri == uri {
                return Ok(existing.driver.clone());
            }
        }
        conns.insert(
            db.id,
            CachedDriver {
                uri,
                driver: driver.clone(),
            },
        );
        Ok(driver)
    }

    /// Drop the cached connection for a data source (on delete or reconfigure).
    pub fn evict(&self, id: Uuid) {
        self.conns.write().expect("conn lock").remove(&id);
    }

    /// Number of cached connections (for diagnostics/tests).
    pub fn len(&self) -> usize {
        self.conns.read().expect("conn lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Reserved content kind + fixed id under which the live AI settings are
/// persisted in the metadata store (so they survive restarts without a new
/// table or migration).
const AI_SETTINGS_KIND: &str = "app_setting";
const AI_SETTINGS_ID: Uuid = Uuid::from_u128(0x9a55_0000_0000_4000_8000_0000_0000_a101);

/// The live AI/NL2SQL state: the effective config plus the built pipeline.
/// Held behind an `RwLock` so settings can be edited at runtime and the
/// translation pipeline hot-swapped without restarting the server.
pub struct AiState {
    pub config: Nl2SqlConfig,
    pub pipeline: Option<Arc<Nl2SqlPipeline<LlmNl2Sql>>>,
}

/// Build the pipeline for a config, or `None` when NL2SQL is disabled.
fn build_pipeline(
    cfg: &Nl2SqlConfig,
) -> gauss_core::CoreResult<Option<Arc<Nl2SqlPipeline<LlmNl2Sql>>>> {
    if !cfg.enabled {
        return Ok(None);
    }
    let llm = build_nl2sql_llm(cfg)?;
    Ok(Some(Arc::new(Nl2SqlPipeline::new(LlmNl2Sql::new(llm)))))
}

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
    /// Live, runtime-editable AI/NL2SQL state (config + hot-swappable pipeline).
    pub ai: Arc<RwLock<AiState>>,
    /// Reused live connections to data sources (one pool/client per source).
    pub connections: Arc<ConnectionRegistry>,
    /// Notebook kernel gateway to the user's local Jupyter Server. Present only
    /// when `jupyter.enabled` is set; otherwise notebook execution endpoints
    /// report that the integration is disabled.
    pub notebook: Option<Arc<KernelGateway>>,
    /// Live kernel sessions: notebook id → its running Jupyter kernel id. A
    /// process-lifetime map (kernels live in the user's Jupyter, not here).
    pub kernels: Arc<RwLock<HashMap<Uuid, String>>>,
    /// Serializes kernel creation so concurrent runs of the same notebook don't
    /// start (and orphan) duplicate kernels or split variable state across them.
    pub kernel_lock: Arc<tokio::sync::Mutex<()>>,
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

        let pipeline = build_pipeline(&config.nl2sql)?;
        let ai = Arc::new(RwLock::new(AiState {
            config: config.nl2sql.clone(),
            pipeline,
        }));

        let cache = Arc::new(ResultCache::new(config.server.cache_ttl_secs));

        // Build the notebook kernel gateway only when the operator has opted in.
        // No connection is attempted here; it is lazy, on first kernel start.
        let notebook = if config.jupyter.enabled {
            Some(Arc::new(KernelGateway::new(
                config.jupyter.url.clone(),
                config.jupyter.token.clone(),
            )))
        } else {
            None
        };

        Ok(Self {
            config: Arc::new(config),
            store,
            cache,
            usage: Arc::new(UsageStats::default()),
            mcp,
            ai,
            connections: Arc::new(ConnectionRegistry::default()),
            notebook,
            kernels: Arc::new(RwLock::new(HashMap::new())),
            kernel_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// The notebook kernel gateway, or a clear error when the integration is
    /// disabled (so handlers can `?` it and surface a helpful message).
    pub fn kernel_gateway(&self) -> CoreResult<Arc<KernelGateway>> {
        self.notebook.clone().ok_or_else(|| {
            CoreError::NotFound(
                "notebook integration is not enabled (set GAUSS_JUPYTER_ENABLED=true and run a \
                 local Jupyter Server)"
                    .into(),
            )
        })
    }

    /// The kernel id currently bound to `notebook_id`, if one is running.
    pub fn notebook_kernel(&self, notebook_id: Uuid) -> Option<String> {
        self.kernels
            .read()
            .expect("kernels lock")
            .get(&notebook_id)
            .cloned()
    }

    /// Bind a started kernel id to a notebook.
    pub fn set_notebook_kernel(&self, notebook_id: Uuid, kernel_id: String) {
        self.kernels
            .write()
            .expect("kernels lock")
            .insert(notebook_id, kernel_id);
    }

    /// Unbind and return the kernel id for a notebook (on stop/delete).
    pub fn take_notebook_kernel(&self, notebook_id: Uuid) -> Option<String> {
        self.kernels
            .write()
            .expect("kernels lock")
            .remove(&notebook_id)
    }

    /// The current NL2SQL pipeline (cloned out of the lock), if enabled.
    pub fn nl2sql_pipeline(&self) -> Option<Arc<Nl2SqlPipeline<LlmNl2Sql>>> {
        self.ai.read().expect("ai lock poisoned").pipeline.clone()
    }

    /// A snapshot of the effective AI configuration.
    pub fn ai_config(&self) -> Nl2SqlConfig {
        self.ai.read().expect("ai lock poisoned").config.clone()
    }

    /// Apply a new AI configuration at runtime: validate + build the pipeline,
    /// persist the settings, then hot-swap them in. No restart required.
    pub async fn update_ai(&self, cfg: Nl2SqlConfig) -> gauss_core::CoreResult<()> {
        // Build first so an invalid config is rejected before we persist or swap.
        let pipeline = build_pipeline(&cfg)?;
        let body = serde_json::to_string(&cfg).map_err(|e| CoreError::Internal(e.to_string()))?;
        self.store
            .put_content(ContentRecord {
                id: AI_SETTINGS_ID,
                kind: AI_SETTINGS_KIND.into(),
                collection_id: None,
                name: "ai".into(),
                body_json: body,
                created_at: chrono::Utc::now(),
            })
            .await?;
        let mut w = self.ai.write().expect("ai lock poisoned");
        w.config = cfg;
        w.pipeline = pipeline;
        Ok(())
    }

    /// Load persisted AI settings (if any) and hot-swap them in. Called at
    /// startup so runtime edits survive a restart.
    pub async fn reload_ai_from_store(&self) -> gauss_core::CoreResult<()> {
        let Some(rec) = self.store.get_content(AI_SETTINGS_ID).await? else {
            return Ok(());
        };
        let cfg: Nl2SqlConfig =
            serde_json::from_str(&rec.body_json).map_err(|e| CoreError::Storage(e.to_string()))?;
        let pipeline = build_pipeline(&cfg)?;
        let mut w = self.ai.write().expect("ai lock poisoned");
        w.config = cfg;
        w.pipeline = pipeline;
        Ok(())
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
