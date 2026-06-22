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
///
/// Its wire form (API JSON), its persisted form (the metadata store), and its
/// dialect/driver resolution all flow through a single canonical string mapping
/// ([`DataSourceKind::as_str`] / [`DataSourceKind::from_kind_str`]), so the
/// frontend, the database, and the backend can never disagree on a kind's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataSourceKind {
    Postgres,
    MySql,
    Sqlite,
    /// Oracle Database (ORDS REST, `:n` bind variables, `FETCH FIRST` paging).
    Oracle,
    /// Google BigQuery (REST `jobs.query`, positional `?` parameters).
    BigQuery,
    /// Snowflake (SQL REST API, positional `?` bindings).
    Snowflake,
    /// ClickHouse (HTTP interface, `{name:Type}` substitution parameters).
    ClickHouse,
    /// A source whose dialect is standard-SQL-compatible.
    Generic,
}

impl DataSourceKind {
    /// The canonical lowercase identifier for this kind. This is the single
    /// source of truth shared by JSON (serde), persistence, and the frontend.
    pub fn as_str(&self) -> &'static str {
        match self {
            DataSourceKind::Postgres => "postgres",
            DataSourceKind::MySql => "mysql",
            DataSourceKind::Sqlite => "sqlite",
            DataSourceKind::Oracle => "oracle",
            DataSourceKind::BigQuery => "bigquery",
            DataSourceKind::Snowflake => "snowflake",
            DataSourceKind::ClickHouse => "clickhouse",
            DataSourceKind::Generic => "generic",
        }
    }

    /// Parse a canonical kind identifier, or `None` if unrecognized.
    pub fn from_kind_str(s: &str) -> Option<Self> {
        Some(match s {
            "postgres" => DataSourceKind::Postgres,
            "mysql" => DataSourceKind::MySql,
            "sqlite" => DataSourceKind::Sqlite,
            "oracle" => DataSourceKind::Oracle,
            "bigquery" => DataSourceKind::BigQuery,
            "snowflake" => DataSourceKind::Snowflake,
            "clickhouse" => DataSourceKind::ClickHouse,
            "generic" => DataSourceKind::Generic,
            _ => return None,
        })
    }
}

impl std::fmt::Display for DataSourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for DataSourceKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DataSourceKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        DataSourceKind::from_kind_str(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown data source kind {s:?}")))
    }
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

/// A free-form Markdown panel on a dashboard (headings, notes, links) — the
/// analog of Metabase's dashboard text cards. It carries content, not a query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardTextCard {
    pub id: Uuid,
    /// Markdown source rendered by the web UI.
    pub markdown: String,
    /// Column span (1 = half width, 2 = full), matching [`CardLayout`].
    #[serde(default = "default_width")]
    pub w: u8,
}

/// A dashboard tile backed by a **notebook cell's output** — a chart, big
/// number, table, or image computed in Python/SQL/ML on the user's kernel and
/// pinned here. The rendered output is stored as a JSON `snapshot`, refreshed on
/// publish and (optionally) on a schedule, so the dashboard renders without a
/// live kernel. This is GaussAnalytics-only: a dashboard tile whose value is an
/// arbitrary computed notebook result, not just a SQL query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardNotebookCard {
    pub id: Uuid,
    pub notebook_id: Uuid,
    pub cell_id: Uuid,
    pub title: String,
    /// Render hint for the snapshot: `chart` | `big_number` | `table` | `image`.
    pub view: String,
    /// JSON snapshot of the cell's last output. Shape:
    /// `{ "result"?: {columns, rows}, "image"?: "<base64 png>", "html"?: ...,
    /// "text"?: ..., "sql"?: ... }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    /// Column span (1 = half width, 2 = full), matching [`CardLayout`].
    #[serde(default = "default_width")]
    pub w: u8,
    /// When the snapshot was last refreshed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<DateTime<Utc>>,
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
    /// Free-form Markdown panels (titles, notes, links) shown alongside cards.
    #[serde(default)]
    pub text_cards: Vec<DashboardTextCard>,
    /// Tiles backed by notebook-cell outputs (computed charts/numbers/tables).
    #[serde(default)]
    pub notebook_cards: Vec<DashboardNotebookCard>,
}

/// The kind of a notebook cell.
///
/// - `Markdown` — prose rendered by the web UI (not executed).
/// - `Python` — code executed on the user's local Jupyter kernel.
/// - `Sql` — read-only SQL run against a data source; its result is injected
///   into the kernel as a pandas `DataFrame` (named by `output_var`).
/// - `Nl2sql` — a natural-language prompt translated to guardrailed SQL, then
///   run and injected exactly like a `Sql` cell.
/// - `Input` — a named variable (text/number) injected into the kernel, so
///   changing it and re-running downstream cells recomputes results.
/// - `Chart` — a nivo visualization of a kernel `DataFrame` (named by
///   `input_var`); the rows are fetched and rendered in the web UI.
/// - `BigNumber` — a single headline value taken from a kernel `DataFrame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellKind {
    Markdown,
    Python,
    Sql,
    Nl2sql,
    Input,
    Chart,
    BigNumber,
}

/// One cell of a notebook. `source` carries the cell's primary text — code,
/// Markdown, SQL, an NL prompt, or (for `Input`) the current value. The optional
/// fields apply to data/input cells and default to absent for plain
/// Markdown/Python cells (so older notebooks deserialize unchanged).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotebookCell {
    pub id: Uuid,
    pub kind: CellKind,
    #[serde(default)]
    pub source: String,
    /// Target data source for `Sql` / `Nl2sql` cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_id: Option<Uuid>,
    /// Variable name the resulting `DataFrame` is bound to (`Sql` / `Nl2sql`).
    /// Defaults to `df` when empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_var: Option<String>,
    /// Variable name an `Input` cell injects into the kernel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_var: Option<String>,
}

/// An embedded data notebook: an ordered list of Markdown/Python cells. Code
/// cells execute on the user's **local** Jupyter kernel (GaussAnalytics never
/// runs arbitrary code in its own process). Persisted as content like cards and
/// dashboards; a running kernel is tracked server-side, not stored here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Notebook {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub collection_id: Option<Uuid>,
    #[serde(default)]
    pub cells: Vec<NotebookCell>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::DataSourceKind;

    #[test]
    fn kind_strings_are_canonical_one_word() {
        // The JSON wire form, the persisted form, and the frontend union all
        // share these exact strings (no serde snake_case surprises like
        // "my_sql" / "click_house").
        for (kind, s) in [
            (DataSourceKind::Postgres, "postgres"),
            (DataSourceKind::MySql, "mysql"),
            (DataSourceKind::Sqlite, "sqlite"),
            (DataSourceKind::Oracle, "oracle"),
            (DataSourceKind::BigQuery, "bigquery"),
            (DataSourceKind::Snowflake, "snowflake"),
            (DataSourceKind::ClickHouse, "clickhouse"),
            (DataSourceKind::Generic, "generic"),
        ] {
            assert_eq!(kind.as_str(), s);
            assert_eq!(DataSourceKind::from_kind_str(s), Some(kind));
            // serde agrees with the canonical mapping, and round-trips.
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{s}\""));
            assert_eq!(
                serde_json::from_str::<DataSourceKind>(&format!("\"{s}\"")).unwrap(),
                kind
            );
        }
        assert_eq!(DataSourceKind::from_kind_str("nope"), None);
        assert!(serde_json::from_str::<DataSourceKind>("\"nope\"").is_err());
    }
}
