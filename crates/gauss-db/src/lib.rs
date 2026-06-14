//! `gauss-db` — the application metadata store.
//!
//! Defines the [`repository`] traits the rest of the platform depends on, plus
//! an [`memory`] in-memory implementation (Phase 1 / tests) and a `sqlx`-backed
//! [`sqlite`] implementation (Phase 2). Both satisfy the same traits, so the
//! rest of the platform is agnostic to which is wired in.

#![forbid(unsafe_code)]

pub mod memory;
pub mod repository;
pub mod sqlite;

pub use memory::InMemoryStore;
pub use repository::{DatabaseRepository, SessionRepository, Store, UserRepository};
pub use sqlite::{migrate_url, run_migrations, SqliteStore};
