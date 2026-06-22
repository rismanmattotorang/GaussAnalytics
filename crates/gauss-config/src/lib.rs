//! `gauss-config` — layered, typed configuration.
//!
//! Configuration is resolved as: built-in defaults, then overrides from the
//! process environment (variables prefixed `GAUSS_`). A future revision adds a
//! file layer between the two; the precedence is intentionally simple and
//! explicit so operators can reason about where a value came from.

#![forbid(unsafe_code)]

use gauss_core::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub security: SecurityConfig,
    pub mcp: McpConfig,
    pub nl2sql: Nl2SqlConfig,
}

/// HTTP server settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Filesystem path to the built frontend assets to serve.
    pub static_dir: String,
    /// Query-result cache TTL in seconds. `0` disables caching.
    #[serde(default)]
    pub cache_ttl_secs: u64,
    /// Background scheduler tick period in seconds (refresh/alerts).
    #[serde(default)]
    pub scheduler_period_secs: u64,
}

/// Application metadata database (the platform's own store).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseConfig {
    /// Connection URL for the app DB. Phase 1 uses an in-memory store and
    /// ignores this; Phase 2 connects via `sqlx`.
    pub url: String,
}

/// Security-related settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityConfig {
    /// Lifetime of an issued session, in seconds.
    pub session_ttl_secs: u64,
    /// When true, all API routes except a small public set require a valid
    /// principal (session or API key). Default false for local development.
    #[serde(default)]
    pub require_auth: bool,
    /// Static service API keys (compared in constant time). Configured via
    /// `GAUSS_API_KEYS` (comma-separated). A request bearing a matching key
    /// authenticates as a service administrator.
    #[serde(default)]
    pub api_keys: Vec<String>,
    /// HMAC secret for signed embedding tokens. Empty disables embedding.
    #[serde(default)]
    pub embedding_secret: String,
}

/// Integration settings for Gaussian's MCP Servers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpConfig {
    /// Whether the MCP gateway integration is enabled.
    pub enabled: bool,
    /// Base URL of the Gaussian MCP control plane.
    pub base_url: String,
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,
}

/// Settings for the in-house NL2SQL engine.
///
/// Translation runs in-process against a configured LLM provider; there is no
/// external NL2SQL service and therefore no service credential. `api_key`, when
/// required, is the LLM provider's own key (read from the environment).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Nl2SqlConfig {
    /// Whether the NL2SQL integration is enabled.
    pub enabled: bool,
    /// LLM provider that performs translation: `mock`, `openai`, `anthropic`,
    /// `ollama`, or `gemini`.
    pub provider: String,
    /// Model identifier passed to the provider.
    pub model: String,
    /// Provider API key. Empty for providers that need none (`mock`, `ollama`).
    pub api_key: String,
    /// Optional base-URL override for self-hosted / OpenAI-compatible / Ollama
    /// endpoints. Empty uses the provider default.
    pub base_url: String,
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".into(),
                port: 3000,
                static_dir: "frontend/dist".into(),
                cache_ttl_secs: 0,
                scheduler_period_secs: 60,
            },
            database: DatabaseConfig {
                url: "sqlite://data/gauss.db".into(),
            },
            security: SecurityConfig {
                session_ttl_secs: 60 * 60 * 24 * 14, // 14 days
                require_auth: false,
                api_keys: Vec::new(),
                embedding_secret: String::new(),
            },
            mcp: McpConfig {
                enabled: false,
                base_url: "http://localhost:8848".into(),
                timeout_ms: 30_000,
            },
            nl2sql: Nl2SqlConfig {
                enabled: false,
                provider: "mock".into(),
                model: String::new(),
                api_key: String::new(),
                base_url: String::new(),
                timeout_ms: 30_000,
            },
        }
    }
}

impl AppConfig {
    /// Load configuration: defaults overlaid with `GAUSS_`-prefixed env vars.
    ///
    /// Recognized variables:
    /// `GAUSS_HOST`, `GAUSS_PORT`, `GAUSS_STATIC_DIR`, `GAUSS_DATABASE_URL`,
    /// `GAUSS_SESSION_TTL_SECS`, `GAUSS_MCP_ENABLED`, `GAUSS_MCP_BASE_URL`,
    /// `GAUSS_MCP_TIMEOUT_MS`, `GAUSS_NL2SQL_ENABLED`, `GAUSS_NL2SQL_PROVIDER`,
    /// `GAUSS_NL2SQL_MODEL`, `GAUSS_NL2SQL_API_KEY`, `GAUSS_NL2SQL_BASE_URL`,
    /// `GAUSS_NL2SQL_TIMEOUT_MS`.
    pub fn from_env() -> CoreResult<Self> {
        let get = |k: &str| std::env::var(k).ok();
        Self::from_lookup(&get)
    }

