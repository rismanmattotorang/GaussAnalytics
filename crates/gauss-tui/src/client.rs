//! Blocking HTTP client the TUI uses to read live data from `gauss-server`.
//!
//! The console is a first-class API client: it reads exactly the same
//! endpoints the web UI does. Admin-only views (users) require a bearer token,
//! supplied via `GAUSS_API_TOKEN`.

use serde::de::DeserializeOwned;
use serde::Deserialize;

/// Health summary from `GET /api/health`.
#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    pub status: String,
    pub version: String,
}

/// A data source row from `GET /api/databases`.
#[derive(Debug, Clone, Deserialize)]
pub struct DbRow {
    pub name: String,
    pub kind: String,
    pub is_synced: bool,
}

/// A user row from `GET /api/users` (admin).
#[derive(Debug, Clone, Deserialize)]
pub struct UserRow {
    pub email: String,
    pub display_name: String,
    pub is_admin: bool,
}

/// A blocking client bound to a server base URL and optional admin token.
pub struct ApiClient {
    base: String,
    token: Option<String>,
    http: reqwest::blocking::Client,
}

impl ApiClient {
    /// Build from the environment: `GAUSS_API_URL` (default
    /// `http://127.0.0.1:3000`) and optional `GAUSS_API_TOKEN`.
    pub fn from_env() -> Self {
        let base =
            std::env::var("GAUSS_API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
        let token = std::env::var("GAUSS_API_TOKEN")
            .ok()
            .filter(|t| !t.is_empty());
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");
        Self {
            base: base.trim_end_matches('/').to_string(),
            token,
            http,
        }
    }

    /// Whether an admin token is configured (gates the Users view).
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let mut req = self.http.get(format!("{}{}", self.base, path));
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().map_err(|e| e.to_string())?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("HTTP {}", status.as_u16()));
        }
        resp.json::<T>().map_err(|e| e.to_string())
    }

    pub fn health(&self) -> Result<Health, String> {
        self.get("/api/health")
    }

    pub fn databases(&self) -> Result<Vec<DbRow>, String> {
        self.get("/api/databases")
    }

    pub fn users(&self) -> Result<Vec<UserRow>, String> {
        self.get("/api/users")
    }
}
