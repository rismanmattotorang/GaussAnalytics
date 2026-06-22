//! `run_sql` tool: executes a SQL query via an injected `SqlRunner` and streams
//! the result as a `DataFrameComponent`. Mirrors `gauss/tools/run_sql.py`.

use async_trait::async_trait;
use gauss_engine::components::{RichComponent, UiComponent};
use gauss_engine::context::{ToolContext, ToolResult};
use gauss_engine::tool::Tool;
use gauss_engine::traits::{FileSystem, SqlRunner};
use gauss_sqlguard::Guardrails;
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

/// Arguments for the `run_sql` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunSqlArgs {
    /// The SQL query to execute.
    pub sql: String,
}

/// Executes SQL against the injected database runner.
pub struct RunSqlTool {
    runner: Arc<dyn SqlRunner>,
    access_groups: Vec<String>,
    /// When set, SELECT results are written here as CSV so `visualize_data`
    /// (and the user) can consume them.
    file_system: Option<Arc<dyn FileSystem>>,
    /// Dry-plan guardrails applied before execution. Read-only by default.
    guardrails: Guardrails,
}

impl RunSqlTool {
    pub fn new(runner: Arc<dyn SqlRunner>) -> Self {
        Self {
            runner,
            access_groups: vec!["user".to_string(), "admin".to_string()],
            file_system: None,
            guardrails: Guardrails::read_only(),
        }
    }

    pub fn with_access_groups(mut self, groups: Vec<String>) -> Self {
        self.access_groups = groups;
        self
    }

    /// Persist query results as CSV via the given file system.
    pub fn with_file_system(mut self, fs: Arc<dyn FileSystem>) -> Self {
        self.file_system = Some(fs);
        self
    }

    /// Override the guardrails (e.g. to allow writes or inject a LIMIT).
    pub fn with_guardrails(mut self, guardrails: Guardrails) -> Self {
        self.guardrails = guardrails;
        self
    }
}

/// Cap the amount of tabular data fed back to the LLM.
const MAX_LLM_ROWS: usize = 50;

#[async_trait]
impl Tool for RunSqlTool {
    type Args = RunSqlArgs;

    fn name(&self) -> &str {
        "run_sql"
    }

    fn description(&self) -> &str {
        "Execute a SQL query against the connected database and return the results as a table."
    }

    fn access_groups(&self) -> Vec<String> {
        self.access_groups.clone()
    }

    async fn execute(&self, context: &ToolContext, args: RunSqlArgs) -> ToolResult {
        // Dry-plan: validate and (optionally) fix the SQL before touching the DB.
        let safe_sql = match self.guardrails.check_and_fix(&args.sql) {
            Ok(sql) => sql,
            Err(e) => return ToolResult::error(format!("SQL rejected by guardrails: {e}")),
        };
        match self.runner.run_sql(&safe_sql, context).await {
            Ok(df) => {
                let row_count = df.row_count();
                let columns = df.columns.clone();

                // Build a bounded CSV summary for the LLM.
                let llm_view = if row_count > MAX_LLM_ROWS {
                    let mut head = df.clone();
                    head.rows.truncate(MAX_LLM_ROWS);
                    format!(
                        "{}\n... ({} of {} rows shown)",
                        head.to_csv().trim_end(),
                        MAX_LLM_ROWS,
                        row_count
                    )
                } else {
                    df.to_csv().trim_end().to_string()
                };

                let component = UiComponent::new(RichComponent::dataframe(
                    df.to_records(),
                    columns.clone(),
                    Some("Query Results".to_string()),
                ));

                // Optionally persist as CSV for downstream tools (e.g. visualize_data).
                let mut saved_note = String::new();
                if let Some(fs) = &self.file_system {
                    let filename = format!(
                        "query_results_{}.csv",
                        &uuid::Uuid::new_v4().simple().to_string()[..8]
                    );
                    match fs.write_file(&filename, &df.to_csv(), context, true).await {
                        Ok(()) => {
                            saved_note = format!(
                                "\nResults saved to `{filename}`. To chart them, call \
                                 visualize_data with this filename."
                            );
                        }
                        Err(e) => {
                            saved_note = format!("\n(Note: could not save CSV: {e})");
                        }
                    }
                }

                ToolResult::success(format!(
                    "Query returned {row_count} row(s) with columns [{}].\n{llm_view}{saved_note}",
                    columns.join(", ")
                ))
                .with_ui(component)
            }
            Err(e) => ToolResult::error(format!("SQL execution failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_engine::dataframe::DataFrame;
    use gauss_engine::defaults::InMemoryAgentMemory;
    use gauss_engine::error::Result;
    use gauss_engine::model::user::User;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Records whether the runner was ever invoked.
    struct SpyRunner {
        called: AtomicBool,
    }
    #[async_trait]
    impl SqlRunner for SpyRunner {
        async fn run_sql(&self, _sql: &str, _ctx: &ToolContext) -> Result<DataFrame> {
            self.called.store(true, Ordering::SeqCst);
            Ok(DataFrame::new(
                vec!["x".into()],
                vec![vec![serde_json::json!(1)]],
            ))
        }
    }

    fn ctx() -> ToolContext {
        ToolContext::new(
            User::new("u"),
            "c",
            "r",
            Arc::new(InMemoryAgentMemory::new()),
        )
    }

    #[tokio::test]
    async fn rejects_writes_before_touching_db() {
        let runner = Arc::new(SpyRunner {
            called: AtomicBool::new(false),
        });
        let tool = RunSqlTool::new(runner.clone());
        let r = tool
            .execute(
                &ctx(),
                RunSqlArgs {
                    sql: "DELETE FROM customers".into(),
                },
            )
            .await;
        assert!(!r.success, "DELETE should be rejected");
        assert!(r.error.unwrap().contains("guardrails"));
        // The runner must never have been called.
        assert!(!runner.called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn allows_select() {
        let runner = Arc::new(SpyRunner {
            called: AtomicBool::new(false),
        });
        let tool = RunSqlTool::new(runner.clone());
        let r = tool
            .execute(
                &ctx(),
                RunSqlArgs {
                    sql: "SELECT 1 AS x".into(),
                },
            )
            .await;
        assert!(r.success, "{:?}", r.error);
        assert!(runner.called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn allow_writes_opt_in() {
        let runner = Arc::new(SpyRunner {
            called: AtomicBool::new(false),
        });
        let tool =
            RunSqlTool::new(runner.clone()).with_guardrails(Guardrails::default().allow_writes());
        let r = tool
            .execute(
                &ctx(),
                RunSqlArgs {
                    sql: "UPDATE t SET a=1 WHERE id=2".into(),
                },
            )
            .await;
        assert!(r.success, "{:?}", r.error);
    }
}
