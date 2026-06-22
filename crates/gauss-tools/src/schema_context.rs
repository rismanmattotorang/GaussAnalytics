//! Schema-injection context enhancer.
//!
//! Before each turn this introspects the live database and prepends the
//! `CREATE TABLE` DDL of every user table to the system prompt, so the LLM can
//! write correct SQL for whatever exists right now — crucially including tables
//! created from a CSV uploaded mid-session. It optionally chains to an inner
//! enhancer (e.g. memory RAG), so it composes with the existing pipeline.
//!
//! Schema introspection uses `sqlite_master`; if that query fails (a non-SQLite
//! backend), it degrades gracefully and just passes the prompt through.

use std::sync::Arc;

use async_trait::async_trait;
use gauss_engine::context::ToolContext;
use gauss_engine::defaults::InMemoryAgentMemory;
use gauss_engine::model::user::User;
use gauss_engine::traits::{AgentMemory, LlmContextEnhancer, SqlRunner};

/// Prepends the live SQLite schema to the system prompt.
pub struct SchemaContextEnhancer {
    runner: Arc<dyn SqlRunner>,
    memory: Arc<dyn AgentMemory>,
    inner: Option<Arc<dyn LlmContextEnhancer>>,
    max_tables: usize,
}

impl SchemaContextEnhancer {
    pub fn new(runner: Arc<dyn SqlRunner>) -> Self {
        Self {
            runner,
            memory: Arc::new(InMemoryAgentMemory::new()),
            inner: None,
            max_tables: 50,
        }
    }

    /// Run `inner` first, then append the schema block (so memory RAG and the
    /// schema both make it into the prompt).
    pub fn with_inner(mut self, inner: Arc<dyn LlmContextEnhancer>) -> Self {
        self.inner = Some(inner);
        self
    }

    async fn schema_block(&self, user: &User) -> Option<String> {
        let ctx = ToolContext::new(
            user.clone(),
            "schema-introspect",
            "schema-introspect",
            self.memory.clone(),
        );
        let df = self
            .runner
            .run_sql(
                "SELECT name, sql FROM sqlite_master \
                 WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                &ctx,
            )
            .await
            .ok()?;
        if df.rows.is_empty() {
            return None;
        }
        let mut out = String::from(
            "## Database schema\n\
             The connected SQLite database has these tables. Only reference these \
             tables and columns when writing SQL:\n\n```sql\n",
        );
        for row in df.rows.iter().take(self.max_tables) {
            if let Some(sql) = row.get(1).and_then(serde_json::Value::as_str) {
                out.push_str(sql.trim());
                out.push_str(";\n");
            }
        }
        out.push_str("```\n");
        Some(out)
    }
}

#[async_trait]
impl LlmContextEnhancer for SchemaContextEnhancer {
    async fn enhance_system_prompt(
        &self,
        system_prompt: String,
        user_message: &str,
        user: &User,
    ) -> String {
        let sp = match &self.inner {
            Some(inner) => {
                inner
                    .enhance_system_prompt(system_prompt, user_message, user)
                    .await
            }
            None => system_prompt,
        };
        match self.schema_block(user).await {
            Some(block) => format!("{sp}\n\n{block}"),
            None => sp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_engine::dataframe::DataFrame;
    use gauss_engine::error::Result;
    use serde_json::json;

    /// A runner that returns a fixed `sqlite_master`-shaped result.
    struct FakeSchemaRunner;
    #[async_trait]
    impl SqlRunner for FakeSchemaRunner {
        async fn run_sql(&self, _sql: &str, _ctx: &ToolContext) -> Result<DataFrame> {
            Ok(DataFrame::new(
                vec!["name".into(), "sql".into()],
                vec![vec![
                    json!("sales"),
                    json!("CREATE TABLE sales (id INTEGER, amount REAL, region TEXT)"),
                ]],
            ))
        }
    }

    /// An inner enhancer whose contribution we can detect.
    struct MarkerInner;
    #[async_trait]
    impl LlmContextEnhancer for MarkerInner {
        async fn enhance_system_prompt(&self, sp: String, _m: &str, _u: &User) -> String {
            format!("{sp}\n[inner-ran]")
        }
    }

    #[tokio::test]
    async fn injects_schema_and_chains_inner() {
        let enh = SchemaContextEnhancer::new(Arc::new(FakeSchemaRunner))
            .with_inner(Arc::new(MarkerInner));
        let out = enh
            .enhance_system_prompt("BASE".into(), "how many sales?", &User::new("u"))
            .await;
        assert!(out.contains("BASE"));
        assert!(out.contains("[inner-ran]"), "inner enhancer must run");
        assert!(out.contains("## Database schema"));
        assert!(out.contains("CREATE TABLE sales"));
    }
}