    /// Apply overrides from an arbitrary lookup function (testable seam).
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> CoreResult<Self> {
        let mut cfg = AppConfig::default();

        if let Some(v) = get("GAUSS_HOST") {
            cfg.server.host = v;
        }
        if let Some(v) = get("GAUSS_PORT") {
            cfg.server.port = parse(&v, "GAUSS_PORT")?;
        }
        if let Some(v) = get("GAUSS_STATIC_DIR") {
            cfg.server.static_dir = v;
        }
        if let Some(v) = get("GAUSS_DATABASE_URL") {
            cfg.database.url = v;
        }
        if let Some(v) = get("GAUSS_SESSION_TTL_SECS") {
            cfg.security.session_ttl_secs = parse(&v, "GAUSS_SESSION_TTL_SECS")?;
        }
        if let Some(v) = get("GAUSS_REQUIRE_AUTH") {
            cfg.security.require_auth = parse_bool(&v, "GAUSS_REQUIRE_AUTH")?;
        }
        if let Some(v) = get("GAUSS_API_KEYS") {
            cfg.security.api_keys = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(v) = get("GAUSS_EMBEDDING_SECRET") {
            cfg.security.embedding_secret = v;
        }
        if let Some(v) = get("GAUSS_CACHE_TTL_SECS") {
            cfg.server.cache_ttl_secs = parse(&v, "GAUSS_CACHE_TTL_SECS")?;
        }
        if let Some(v) = get("GAUSS_SCHEDULER_PERIOD_SECS") {
            cfg.server.scheduler_period_secs = parse(&v, "GAUSS_SCHEDULER_PERIOD_SECS")?;
        }
        if let Some(v) = get("GAUSS_MCP_ENABLED") {
            cfg.mcp.enabled = parse_bool(&v, "GAUSS_MCP_ENABLED")?;
        }
        if let Some(v) = get("GAUSS_MCP_BASE_URL") {
            cfg.mcp.base_url = v;
        }
        if let Some(v) = get("GAUSS_MCP_TIMEOUT_MS") {
            cfg.mcp.timeout_ms = parse(&v, "GAUSS_MCP_TIMEOUT_MS")?;
        }
        if let Some(v) = get("GAUSS_NL2SQL_ENABLED") {
            cfg.nl2sql.enabled = parse_bool(&v, "GAUSS_NL2SQL_ENABLED")?;
        }
        if let Some(v) = get("GAUSS_NL2SQL_PROVIDER") {
            cfg.nl2sql.provider = v;
        }
        if let Some(v) = get("GAUSS_NL2SQL_MODEL") {
            cfg.nl2sql.model = v;
        }
        if let Some(v) = get("GAUSS_NL2SQL_API_KEY") {
            cfg.nl2sql.api_key = v;
        }
        if let Some(v) = get("GAUSS_NL2SQL_BASE_URL") {
            cfg.nl2sql.base_url = v;
        }
        if let Some(v) = get("GAUSS_NL2SQL_TIMEOUT_MS") {
            cfg.nl2sql.timeout_ms = parse(&v, "GAUSS_NL2SQL_TIMEOUT_MS")?;
        }

        Ok(cfg)
    }

    /// The `host:port` socket address string for the server to bind.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }
}

fn parse<T: std::str::FromStr>(v: &str, key: &str) -> CoreResult<T> {
    v.parse::<T>()
        .map_err(|_| CoreError::Config(format!("invalid value for {key}: {v:?}")))
}

fn parse_bool(v: &str, key: &str) -> CoreResult<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(CoreError::Config(format!(
            "invalid boolean for {key}: {v:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn defaults_are_sane() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.bind_addr(), "127.0.0.1:3000");
        assert!(!cfg.mcp.enabled);
    }

    #[test]
    fn env_overrides_apply() {
        let mut env = HashMap::new();
        env.insert("GAUSS_PORT".to_string(), "8080".to_string());
        env.insert("GAUSS_MCP_ENABLED".to_string(), "true".to_string());
        let get = |k: &str| env.get(k).cloned();
        let cfg = AppConfig::from_lookup(&get).unwrap();
        assert_eq!(cfg.server.port, 8080);
        assert!(cfg.mcp.enabled);
    }

    #[test]
    fn bad_port_is_an_error() {
        let get = |k: &str| (k == "GAUSS_PORT").then(|| "not-a-port".to_string());
        assert!(AppConfig::from_lookup(&get).is_err());
    }
}
