//! MySQL data-source driver, backed by `sqlx`.
//!
//! Executes parameterized queries (`?` placeholders) and introspects schema via
//! `information_schema` scoped to the connected database. Live tests need a
//! running MySQL and are `#[ignore]`d.

use async_trait::async_trait;
use gauss_core::domain::{FieldType, Fingerprint};
use gauss_core::error::{CoreError, CoreResult};
use gauss_query::{CompiledQuery, SqlParam};
use serde_json::{json, Value as JsonValue};
use sqlx::mysql::{MySqlPool, MySqlRow};
use sqlx::{Column, Row};

use crate::{DiscoveredColumn, DiscoveredTable, Driver, QueryResult};

/// A driver that executes queries against a MySQL database.
pub struct MySqlDriver {
    pool: MySqlPool,
}

impl MySqlDriver {
    pub async fn connect(url: &str) -> CoreResult<Self> {
        let pool = MySqlPool::connect(url).await.map_err(storage)?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &MySqlPool {
        &self.pool
    }
}

fn storage<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Storage(e.to_string())
}

#[async_trait]
impl Driver for MySqlDriver {
    async fn run(&self, query: &CompiledQuery) -> CoreResult<QueryResult> {
        let mut q = sqlx::query(&query.sql);
        for p in &query.params {
            q = match p {
                SqlParam::Int(i) => q.bind(*i),
                SqlParam::Float(f) => q.bind(*f),
                SqlParam::Text(s) => q.bind(s.clone()),
                SqlParam::Bool(b) => q.bind(*b),
                SqlParam::Null => q.bind(Option::<String>::None),
            };
        }

        let rows = q.fetch_all(&self.pool).await.map_err(storage)?;
        let columns: Vec<String> = match rows.first() {
            Some(r) => r.columns().iter().map(|c| c.name().to_string()).collect(),
            None => Vec::new(),
        };

        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut cells = Vec::with_capacity(columns.len());
            for i in 0..columns.len() {
                cells.push(decode_cell(row, i));
            }
            out.push(cells);
        }
        Ok(QueryResult { columns, rows: out })
    }

    async fn sync_schema(&self) -> CoreResult<Vec<DiscoveredTable>> {
        let table_rows = sqlx::query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' \
             ORDER BY table_name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;

        let mut tables = Vec::with_capacity(table_rows.len());
        for tr in &table_rows {
            let name: String = tr.try_get("table_name").map_err(storage)?;
            let col_rows = sqlx::query(
                "SELECT column_name, data_type FROM information_schema.columns \
                 WHERE table_schema = DATABASE() AND table_name = ? ORDER BY ordinal_position",
            )
            .bind(&name)
            .fetch_all(&self.pool)
            .await
            .map_err(storage)?;

            let mut columns = Vec::with_capacity(col_rows.len());
            for cr in &col_rows {
                let col_name: String = cr.try_get("column_name").map_err(storage)?;
                let data_type: String = cr.try_get("data_type").map_err(storage)?;
                columns.push(DiscoveredColumn {
                    name: col_name,
                    field_type: classify(&data_type),
                });
            }
            tables.push(DiscoveredTable { name, columns });
        }
        Ok(tables)
    }

    async fn fingerprint(
        &self,
        table: &str,
        columns: &[String],
    ) -> CoreResult<Vec<(String, Fingerprint)>> {
        if columns.is_empty() {
            return Ok(Vec::new());
        }
        let sql = crate::fingerprint_sql(table, columns, |c| format!("`{}`", c.replace('`', "``")));
        let row = sqlx::query(&sql)
            .fetch_one(&self.pool)
            .await
            .map_err(storage)?;
        let total: i64 = row.try_get(0).map_err(storage)?;
        let mut out = Vec::with_capacity(columns.len());
        for (i, c) in columns.iter().enumerate() {
            let nonnull: i64 = row.try_get(1 + 2 * i).map_err(storage)?;
            let distinct: i64 = row.try_get(2 + 2 * i).map_err(storage)?;
            out.push((
                c.clone(),
                Fingerprint {
                    total_rows: total,
                    null_count: total - nonnull,
                    distinct_count: distinct,
                },
            ));
        }
        Ok(out)
    }
}

fn decode_cell(row: &MySqlRow, i: usize) -> JsonValue {
    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
        return v.map(|x| json!(x)).unwrap_or(JsonValue::Null);
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(i) {
        return v.map(|x| json!(x as i64)).unwrap_or(JsonValue::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
        return v.map(|x| json!(x)).unwrap_or(JsonValue::Null);
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
        return v.map(|x| json!(x)).unwrap_or(JsonValue::Null);
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return v.map(|x| json!(x)).unwrap_or(JsonValue::Null);
    }
    JsonValue::Null
}

/// Classify a MySQL `information_schema` data type into a [`FieldType`].
fn classify(data_type: &str) -> FieldType {
    let t = data_type.to_ascii_lowercase();
    match t.as_str() {
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" => FieldType::Integer,
        "decimal" | "numeric" | "float" | "double" => FieldType::Float,
        "bool" | "boolean" => FieldType::Boolean,
        _ if t.contains("timestamp") || t.contains("date") || t.contains("time") => {
            FieldType::DateTime
        }
        _ if t.contains("char") || t.contains("text") => FieldType::Text,
        _ => FieldType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requires a live MySQL; set `GAUSS_TEST_MYSQL_URL` and run with
    /// `cargo test -p gauss-drivers -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn mysql_sync_and_run() {
        use gauss_core::gql::Query;
        let url = std::env::var("GAUSS_TEST_MYSQL_URL").expect("GAUSS_TEST_MYSQL_URL");
        let d = MySqlDriver::connect(&url).await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS gauss_t (id INT, label VARCHAR(64))")
            .execute(d.pool())
            .await
            .unwrap();
        let schema = d.sync_schema().await.unwrap();
        assert!(schema.iter().any(|t| t.name == "gauss_t"));
        let compiled =
            gauss_query::compile(&Query::new("gauss_t"), &gauss_query::MySqlDialect).unwrap();
        let _ = d.run(&compiled).await.unwrap();
    }
}
