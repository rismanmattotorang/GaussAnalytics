//! SQL dialect abstraction.
//!
//! Per-database differences (identifier quoting and parameter placeholder
//! style) are isolated behind the [`Dialect`] trait. The reference engine
//! achieves the same with driver multimethods; in Rust this is a trait with one
//! implementation per supported database.

use gauss_core::domain::DataSourceKind;

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
    fn placeholder(&self, n: usize) -> String;
}

/// Resolve the [`Dialect`] for a connected data source kind.
pub fn for_kind(kind: DataSourceKind) -> Box<dyn Dialect> {
    match kind {
        DataSourceKind::Postgres => Box::new(PostgresDialect),
        DataSourceKind::MySql => Box::new(MySqlDialect),
        DataSourceKind::Sqlite => Box::new(SqliteDialect),
        DataSourceKind::Generic => Box::new(GenericDialect),
    }
}

/// Double-quote identifier style (`"col"`), escaping `"` as `""`.
fn double_quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
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
    fn placeholder(&self, n: usize) -> String {
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
        format!("`{}`", ident.replace('`', "``"))
    }
    fn placeholder(&self, _n: usize) -> String {
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
    fn placeholder(&self, _n: usize) -> String {
        "?".to_string()
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
    fn placeholder(&self, _n: usize) -> String {
        "?".to_string()
    }
}
