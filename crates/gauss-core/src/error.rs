//! Shared error type for the GaussAnalytics workspace.

use thiserror::Error;

/// The canonical error type returned across GaussAnalytics crates.
///
/// Crates may convert their internal failures into a `CoreError` so the server
/// can map a single error enum onto HTTP responses.
#[derive(Debug, Error)]
pub enum CoreError {
    /// A query failed validation before compilation (e.g. unknown field).
    #[error("invalid query: {0}")]
    InvalidQuery(String),

    /// Compilation of a [`crate::gql::Query`] to SQL failed.
    #[error("query compilation failed: {0}")]
    Compilation(String),

    /// The requested entity does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The caller is not permitted to perform the action.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// Authentication failed or no valid session was presented.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// Configuration was missing or invalid.
    #[error("configuration error: {0}")]
    Config(String),

    /// A downstream integration (MCP gateway, NL2SQL) failed.
    #[error("integration error: {0}")]
    Integration(String),

    /// Persistence-layer failure.
    #[error("storage error: {0}")]
    Storage(String),

    /// Catch-all for unexpected internal failures.
    #[error("internal error: {0}")]
    Internal(String),
}

/// Convenience alias for results that fail with [`CoreError`].
pub type CoreResult<T> = Result<T, CoreError>;
