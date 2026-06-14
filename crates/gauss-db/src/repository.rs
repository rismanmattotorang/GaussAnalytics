//! Repository traits — the persistence seam.
//!
//! Everything above the storage layer depends on these traits, never on a
//! concrete database. Phase 1 ships [`crate::memory::InMemoryStore`]; Phase 2
//! introduces a `sqlx`-backed implementation behind the *same* traits, so the
//! cutover is a wiring change, not a rewrite. This is the strangler boundary
//! that keeps the migration safe.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gauss_auth::{Permission, Session};
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

/// Persistence for persisted, per-user permission grants.
#[async_trait]
pub trait GrantRepository: Send + Sync {
    async fn grant(&self, user_id: Uuid, perm: Permission) -> CoreResult<()>;
    async fn revoke(&self, user_id: Uuid, perm: Permission) -> CoreResult<()>;
    async fn grants_for(&self, user_id: Uuid) -> CoreResult<Vec<Permission>>;
}

/// A stored API key (only the hash is persisted).
#[derive(Debug, Clone)]
pub struct ApiKeyRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub key_hash: String,
    pub created_at: DateTime<Utc>,
}

/// Metadata about an API key, safe to return to clients (no hash).
#[derive(Debug, Clone)]
pub struct ApiKeyInfo {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub revoked: bool,
}

/// Persistence for DB-backed, rotatable API keys.
#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    async fn create_api_key(&self, record: ApiKeyRecord) -> CoreResult<()>;
    /// The owning user of an *active* (non-revoked) key with this hash.
    async fn api_key_user(&self, key_hash: &str) -> CoreResult<Option<Uuid>>;
    async fn list_api_keys(&self, user_id: Uuid) -> CoreResult<Vec<ApiKeyInfo>>;
    async fn revoke_api_key(&self, id: Uuid) -> CoreResult<()>;
}

/// The full application store: the union of all repositories. The server holds
/// one `Arc<dyn Store>`.
pub trait Store:
    UserRepository + DatabaseRepository + SessionRepository + GrantRepository + ApiKeyRepository
{
}
impl<T> Store for T where
    T: UserRepository + DatabaseRepository + SessionRepository + GrantRepository + ApiKeyRepository
{
}
