//! `gauss-db` — the application metadata store.
//!
//! Defines the [`repository`] traits the rest of the platform depends on, plus
//! an [`memory`] in-memory implementation (Phase 1 / tests) and a `sqlx`-backed
//! [`sqlite`] implementation (Phase 2). Both satisfy the same traits, so the
//! rest of the platform is agnostic to which is wired in.

#![forbid(unsafe_code)]

pub mod memory;
pub mod mysql;
pub mod postgres;
pub mod repository;
pub mod sqlite;

pub use memory::InMemoryStore;
pub use mysql::MySqlStore;
pub use postgres::PgStore;
pub use repository::{
    ApiKeyInfo, ApiKeyRecord, ApiKeyRepository, DatabaseRepository, GrantRepository,
    SessionRepository, Store, UserRepository,
};
pub use sqlite::{run_migrations, SqliteStore};

use gauss_core::error::CoreResult;

/// Apply migrations to the database at `url`, dispatching by scheme
/// (`postgres*` / `mysql*` / `sqlite*`). Used by `gaussctl migrate`.
pub async fn migrate_url(url: &str) -> CoreResult<()> {
    if url.starts_with("postgres") {
        postgres::PgStore::connect(url).await.map(|_| ())
    } else if url.starts_with("mysql") {
        mysql::MySqlStore::connect(url).await.map(|_| ())
    } else {
        sqlite::migrate_url(url).await
    }
}
