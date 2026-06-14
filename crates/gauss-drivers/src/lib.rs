//! `gauss-drivers` — execute compiled queries against connected data sources
//! and discover their schema.
//!
//! This is the Rust replacement for the reference engine's JDBC-based driver
//! multimethods. Where `gauss-query` turns GQL into a parameterized
//! [`CompiledQuery`], a [`Driver`] is what actually runs it against a live
//! source and streams back rows, and what introspects a source's tables and
//! columns during sync.
//!
//! Phase 2 ships a fully-working SQLite driver (in-process, so it is testable
//! without external infrastructure). Postgres and MySQL drivers slot in behind
//! the same trait using the same `sqlx` pool pattern.

#![forbid(unsafe_code)]

pub mod bigquery;
pub mod clickhouse;
pub mod mysql;
pub mod postgres;
pub mod rest;
pub mod snowflake;
pub mod sqlite;

use async_trait::async_trait;
use gauss_core::domain::{DataSourceKind, FieldType};
use gauss_core::error::{CoreError, CoreResult};
use gauss_query::CompiledQuery;
use serde::Serialize;
use serde_json::Value as JsonValue;

pub use bigquery::BigQueryDriver;
pub use clickhouse::ClickHouseDriver;
pub use gauss_core::domain::Fingerprint;
pub use mysql::MySqlDriver;
pub use postgres::PgDriver;
pub use snowflake::SnowflakeDriver;
pub use sqlite::SqliteDriver;

/// The tabular result of executing a query.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryResult {
    /// Column names, in result order.
    pub columns: Vec<String>,
    /// Rows; each row has one cell per column, as a JSON value.
    pub rows: Vec<Vec<JsonValue>>,
}

/// A column discovered during schema introspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredColumn {
    pub name: String,
    pub field_type: FieldType,
}

/// A table discovered during schema introspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredTable {
    pub name: String,
    pub columns: Vec<DiscoveredColumn>,
}

/// The capability surface of a data-source driver.
#[async_trait]
pub trait Driver: Send + Sync {
    /// Execute a parameterized query and return its rows.
    async fn run(&self, query: &CompiledQuery) -> CoreResult<QueryResult>;

    /// Introspect the source and return its tables and columns.
    async fn sync_schema(&self) -> CoreResult<Vec<DiscoveredTable>>;

    /// Compute value statistics (a fingerprint) for the named columns of a
    /// table, returned in the same order as `columns`.
    async fn fingerprint(
        &self,
        table: &str,
        columns: &[String],
    ) -> CoreResult<Vec<(String, Fingerprint)>>;
}

/// Shared SQL builder for fingerprints: one row of
/// `COUNT(*), [COUNT(c), COUNT(DISTINCT c)]*` for the given quoted columns.
/// `quote` applies the dialect's identifier quoting. `COUNT` is 64-bit in every
/// supported engine, so the result decodes uniformly to `i64`.
pub(crate) fn fingerprint_sql(
    table: &str,
    columns: &[String],
    quote: impl Fn(&str) -> String,
) -> String {
    let mut sql = String::from("SELECT COUNT(*)");
    for c in columns {
        let q = quote(c);
        sql.push_str(&format!(", COUNT({q}), COUNT(DISTINCT {q})"));
    }
    sql.push_str(&format!(" FROM {}", quote(table)));
    sql
}

/// Build a [`Driver`] for a data source of the given `kind` at `uri`.
///
/// SQLite is implemented today; other kinds return an error until their drivers
/// land (same `sqlx` pool pattern).
pub async fn connect(kind: DataSourceKind, uri: &str) -> CoreResult<Box<dyn Driver>> {
    match kind {
        DataSourceKind::Sqlite => Ok(Box::new(SqliteDriver::connect(uri).await?)),
        DataSourceKind::Postgres => Ok(Box::new(PgDriver::connect(uri).await?)),
        DataSourceKind::MySql => Ok(Box::new(MySqlDriver::connect(uri).await?)),
        DataSourceKind::BigQuery => Ok(Box::new(BigQueryDriver::connect(uri)?)),
        DataSourceKind::Snowflake => Ok(Box::new(SnowflakeDriver::connect(uri)?)),
        DataSourceKind::ClickHouse => Ok(Box::new(ClickHouseDriver::connect(uri)?)),
        DataSourceKind::Generic => Err(CoreError::Integration(
            "no driver for the Generic data source kind".into(),
        )),
    }
}
