//! `gauss-nl2sql` — in-house NL2SQL integration.
//!
//! Earlier revisions delegated translation to an external, credentialed
//! "NL2SQL service" over HTTP. GaussAnalytics now owns the full text-to-SQL
//! stack in-process (`gauss-engine`, `gauss-llm`, `gauss-semantic`,
//! `gauss-sqlguard`, `gauss-textsql`), so there is no outbound service call and
//! no service credential to manage — the platform drives a configured LLM
//! provider directly.
//!
//! This crate owns the **grounding** (assembling schema context), drives an
//! in-process [`LlmService`] to **translate** the question, and applies
//! read-only **guardrails** to the result. The output is a `GuardedQuery` the
//! server can execute under the requesting user's permissions.

#![forbid(unsafe_code)]

pub mod guard;

use std::sync::Arc;

use async_trait::async_trait;
use gauss_core::error::{CoreError, CoreResult};
use gauss_engine::model::llm::{LlmMessage, LlmRequest};
use gauss_engine::model::user::User;
use gauss_engine::traits::LlmService;
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

/// In-house NL2SQL translator backed by an in-process [`LlmService`].
///
/// This replaces the previous external, credentialed HTTP service: the schema
/// context is rendered into a prompt and translated by a configured LLM
/// provider (Anthropic, OpenAI, Ollama, Gemini, or the deterministic mock) from
/// `gauss-llm`. The returned SQL still passes through [`ensure_read_only`] in
/// the pipeline before it can run.
pub struct LlmNl2Sql {
    llm: Arc<dyn LlmService>,
}

impl LlmNl2Sql {
    pub fn new(llm: Arc<dyn LlmService>) -> Self {
        Self { llm }
    }

    /// Render the grounded schema context into a compact prompt block.
    fn render_schema(context: &SchemaContext) -> String {
        let mut s = format!("## Data model\nDatabase: {}\n\nTables:\n", context.database);
        for table in &context.tables {
            let cols = table
                .columns
                .iter()
                .map(|(name, ty)| format!("{name} {ty}"))
                .collect::<Vec<_>>()
                .join(", ");
            s.push_str(&format!("- {} ({cols})\n", table.name));
        }
        s
    }
}

#[async_trait]
impl Nl2Sql for LlmNl2Sql {
    async fn translate(&self, request: &Nl2SqlRequest) -> CoreResult<Nl2SqlCandidate> {
        let system = format!(
            "You are an expert data analyst for GaussAnalytics. Write a single, correct, \
             read-only SQL SELECT query that answers the user's question using ONLY the \
             tables and columns in the data model below. Return ONLY the SQL inside a \
             ```sql code block.\n\n{}",
            Self::render_schema(&request.context)
        );

        // Replay prior turns so the model can refine multi-turn conversations.
        let mut messages = Vec::with_capacity(request.history.len() * 2 + 1);
        for turn in &request.history {
            messages.push(LlmMessage::new("user", turn.prompt.clone()));
            messages.push(LlmMessage::new(
                "assistant",
                format!("```sql\n{}\n```", turn.sql),
            ));
        }
        messages.push(LlmMessage::new("user", request.prompt.clone()));

        let req = LlmRequest {
            messages,
            tools: None,
            user: User::new("nl2sql"),
            stream: false,
            temperature: 0.0,
            max_tokens: None,
            system_prompt: Some(system),
            metadata: Default::default(),
        };

        let resp = self
            .llm
            .send_request(req)
            .await
            .map_err(|e| CoreError::Integration(format!("nl2sql llm error: {e}")))?;
        let content = resp
            .content
            .ok_or_else(|| CoreError::Integration("nl2sql llm returned no content".into()))?;

        Ok(Nl2SqlCandidate {
            sql: gauss_textsql::extract_sql(&content),
            explanation: None,
            confidence: None,
        })
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

    /// A scripted in-process LLM that returns a fixed fenced SQL block.
    struct ScriptedLlm(String);

    #[async_trait]
    impl LlmService for ScriptedLlm {
        async fn send_request(
            &self,
            _request: LlmRequest,
        ) -> gauss_engine::error::Result<gauss_engine::model::llm::LlmResponse> {
            Ok(gauss_engine::model::llm::LlmResponse {
                content: Some(self.0.clone()),
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn llm_translate_extracts_and_guards_sql() {
        // The in-house translator parses the fenced SQL out of the LLM reply and
        // the pipeline guardrails accept the read-only query — no external call.
        let llm = Arc::new(ScriptedLlm(
            "Here you go:\n```sql\nSELECT status, SUM(total) FROM orders GROUP BY status\n```"
                .into(),
        ));
        let pipe = Nl2SqlPipeline::new(LlmNl2Sql::new(llm));
        let guarded = pipe.propose(&req()).await.unwrap();
        assert!(guarded.sql.starts_with("SELECT status"));
    }

    #[tokio::test]
    async fn llm_translate_blocks_unsafe_sql() {
        let llm = Arc::new(ScriptedLlm("```sql\nDROP TABLE orders\n```".into()));
        let pipe = Nl2SqlPipeline::new(LlmNl2Sql::new(llm));
        assert!(pipe.propose(&req()).await.is_err());
    }
}
