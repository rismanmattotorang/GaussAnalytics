//! SQL dialect abstraction.
//!
//! Per-database differences (identifier quoting and parameter placeholder
//! style) are isolated behind the [`Dialect`] trait. The reference engine
//! achieves the same with driver multimethods; in Rust this is a trait with one
//! implementation per supported database.

use gauss_core::domain::DataSourceKind;

use crate::SqlParam;

/// SQL generation rules that vary by database.
pub trait Dialect: Send + Sync {
    /// A human-readable dialect name (for logs/diagnostics).
    fn name(&self) -> &'static str;

    /// Quote an identifier (table/column), escaping the quote character.
    ///
    /// Identifiers come from validated, synced metadata, but we escape anyway
    /// as defense in depth.
    fn quote_ident(&self, ident: &str) -> String;

    /// Render the placeholder for the `n`-th bound parameter (1-based).
    ///
    /// `param` is provided because some engines (e.g. ClickHouse) require the
    /// parameter's type in the placeholder itself (`{p1:Int64}`); most dialects
    /// ignore it.
    fn placeholder(&self, n: usize, param: &SqlParam) -> String;

    /// Render the row-limit clause for `n` rows, appended after `ORDER BY`.
    ///
    /// Most engines use `LIMIT n`; the default reflects that. Engines that do
    /// not (Oracle, SQL Server, DB2) override this with the ANSI `OFFSET/FETCH`
    /// form. `n` is an integer the compiler controls, so inlining is
    /// injection-safe.
    fn limit_clause(&self, n: u64) -> String {
        format!("LIMIT {n}")
    }
}

/// Resolve the [`Dialect`] for a connected data source kind.
pub fn for_kind(kind: DataSourceKind) -> Box<dyn Dialect> {
    match kind {
        DataSourceKind::Postgres => Box::new(PostgresDialect),
        DataSourceKind::MySql => Box::new(MySqlDialect),
        DataSourceKind::Sqlite => Box::new(SqliteDialect),
        DataSourceKind::Oracle => Box::new(OracleDialect),
        DataSourceKind::BigQuery => Box::new(BigQueryDialect),
        DataSourceKind::Snowflake => Box::new(SnowflakeDialect),
        DataSourceKind::ClickHouse => Box::new(ClickHouseDialect),
        DataSourceKind::Generic => Box::new(GenericDialect),
    }
}

/// Double-quote identifier style (`"col"`), escaping `"` as `""`.
fn double_quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Backtick identifier style (`` `col` ``), escaping `` ` `` as ` `` `.
fn backtick(ident: &str) -> String {
    format!("`{}`", ident.replace('`', "``"))
}

/// PostgreSQL: double-quoted identifiers, `$n` placeholders.
pub struct PostgresDialect;
impl Dialect for PostgresDialect {
    fn name(&self) -> &'static str {
        "postgres"
    }
    fn quote_ident(&self, ident: &str) -> String {
        double_quote(ident)
    }
    fn placeholder(&self, n: usize, _param: &SqlParam) -> String {
        format!("${n}")
    }
}

/// MySQL: backtick identifiers, `?` placeholders.
pub struct MySqlDialect;
impl Dialect for MySqlDialect {
    fn name(&self) -> &'static str {
        "mysql"
    }
    fn quote_ident(&self, ident: &str) -> String {
        backtick(ident)
    }
    fn placeholder(&self, _n: usize, _param: &SqlParam) -> String {
        "?".to_string()
    }
}

/// SQLite: double-quoted identifiers, `?` placeholders.
pub struct SqliteDialect;
impl Dialect for SqliteDialect {
    fn name(&self) -> &'static str {
        "sqlite"
    }
    fn quote_ident(&self, ident: &str) -> String {
        double_quote(ident)
    }
    fn placeholder(&self, _n: usize, _param: &SqlParam) -> String {
        "?".to_string()
    }
}

/// Oracle: double-quoted identifiers, `:n` numbered bind variables, and the
/// ANSI `OFFSET 0 ROWS FETCH NEXT n ROWS ONLY` paging form (Oracle has no
/// `LIMIT`). Oracle folds unquoted identifiers to upper-case, but our metadata
/// is synced with the source's exact names, so quoting them verbatim is correct.
pub struct OracleDialect;
impl Dialect for OracleDialect {
    fn name(&self) -> &'static str {
        "oracle"
    }
    fn quote_ident(&self, ident: &str) -> String {
        double_quote(ident)
    }
    fn placeholder(&self, n: usize, _param: &SqlParam) -> String {
        format!(":{n}")
    }
    fn limit_clause(&self, n: u64) -> String {
        format!("OFFSET 0 ROWS FETCH NEXT {n} ROWS ONLY")
    }
}

/// BigQuery: backtick identifiers, positional `?` parameters (StandardSQL).
pub struct BigQueryDialect;
impl Dialect for BigQueryDialect {
    fn name(&self) -> &'static str {
        "bigquery"
    }
    fn quote_ident(&self, ident: &str) -> String {
        backtick(ident)
    }
    fn placeholder(&self, _n: usize, _param: &SqlParam) -> String {
        "?".to_string()
    }
}

/// Snowflake: double-quoted identifiers, positional `?` bindings.
pub struct SnowflakeDialect;
impl Dialect for SnowflakeDialect {
    fn name(&self) -> &'static str {
        "snowflake"
    }
    fn quote_ident(&self, ident: &str) -> String {
        double_quote(ident)
    }
    fn placeholder(&self, _n: usize, _param: &SqlParam) -> String {
        "?".to_string()
    }
}

/// ClickHouse type name for a parameter (used in `{pN:Type}` placeholders).
fn clickhouse_type(param: &SqlParam) -> &'static str {
    match param {
        SqlParam::Int(_) => "Int64",
        SqlParam::Float(_) => "Float64",
        SqlParam::Text(_) => "String",
        SqlParam::Bool(_) => "UInt8",
        SqlParam::Null => "Nullable(String)",
    }
}

/// ClickHouse: backtick identifiers, typed `{pN:Type}` substitution params.
pub struct ClickHouseDialect;
impl Dialect for ClickHouseDialect {
    fn name(&self) -> &'static str {
        "clickhouse"
    }
    fn quote_ident(&self, ident: &str) -> String {
        backtick(ident)
    }
    fn placeholder(&self, n: usize, param: &SqlParam) -> String {
        format!("{{p{n}:{}}}", clickhouse_type(param))
    }
}

/// Standard-SQL fallback: double-quoted identifiers, `?` placeholders.
pub struct GenericDialect;
impl Dialect for GenericDialect {
    fn name(&self) -> &'static str {
        "generic"
    }
    fn quote_ident(&self, ident: &str) -> String {
        double_quote(ident)
    }
    fn placeholder(&self, _n: usize, _param: &SqlParam) -> String {
        "?".to_string()
    }
}
