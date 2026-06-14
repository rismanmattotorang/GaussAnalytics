//! `gauss-nl2sql` — integration layer to Gaussian's NL2SQL service.
//!
//! Gaussian Technologies owns the NL2SQL model; this crate owns the
//! **grounding** (assembling schema context) and the **guardrails** (validating
//! and constraining the returned query). The result is a `GuardedQuery` the
//! server can execute under the requesting user's permissions.

#![forbid(unsafe_code)]

pub mod guard;

use async_trait::async_trait;
use gauss_core::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

pub use guard::ensure_read_only;

/// Schema grounding for one table, sent to the model to reduce hallucination.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableContext {
    pub name: String,
    /// `(column_name, column_type)` pairs.
    pub columns: Vec<(String, String)>,
}

/// The grounded schema context for a translation request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaContext {
    pub database: String,
    pub tables: Vec<TableContext>,
}

/// One prior exchange in a multi-turn NL2SQL conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Turn {
    pub prompt: String,
    pub sql: String,
}

/// A natural-language translation request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Nl2SqlRequest {
    pub prompt: String,
    pub context: SchemaContext,
    /// Prior turns, enabling multi-turn refinement and clarifying follow-ups.
    #[serde(default)]
    pub history: Vec<Turn>,
}

/// The raw candidate returned by the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Nl2SqlCandidate {
    pub sql: String,
    #[serde(default)]
    pub explanation: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
}

/// A candidate that has passed GaussAnalytics' guardrails and is safe to run
/// (still subject to the requesting user's permissions at execution time).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GuardedQuery {
    pub sql: String,
    pub explanation: Option<String>,
    pub confidence: Option<f32>,
}

/// The capability surface of the NL2SQL integration.
#[async_trait]
pub trait Nl2Sql: Send + Sync {
    async fn translate(&self, request: &Nl2SqlRequest) -> CoreResult<Nl2SqlCandidate>;
}

/// HTTP-backed client for Gaussian's NL2SQL service.
pub struct HttpNl2Sql {
    client: reqwest::Client,
    base_url: String,
}

impl HttpNl2Sql {
    pub fn new(base_url: impl Into<String>, timeout_ms: u64) -> CoreResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .map_err(|e| CoreError::Integration(format!("nl2sql client init failed: {e}")))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }
}

#[async_trait]
impl Nl2Sql for HttpNl2Sql {
    async fn translate(&self, request: &Nl2SqlRequest) -> CoreResult<Nl2SqlCandidate> {
        let url = format!("{}/translate", self.base_url);
        let resp = self
            .client
            .post(url)
            .json(request)
            .send()
            .await
            .map_err(|e| CoreError::Integration(e.to_string()))?;
        resp.error_for_status()
            .map_err(|e| CoreError::Integration(e.to_string()))?
            .json::<Nl2SqlCandidate>()
            .await
            .map_err(|e| CoreError::Integration(e.to_string()))
    }
}

/// The NL→guarded-SQL pipeline: translate, then enforce read-only guardrails.
///
/// Permission enforcement and execution are the server's responsibility, since
/// they require the request's identity and the metadata store.
pub struct Nl2SqlPipeline<C: Nl2Sql> {
    client: C,
}

impl<C: Nl2Sql> Nl2SqlPipeline<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }

    /// Translate `request` and return a guardrail-checked query.
    pub async fn propose(&self, request: &Nl2SqlRequest) -> CoreResult<GuardedQuery> {
        let candidate = self.client.translate(request).await?;
        let sql = ensure_read_only(&candidate.sql)?;
        Ok(GuardedQuery {
            sql,
            explanation: candidate.explanation,
            confidence: candidate.confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubClient {
        sql: String,
    }

    #[async_trait]
    impl Nl2Sql for StubClient {
        async fn translate(&self, _r: &Nl2SqlRequest) -> CoreResult<Nl2SqlCandidate> {
            Ok(Nl2SqlCandidate {
                sql: self.sql.clone(),
                explanation: Some("stub".into()),
                confidence: Some(0.9),
            })
        }
    }

    fn req() -> Nl2SqlRequest {
        Nl2SqlRequest {
            prompt: "total revenue by status".into(),
            context: SchemaContext {
                database: "warehouse".into(),
                tables: vec![TableContext {
                    name: "orders".into(),
                    columns: vec![("total".into(), "float".into())],
                }],
            },
            history: vec![],
        }
    }

    #[tokio::test]
    async fn pipeline_passes_safe_select() {
        let pipe = Nl2SqlPipeline::new(StubClient {
            sql: "SELECT status, SUM(total) FROM orders GROUP BY status".into(),
        });
        let guarded = pipe.propose(&req()).await.unwrap();
        assert!(guarded.sql.starts_with("SELECT"));
        assert_eq!(guarded.confidence, Some(0.9));
    }

    #[tokio::test]
    async fn pipeline_blocks_unsafe_candidate() {
        let pipe = Nl2SqlPipeline::new(StubClient {
            sql: "DROP TABLE orders".into(),
        });
        assert!(pipe.propose(&req()).await.is_err());
    }
}
