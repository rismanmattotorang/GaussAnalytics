//! SQL guardrails ("dry-plan"): parse-validate a statement, enforce read-only,
//! check a table allowlist, and inject a default LIMIT — all before any database
//! execution. Shared by the `text_to_sql` pipeline and the raw `run_sql` tool so
//! governance is applied consistently.

use sqlparser::ast::{ObjectName, Query, Statement, Visit, Visitor};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::collections::HashSet;
use std::ops::ControlFlow;

/// Configurable SQL guardrails.
#[derive(Debug, Clone)]
pub struct Guardrails {
    /// Reject anything that is not a read-only query.
    pub enforce_read_only: bool,
    /// Lower-cased allowed table names; empty means "no allowlist".
    pub allowed_tables: HashSet<String>,
    /// If set, a `LIMIT` is appended to limit-less queries.
    pub default_limit: Option<u64>,
}

impl Default for Guardrails {
    fn default() -> Self {
        Self {
            enforce_read_only: true,
            allowed_tables: HashSet::new(),
            default_limit: None,
        }
    }
}

impl Guardrails {
    /// Read-only, no allowlist, no LIMIT injection — the secure baseline.
    pub fn read_only() -> Self {
        Self::default()
    }

    /// Permit writes (turns off read-only enforcement). Use sparingly.
    pub fn allow_writes(mut self) -> Self {
        self.enforce_read_only = false;
        self
    }

    /// Restrict referenced tables to `tables` (case-insensitive).
    pub fn with_allowed_tables<I, S>(mut self, tables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_tables = tables
            .into_iter()
            .map(|t| t.into().to_lowercase())
            .collect();
        self
    }

    /// Inject `LIMIT n` into limit-less queries.
    pub fn with_default_limit(mut self, n: u64) -> Self {
        self.default_limit = Some(n);
        self
    }

    /// Validate `sql`; on success return the (possibly LIMIT-augmented) SQL.
    pub fn check_and_fix(&self, sql: &str) -> Result<String, String> {
        let sql = sql.trim().trim_end_matches(';').trim();
        if sql.is_empty() {
            return Err("empty SQL".into());
        }

        // 1. Parse — rejects syntactically invalid SQL without touching the DB.
        let dialect = GenericDialect {};
        let statements =
            Parser::parse_sql(&dialect, sql).map_err(|e| format!("SQL parse error: {e}"))?;

        // 2. Exactly one statement (blocks stacked-statement injection).
        if statements.len() != 1 {
            return Err(format!(
                "expected exactly one statement, found {}",
                statements.len()
            ));
        }
        let is_query = matches!(statements[0], Statement::Query(_));

        // 3. Read-only.
        if self.enforce_read_only && !is_query {
            return Err("only read-only SELECT queries are permitted".into());
        }

        // 4. Table allowlist (AST-derived, CTE-aware).
        if !self.allowed_tables.is_empty() {
            for t in referenced_tables(&statements[0]) {
                if !self.allowed_tables.contains(&t) {
                    return Err(format!("table `{t}` is not allowed"));
                }
            }
        }

        // 5. Inject a default LIMIT on limit-less queries.
        let mut fixed = sql.to_string();
        if is_query {
            if let Some(limit) = self.default_limit {
                if !has_limit(sql) {
                    fixed = format!("{fixed} LIMIT {limit}");
                }
            }
        }
        Ok(fixed)
    }
}

/// AST visitor that collects real table references (the last identifier of each
/// relation) while tracking CTE names so they aren't mistaken for tables.
#[derive(Default)]
struct TableCollector {
    tables: Vec<String>,
    ctes: HashSet<String>,
}

impl Visitor for TableCollector {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                self.ctes.insert(cte.alias.name.value.to_lowercase());
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        if let Some(ident) = relation.0.last() {
            self.tables.push(ident.value.to_lowercase());
        }
        ControlFlow::Continue(())
    }
}

/// Collect the real (non-CTE) tables a statement references, at any nesting
/// depth — joins, subqueries, set operations, and CTE bodies. Walks the parsed
/// AST rather than scanning tokens, so it cannot be fooled by string contents.
fn referenced_tables(stmt: &Statement) -> Vec<String> {
    let mut collector = TableCollector::default();
    let _ = stmt.visit(&mut collector);
    let TableCollector { tables, ctes } = collector;
    tables.into_iter().filter(|t| !ctes.contains(t)).collect()
}

fn has_limit(sql: &str) -> bool {
    sql.split(|c: char| !c.is_alphanumeric())
        .any(|t| t.eq_ignore_ascii_case("limit"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guards() -> Guardrails {
        Guardrails::read_only()
            .with_allowed_tables(["customers", "orders"])
            .with_default_limit(100)
    }

    #[test]
    fn rejects_non_select() {
        assert!(guards().check_and_fix("DELETE FROM customers").is_err());
        assert!(guards().check_and_fix("DROP TABLE customers").is_err());
    }

    #[test]
    fn allow_writes_permits_dml() {
        let g = Guardrails::default().allow_writes();
        assert!(g
            .check_and_fix("UPDATE customers SET name='x' WHERE id=1")
            .is_ok());
    }

    #[test]
    fn rejects_unknown_table() {
        let e = guards().check_and_fix("SELECT * FROM secrets").unwrap_err();
        assert!(e.contains("secrets"), "{e}");
    }

    #[test]
    fn rejects_stacked_statements() {
        assert!(guards()
            .check_and_fix("SELECT 1 FROM customers; DROP TABLE customers")
            .is_err());
    }

    #[test]
    fn injects_limit_when_missing() {
        let fixed = guards()
            .check_and_fix("SELECT name FROM customers")
            .unwrap();
        assert!(fixed.to_lowercase().contains("limit 100"), "{fixed}");
    }

    #[test]
    fn keeps_existing_limit() {
        let fixed = guards()
            .check_and_fix("SELECT name FROM customers LIMIT 5")
            .unwrap();
        assert_eq!(fixed.to_lowercase().matches("limit").count(), 1, "{fixed}");
    }

    #[test]
    fn allows_join_of_known_tables() {
        let sql = "SELECT c.name FROM customers c JOIN orders o ON o.customer_id = c.id";
        assert!(guards().check_and_fix(sql).is_ok());
    }

    #[test]
    fn catches_disallowed_table_in_subquery() {
        let sql = "SELECT name FROM customers WHERE id IN (SELECT id FROM secrets)";
        assert!(guards().check_and_fix(sql).is_err());
    }

    #[test]
    fn read_only_without_allowlist_allows_any_table() {
        let g = Guardrails::read_only();
        assert!(g.check_and_fix("SELECT * FROM anything").is_ok());
    }

    #[test]
    fn cte_name_is_not_treated_as_a_table() {
        // `recent` is a CTE alias, not a real table; only `orders` must be checked.
        let sql = "WITH recent AS (SELECT * FROM orders) SELECT * FROM recent";
        assert!(guards().check_and_fix(sql).is_ok());
    }

    #[test]
    fn disallowed_table_inside_cte_body_is_caught() {
        let sql = "WITH x AS (SELECT * FROM secrets) SELECT * FROM x";
        assert!(guards().check_and_fix(sql).is_err());
    }

    #[test]
    fn schema_qualified_table_uses_table_name() {
        // `public.customers` → table `customers`, which is allowed.
        assert!(guards()
            .check_and_fix("SELECT * FROM public.customers")
            .is_ok());
    }
}
