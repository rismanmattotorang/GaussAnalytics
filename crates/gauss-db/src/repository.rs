//! Repository traits — the persistence seam.
//!
//! Everything above the storage layer depends on these traits, never on a
//! concrete database. Phase 1 ships [`crate::memory::InMemoryStore`]; Phase 2
//! introduces a `sqlx`-backed implementation behind the *same* traits, so the
//! cutover is a wiring change, not a rewrite. This is the strangler boundary
//! that keeps the migration safe.

use async_trait::async_trait;
use gauss_auth::Session;
use gauss_core::domain::{Database, Table, User};
use gauss_core::error::CoreResult;
use uuid::Uuid;

/// Persistence for [`User`] records.
#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn create_user(&self, user: User, password_hash: String) -> CoreResult<()>;
    async fn user_by_email(&self, email: &str) -> CoreResult<Option<User>>;
    async fn user_by_id(&self, id: Uuid) -> CoreResult<Option<User>>;
    /// List all users (admin views). Excludes password hashes.
    async fn list_users(&self) -> CoreResult<Vec<User>>;
    /// Return the stored Argon2 hash for `email`, if the user exists.
    async fn password_hash(&self, email: &str) -> CoreResult<Option<String>>;
}

/// Persistence for connected [`Database`] sources and their [`Table`] metadata.
#[async_trait]
pub trait DatabaseRepository: Send + Sync {
    async fn create_database(&self, db: Database) -> CoreResult<()>;
    async fn list_databases(&self) -> CoreResult<Vec<Database>>;
    async fn database_by_id(&self, id: Uuid) -> CoreResult<Option<Database>>;
    /// Update a data source's `is_synced` flag.
    async fn set_database_synced(&self, id: Uuid, synced: bool) -> CoreResult<()>;
    async fn upsert_table(&self, table: Table) -> CoreResult<()>;
    async fn table_by_name(&self, database_id: Uuid, name: &str) -> CoreResult<Option<Table>>;
    async fn list_tables(&self, database_id: Uuid) -> CoreResult<Vec<Table>>;
}

/// Persistence for opaque [`Session`]s.
#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn insert_session(&self, session: Session) -> CoreResult<()>;
    async fn session_by_token(&self, token: &str) -> CoreResult<Option<Session>>;
    async fn delete_session(&self, token: &str) -> CoreResult<()>;
}

/// The full application store: the union of all repositories. The server holds
/// one `Arc<dyn Store>`.
pub trait Store: UserRepository + DatabaseRepository + SessionRepository {}
impl<T: UserRepository + DatabaseRepository + SessionRepository> Store for T {}
