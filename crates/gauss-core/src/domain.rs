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
    /// Google BigQuery (REST `jobs.query`, positional `?` parameters).
    BigQuery,
    /// Snowflake (SQL REST API, positional `?` bindings).
    Snowflake,
    /// ClickHouse (HTTP interface, `{name:Type}` substitution parameters).
    ClickHouse,
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
    /// Connection URI used by a driver to reach the source (e.g.
    /// `sqlite://data/source.db`). `None` for sources not yet configured.
    #[serde(default)]
    pub connection_uri: Option<String>,
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

/// A higher-level, usage-oriented classification inferred during sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticType {
    /// Low-cardinality values suited to grouping/filtering.
    Category,
    /// A measure to aggregate.
    Quantity,
    /// A date/time dimension.
    Temporal,
    /// Free-form text.
    Text,
    /// An identifier/key.
    Key,
    Unknown,
}

/// Value statistics computed for a column during sync (a "fingerprint").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub total_rows: i64,
    pub null_count: i64,
    pub distinct_count: i64,
}

/// A column within a [`Table`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Field {
    pub id: Uuid,
    pub name: String,
    pub field_type: FieldType,
    /// Inferred semantic type (populated by sync fingerprinting).
    #[serde(default)]
    pub semantic_type: Option<SemanticType>,
    /// Value statistics (populated by sync fingerprinting).
    #[serde(default)]
    pub fingerprint: Option<Fingerprint>,
}

impl Field {
    /// Construct a field with no semantic profile yet.
    pub fn new(name: impl Into<String>, field_type: FieldType) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            field_type,
            semantic_type: None,
            fingerprint: None,
        }
    }
}

/// Infer a [`SemanticType`] from a column's declared type and value statistics.
pub fn infer_semantic_type(field_type: FieldType, fp: &Fingerprint) -> SemanticType {
    let non_null = (fp.total_rows - fp.null_count).max(0);
    match field_type {
        FieldType::DateTime => SemanticType::Temporal,
        FieldType::Boolean => SemanticType::Category,
        FieldType::Integer | FieldType::Float => {
            if fp.distinct_count > 0 && fp.distinct_count <= 12 {
                SemanticType::Category
            } else {
                SemanticType::Quantity
            }
        }
        FieldType::Text => {
            // Mostly-unique text behaves like a key; low-cardinality is a category.
            if non_null > 0 && fp.distinct_count == non_null {
                SemanticType::Key
            } else if fp.distinct_count > 0
                && (fp.distinct_count as f64) <= (non_null as f64 * 0.1).max(20.0)
            {
                SemanticType::Category
            } else {
                SemanticType::Text
            }
        }
        FieldType::Unknown => SemanticType::Unknown,
    }
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

/// A row-level-security policy: a mandatory filter automatically injected into
/// queries against `table` for **non-admin** principals. Enforced as a bound
/// GQL predicate (parameterized SQL), so it cannot be bypassed by query text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RlsPolicy {
    pub id: Uuid,
    pub database_id: Uuid,
    pub table: String,
    pub column: String,
    #[serde(default)]
    pub op: crate::gql::CompareOp,
    pub value: crate::gql::Literal,
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

/// The value type of a dashboard filter parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    Text,
    Number,
}

/// A dashboard-level filter parameter (e.g. a `status` text filter).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardParameter {
    pub name: String,
    pub kind: ParamKind,
}

/// Binds a dashboard parameter to a field of one card. When the dashboard is
/// run with a value for `parameter`, that value is injected as a **bound GQL
/// filter** (`field <op> value`) into the card's query — parameterized SQL, not
/// string interpolation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParamBinding {
    pub parameter: String,
    pub card_id: Uuid,
    pub field: String,
    #[serde(default)]
    pub op: crate::gql::CompareOp,
}

fn default_width() -> u8 {
    1
}

/// Layout entry for one card on a dashboard. The order of the `layout` vector is
/// the display order; `w` is the column span (1 = half width, 2 = full).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardLayout {
    pub card_id: Uuid,
    #[serde(default = "default_width")]
    pub w: u8,
}

/// A named tab grouping a subset of a dashboard's cards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardTab {
    pub name: String,
    pub card_ids: Vec<Uuid>,
}

/// A dashboard arranges cards for at-a-glance consumption, optionally with
/// shared filter parameters that apply across its cards and a saved layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dashboard {
    pub id: Uuid,
    pub name: String,
    pub collection_id: Option<Uuid>,
    pub card_ids: Vec<Uuid>,
    #[serde(default)]
    pub parameters: Vec<DashboardParameter>,
    #[serde(default)]
    pub bindings: Vec<ParamBinding>,
    /// Saved drag-and-drop layout (order + per-card width). Empty = default grid.
    #[serde(default)]
    pub layout: Vec<CardLayout>,
    /// Other dashboards linked from this one (dashboard-to-dashboard navigation).
    #[serde(default)]
    pub links: Vec<Uuid>,
    /// Optional tabs grouping cards into named sections.
    #[serde(default)]
    pub tabs: Vec<DashboardTab>,
}
