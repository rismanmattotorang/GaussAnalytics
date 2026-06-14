//! Read-only guardrails for NL2SQL output.
//!
//! Natural-language-to-SQL is delegated to Gaussian's service, but
//! GaussAnalytics never trusts model output blindly. Before a candidate query
//! can run, it must pass these lexical guardrails: a single, read-only
//! statement. (Phase 2 strengthens this with full SQL parsing and compilation
//! through `gauss-query`, plus per-user permission enforcement.)

use gauss_core::error::{CoreError, CoreResult};

/// Statement keywords that mutate data or schema — never allowed from NL2SQL.
const FORBIDDEN_KEYWORDS: &[&str] = &[
    "insert",
    "update",
    "delete",
    "drop",
    "alter",
    "create",
    "truncate",
    "grant",
    "revoke",
    "attach",
    "detach",
    "pragma",
    "vacuum",
    "replace",
    "merge",
    "call",
    "execute",
    "exec",
    "commit",
    "rollback",
    "savepoint",
    "set",
];

/// Validate that `sql` is a single, read-only statement.
///
/// Returns the normalized (trimmed, trailing-semicolon-stripped) SQL on success.
pub fn ensure_read_only(sql: &str) -> CoreResult<String> {
    let trimmed = sql.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidQuery("empty NL2SQL candidate".into()));
    }

    // Reject statement batching: any interior semicolon means multiple statements.
    if trimmed.contains(';') {
        return Err(CoreError::InvalidQuery(
            "NL2SQL candidate contains multiple statements".into(),
        ));
    }

    // Must be a read query.
    let lowered = trimmed.to_ascii_lowercase();
    let starts_ok = lowered.starts_with("select") || lowered.starts_with("with");
    if !starts_ok {
        return Err(CoreError::InvalidQuery(
            "NL2SQL candidate is not a SELECT/WITH query".into(),
        ));
    }

    // Defense in depth: reject mutating keywords appearing as whole tokens.
    for tok in lowered.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if FORBIDDEN_KEYWORDS.contains(&tok) {
            return Err(CoreError::InvalidQuery(format!(
                "NL2SQL candidate contains forbidden keyword `{tok}`"
            )));
        }
    }

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_plain_select() {
        assert_eq!(
            ensure_read_only("  SELECT a, b FROM t WHERE a > 1 ;  ").unwrap(),
            "SELECT a, b FROM t WHERE a > 1"
        );
    }

    #[test]
    fn allows_cte() {
        assert!(ensure_read_only("WITH x AS (SELECT 1) SELECT * FROM x").is_ok());
    }

    #[test]
    fn rejects_mutations() {
        assert!(ensure_read_only("DELETE FROM users").is_err());
        assert!(ensure_read_only("DROP TABLE users").is_err());
    }

    #[test]
    fn rejects_statement_batching() {
        assert!(ensure_read_only("SELECT 1; DROP TABLE users").is_err());
    }

    #[test]
    fn rejects_hidden_mutation_keyword() {
        // A SELECT that smuggles a forbidden token is rejected.
        assert!(ensure_read_only("SELECT 1 FROM t; INSERT INTO t VALUES (1)").is_err());
    }
}
