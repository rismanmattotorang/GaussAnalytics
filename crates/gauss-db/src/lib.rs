//! `gauss-db` — the application metadata store.
//!
//! Defines the [`repository`] traits the rest of the platform depends on, plus
//! an [`memory`] in-memory implementation for Phase 1 and tests. The Phase 2
//! `sqlx` implementation slots in behind the same traits.

#![forbid(unsafe_code)]

pub mod memory;
pub mod repository;

pub use memory::InMemoryStore;
pub use repository::{DatabaseRepository, SessionRepository, Store, UserRepository};
