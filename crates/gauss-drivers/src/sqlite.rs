//! SQLite data-source driver, backed by `sqlx`.

use async_trait::async_trait;
use gauss_core::domain::{FieldType, Fingerprint};
use gauss_core::error::{CoreError, CoreResult};
use gauss_query::{CompiledQuery, SqlParam};
use serde_json::{json, Value as JsonValue};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, Row};
use std::str::FromStr;

use crate::{DiscoveredColumn, DiscoveredTable, Driver, QueryResult};

/// A driver that executes queries against a SQLite database.
pub struct SqliteDriver {
    pool: SqlitePool,
}

impl SqliteDriver {
    /// Connect to a SQLite database by URL (e.g. `sqlite://data/source.db`),
    /// creating the database file if it does not yet exist.
    pub async fn connect(url: &str) -> CoreResult<Self> {
        let opts = SqliteConnectOptions::from_str(url)
            .map_err(storage)?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(opts)
            .await
            .map_err(storage)?;
        Ok(Self { pool })
    }

    /// Wrap an existing pool (used in tests and by the host application).
    pub fn from_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Access the underlying pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

fn storage<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Storage(e.to_string())
}

#[async_trait]
impl Driver for SqliteDriver {
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
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;

        let mut tables = Vec::with_capacity(table_rows.len());
        for tr in &table_rows {
            let name: String = tr.try_get("name").map_err(storage)?;
            // `PRAGMA table_info(?)` does not accept a bound parameter; the name
            // comes from `sqlite_master` (our own catalog) and is quote-escaped.
            let pragma = format!("PRAGMA table_info(\"{}\")", name.replace('"', "\"\""));
            let col_rows = sqlx::query(&pragma)
                .fetch_all(&self.pool)
                .await
                .map_err(storage)?;

            let mut columns = Vec::with_capacity(col_rows.len());
            for cr in &col_rows {
                let col_name: String = cr.try_get("name").map_err(storage)?;
                let decl_type: String = cr.try_get("type").map_err(storage)?;
                columns.push(DiscoveredColumn {
                    name: col_name,
                    field_type: classify(&decl_type),
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
        let sql = crate::fingerprint_sql(table, columns, |c| {
            format!("\"{}\"", c.replace('"', "\"\""))
        });
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

/// Decode a single cell to JSON, trying the common SQLite storage classes.
fn decode_cell(row: &SqliteRow, i: usize) -> JsonValue {
    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
        return v.map(|x| json!(x)).unwrap_or(JsonValue::Null);
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

/// Classify a SQLite declared type into a [`FieldType`] using affinity rules.
fn classify(decl_type: &str) -> FieldType {
    let t = decl_type.to_ascii_uppercase();
    if t.contains("INT") {
        FieldType::Integer
    } else if t.contains("CHAR") || t.contains("CLOB") || t.contains("TEXT") {
        FieldType::Text
    } else if t.contains("BOOL") {
        FieldType::Boolean
    } else if t.contains("DATE") || t.contains("TIME") {
        FieldType::DateTime
    } else if t.contains("REAL")
        || t.contains("FLOA")
        || t.contains("DOUB")
        || t.contains("NUM")
        || t.contains("DEC")
    {
        FieldType::Float
    } else {
        FieldType::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_core::gql::{CompareOp, Filter, Literal, Query};
    use sqlx::sqlite::SqlitePoolOptions;

    /// A single-connection pool so the in-memory database persists across calls.
    async fn driver() -> SqliteDriver {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        SqliteDriver::from_pool(pool)
    }

    #[tokio::test]
    async fn executes_compiled_query() {
        let d = driver().await;
        sqlx::query("CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL, status TEXT)")
            .execute(d.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO orders (total, status) VALUES (?,?),(?,?)")
            .bind(10.5)
            .bind("paid")
            .bind(3.0)
            .bind("refunded")
            .execute(d.pool())
            .await
            .unwrap();

        let mut q = Query::new("orders");
        q.fields = vec!["status".into(), "total".into()];
        q.filters = vec![Filter::Compare {
            field: "status".into(),
            op: CompareOp::Eq,
            value: Literal::Text("paid".into()),
        }];
        let compiled = gauss_query::compile(&q, &gauss_query::SqliteDialect).unwrap();

        let res = d.run(&compiled).await.unwrap();
        assert_eq!(res.columns, vec!["status".to_string(), "total".to_string()]);
        assert_eq!(res.rows.len(), 1);
        assert_eq!(res.rows[0][0], json!("paid"));
        assert_eq!(res.rows[0][1], json!(10.5));
    }

    #[tokio::test]
    async fn discovers_schema_with_types() {
        let d = driver().await;
        sqlx::query("CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL, status TEXT, created_at DATETIME)")
            .execute(d.pool())
            .await
            .unwrap();

        let schema = d.sync_schema().await.unwrap();
        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0].name, "orders");
        let by_name: std::collections::HashMap<_, _> = schema[0]
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.field_type))
            .collect();
        assert_eq!(by_name["id"], FieldType::Integer);
        assert_eq!(by_name["total"], FieldType::Float);
        assert_eq!(by_name["status"], FieldType::Text);
        assert_eq!(by_name["created_at"], FieldType::DateTime);
    }

    #[tokio::test]
    async fn fingerprints_columns_and_infers_semantics() {
        use gauss_core::domain::infer_semantic_type;
        let d = driver().await;
        sqlx::query("CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL, status TEXT)")
            .execute(d.pool())
            .await
            .unwrap();
        // 13 rows: 13 distinct totals, 2 distinct statuses, 2 NULL statuses.
        for i in 1..=13 {
            let status = if i % 5 == 0 {
                None
            } else if i % 2 == 0 {
                Some("paid")
            } else {
                Some("refunded")
            };
            sqlx::query("INSERT INTO orders (total, status) VALUES (?, ?)")
                .bind(i as f64)
                .bind(status)
                .execute(d.pool())
                .await
                .unwrap();
        }

        let fps = d
            .fingerprint("orders", &["status".to_string(), "total".to_string()])
            .await
            .unwrap();
        let by: std::collections::HashMap<_, _> = fps.into_iter().collect();

        let status = &by["status"];
        assert_eq!(status.total_rows, 13);
        assert_eq!(status.null_count, 2);
        assert_eq!(status.distinct_count, 2); // paid, refunded
                                              // Low-cardinality text -> Category.
        assert_eq!(
            infer_semantic_type(FieldType::Text, status),
            gauss_core::domain::SemanticType::Category
        );

        let total = &by["total"];
        assert_eq!(total.null_count, 0);
        assert_eq!(total.distinct_count, 13);
        assert_eq!(
            infer_semantic_type(FieldType::Float, total),
            gauss_core::domain::SemanticType::Quantity
        );
    }
}
