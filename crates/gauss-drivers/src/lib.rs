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

pub mod sqlite;

use async_trait::async_trait;
use gauss_core::domain::FieldType;
use gauss_core::error::CoreResult;
use gauss_query::CompiledQuery;
use serde_json::Value as JsonValue;

pub use sqlite::SqliteDriver;

/// The tabular result of executing a query.
#[derive(Debug, Clone, PartialEq)]
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
}
