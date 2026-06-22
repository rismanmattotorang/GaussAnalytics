//! PostgreSQL `SqlRunner` backed by `sqlx`.
//!
//! Column values are decoded by Postgres type name into JSON. Common types are
//! handled explicitly; anything else falls back to a text decode (or null).
//! NUMERIC is best-effort; richer decimal handling can be added with the
//! `sqlx` `rust_decimal`/`bigdecimal` features in a follow-up.

use async_trait::async_trait;
use gauss_engine::context::ToolContext;
use gauss_engine::dataframe::DataFrame;
use gauss_engine::error::{AgentError, Result};
use gauss_engine::traits::SqlRunner;
use serde_json::{json, Value};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Column, Row, TypeInfo};

/// Runs SQL against a PostgreSQL database via a connection pool.
pub struct PostgresRunner {
    pool: PgPool,
}

impl PostgresRunner {
    /// Connect and build a pool. `url` is a standard `postgres://…` URL.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(url)
            .await
            .map_err(|e| AgentError::other(format!("connect postgres: {e}")))?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn is_read_query(sql: &str) -> bool {
    let first = sql
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        first.as_str(),
        "SELECT" | "WITH" | "SHOW" | "EXPLAIN" | "TABLE" | "VALUES"
    )
}

fn decode_cell(row: &PgRow, i: usize) -> Value {
    let type_name = row.column(i).type_info().name();
    match type_name {
        "INT2" | "INT4" | "INT8" => row
            .try_get::<Option<i64>, _>(i)
            .ok()
            .flatten()
            .map_or(Value::Null, |v| json!(v)),
        "FLOAT4" | "FLOAT8" => row
            .try_get::<Option<f64>, _>(i)
            .ok()
            .flatten()
            .map_or(Value::Null, |v| json!(v)),
        "BOOL" => row
            .try_get::<Option<bool>, _>(i)
            .ok()
            .flatten()
            .map_or(Value::Null, |v| json!(v)),
        "JSON" | "JSONB" => row
            .try_get::<Option<Value>, _>(i)
            .ok()
            .flatten()
            .unwrap_or(Value::Null),
        "TIMESTAMP" => row
            .try_get::<Option<chrono::NaiveDateTime>, _>(i)
            .ok()
            .flatten()
            .map_or(Value::Null, |v| json!(v.to_string())),
        "TIMESTAMPTZ" => row
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(i)
            .ok()
            .flatten()
            .map_or(Value::Null, |v| json!(v.to_rfc3339())),
        "DATE" => row
            .try_get::<Option<chrono::NaiveDate>, _>(i)
            .ok()
            .flatten()
            .map_or(Value::Null, |v| json!(v.to_string())),
        // TEXT-like and unknown types: best-effort string decode.
        _ => row
            .try_get::<Option<String>, _>(i)
            .ok()
            .flatten()
            .map_or(Value::Null, Value::String),
    }
}

#[async_trait]
impl SqlRunner for PostgresRunner {
    async fn run_sql(&self, sql: &str, _context: &ToolContext) -> Result<DataFrame> {
        if !is_read_query(sql) {
            let result = sqlx::query(sql)
                .execute(&self.pool)
                .await
                .map_err(|e| AgentError::other(format!("execute: {e}")))?;
            return Ok(DataFrame::new(
                vec!["rows_affected".to_string()],
                vec![vec![json!(result.rows_affected())]],
            ));
        }

        let rows = sqlx::query(sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AgentError::other(format!("query: {e}")))?;

        let Some(first) = rows.first() else {
            return Ok(DataFrame::default());
        };
        let columns: Vec<String> = first
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();
        let col_count = columns.len();

        let out_rows = rows
            .iter()
            .map(|row| (0..col_count).map(|i| decode_cell(row, i)).collect())
            .collect();

        Ok(DataFrame::new(columns, out_rows))
    }
}
