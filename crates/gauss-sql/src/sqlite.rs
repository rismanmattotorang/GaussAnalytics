//! SQLite `SqlRunner` backed by `rusqlite` (bundled, no system dependency).
//!
//! `rusqlite` is synchronous, so queries run on a blocking thread-pool via
//! `spawn_blocking` to keep the async runtime unblocked.

use async_trait::async_trait;
use gauss_engine::context::ToolContext;
use gauss_engine::dataframe::DataFrame;
use gauss_engine::error::{AgentError, Result};
use gauss_engine::traits::SqlRunner;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde_json::{json, Value};

/// Runs SQL against a SQLite database file (or `:memory:`).
pub struct SqliteRunner {
    path: String,
}

impl SqliteRunner {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

fn value_ref_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::String(format!("<blob {} bytes>", b.len())),
    }
}

#[async_trait]
impl SqlRunner for SqliteRunner {
    async fn run_sql(&self, sql: &str, _context: &ToolContext) -> Result<DataFrame> {
        let path = self.path.clone();
        let sql = sql.to_string();

        tokio::task::spawn_blocking(move || -> Result<DataFrame> {
            let conn =
                Connection::open(&path).map_err(|e| AgentError::other(format!("open db: {e}")))?;
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| AgentError::other(format!("prepare: {e}")))?;
            let col_count = stmt.column_count();

            // No result columns → treat as a DML/DDL statement.
            if col_count == 0 {
                drop(stmt);
                let affected = conn
                    .execute(&sql, [])
                    .map_err(|e| AgentError::other(format!("execute: {e}")))?;
                return Ok(DataFrame::new(
                    vec!["rows_affected".to_string()],
                    vec![vec![json!(affected)]],
                ));
            }

            let columns: Vec<String> = stmt
                .column_names()
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            let mut out_rows: Vec<Vec<Value>> = Vec::new();
            let mut rows = stmt
                .query([])
                .map_err(|e| AgentError::other(format!("query: {e}")))?;
            while let Some(row) = rows
                .next()
                .map_err(|e| AgentError::other(format!("row: {e}")))?
            {
                let mut r = Vec::with_capacity(col_count);
                for i in 0..col_count {
                    let v = row
                        .get_ref(i)
                        .map_err(|e| AgentError::other(format!("cell: {e}")))?;
                    r.push(value_ref_to_json(v));
                }
                out_rows.push(r);
            }
            Ok(DataFrame::new(columns, out_rows))
        })
        .await
        .map_err(|e| AgentError::other(format!("blocking task: {e}")))?
    }
}
