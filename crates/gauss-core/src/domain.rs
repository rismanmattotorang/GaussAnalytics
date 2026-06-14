//! Core domain entities.
//!
//! These are the persistent concepts of GaussAnalytics: the people who use it,
//! the data sources they connect, and the saved analytical content they build.
//! Persistence lives in `gauss-db`; this module only describes the shapes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A registered user of the platform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    /// Whether the user holds the platform administrator role.
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
}

/// The kind of an external data source GaussAnalytics can query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceKind {
    Postgres,
    MySql,
    Sqlite,
    /// A source whose dialect is standard-SQL-compatible.
    Generic,
}

/// A connected data source (a database GaussAnalytics can run queries against).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Database {
    pub id: Uuid,
    pub name: String,
    pub kind: DataSourceKind,
    /// Whether schema sync has populated this database's tables.
    pub is_synced: bool,
    pub created_at: DateTime<Utc>,
}

/// A semantic classification of a column, used to drive UI and validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Integer,
    Float,
    Text,
    Boolean,
    DateTime,
    /// Unknown / not yet classified.
    Unknown,
}

/// A column within a [`Table`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Field {
    pub id: Uuid,
    pub name: String,
    pub field_type: FieldType,
}

/// A table discovered within a [`Database`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Table {
    pub id: Uuid,
    pub database_id: Uuid,
    pub name: String,
    pub fields: Vec<Field>,
}

impl Table {
    /// Look up a field by name (case-sensitive), used during GQL validation.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// A saved question: a named, reusable [`crate::gql::Query`] over a database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Card {
    pub id: Uuid,
    pub name: String,
    pub database_id: Uuid,
    pub query: crate::gql::Query,
    pub created_at: DateTime<Utc>,
}

/// A collection groups content (cards, dashboards) for organization + perms.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Collection {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
}

/// A dashboard arranges cards for at-a-glance consumption.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dashboard {
    pub id: Uuid,
    pub name: String,
    pub collection_id: Option<Uuid>,
    pub card_ids: Vec<Uuid>,
}
