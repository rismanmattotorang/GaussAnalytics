//! In-memory implementation of the repository traits.
//!
//! Used for Phase 1 development and for tests across the workspace. It is fully
//! thread-safe (`RwLock`-guarded maps) and behaves like the future `sqlx`
//! implementation from the caller's perspective.

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use async_trait::async_trait;
use gauss_auth::{Permission, Session};
use gauss_core::domain::{Database, Table, User};
use gauss_core::error::{CoreError, CoreResult};
use uuid::Uuid;

use crate::repository::{
    ApiKeyInfo, ApiKeyRecord, ApiKeyRepository, DatabaseRepository, GrantRepository,
    SessionRepository, UserRepository,
};

/// A thread-safe, process-local application store.
#[derive(Default)]
pub struct InMemoryStore {
    users: RwLock<HashMap<Uuid, User>>,
    password_hashes: RwLock<HashMap<String, String>>, // email -> hash
    databases: RwLock<HashMap<Uuid, Database>>,
    tables: RwLock<HashMap<(Uuid, String), Table>>, // (database_id, name) -> table
    sessions: RwLock<HashMap<String, Session>>,     // token -> session
    grants: RwLock<HashMap<Uuid, HashSet<Permission>>>, // user_id -> permissions
    api_keys: RwLock<HashMap<Uuid, (ApiKeyRecord, bool)>>, // id -> (record, revoked)
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Map a poisoned lock into a `CoreError` instead of panicking.
fn lock_err<T>(_e: T) -> CoreError {
    CoreError::Storage("lock poisoned".into())
}

#[async_trait]
impl UserRepository for InMemoryStore {
    async fn create_user(&self, user: User, password_hash: String) -> CoreResult<()> {
        let email = user.email.clone();
        self.users.write().map_err(lock_err)?.insert(user.id, user);
        self.password_hashes
            .write()
            .map_err(lock_err)?
            .insert(email, password_hash);
        Ok(())
    }

    async fn user_by_email(&self, email: &str) -> CoreResult<Option<User>> {
        Ok(self
            .users
            .read()
            .map_err(lock_err)?
            .values()
            .find(|u| u.email == email)
            .cloned())
    }

    async fn user_by_id(&self, id: Uuid) -> CoreResult<Option<User>> {
        Ok(self.users.read().map_err(lock_err)?.get(&id).cloned())
    }

    async fn password_hash(&self, email: &str) -> CoreResult<Option<String>> {
        Ok(self
            .password_hashes
            .read()
            .map_err(lock_err)?
            .get(email)
            .cloned())
    }

    async fn list_users(&self) -> CoreResult<Vec<User>> {
        let mut v: Vec<User> = self
            .users
            .read()
            .map_err(lock_err)?
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| a.email.cmp(&b.email));
        Ok(v)
    }
}

#[async_trait]
impl DatabaseRepository for InMemoryStore {
    async fn create_database(&self, db: Database) -> CoreResult<()> {
        self.databases.write().map_err(lock_err)?.insert(db.id, db);
        Ok(())
    }

    async fn list_databases(&self) -> CoreResult<Vec<Database>> {
        let mut v: Vec<Database> = self
            .databases
            .read()
            .map_err(lock_err)?
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(v)
    }

    async fn database_by_id(&self, id: Uuid) -> CoreResult<Option<Database>> {
        Ok(self.databases.read().map_err(lock_err)?.get(&id).cloned())
    }

    async fn set_database_synced(&self, id: Uuid, synced: bool) -> CoreResult<()> {
        if let Some(db) = self.databases.write().map_err(lock_err)?.get_mut(&id) {
            db.is_synced = synced;
        }
        Ok(())
    }

    async fn upsert_table(&self, table: Table) -> CoreResult<()> {
        let key = (table.database_id, table.name.clone());
        self.tables.write().map_err(lock_err)?.insert(key, table);
        Ok(())
    }

    async fn table_by_name(&self, database_id: Uuid, name: &str) -> CoreResult<Option<Table>> {
        Ok(self
            .tables
            .read()
            .map_err(lock_err)?
            .get(&(database_id, name.to_string()))
            .cloned())
    }

    async fn list_tables(&self, database_id: Uuid) -> CoreResult<Vec<Table>> {
        let mut v: Vec<Table> = self
            .tables
            .read()
            .map_err(lock_err)?
            .values()
            .filter(|t| t.database_id == database_id)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(v)
    }
}

#[async_trait]
impl SessionRepository for InMemoryStore {
    async fn insert_session(&self, session: Session) -> CoreResult<()> {
        self.sessions
            .write()
            .map_err(lock_err)?
            .insert(session.token.clone(), session);
        Ok(())
    }

    async fn session_by_token(&self, token: &str) -> CoreResult<Option<Session>> {
        Ok(self.sessions.read().map_err(lock_err)?.get(token).cloned())
    }

    async fn delete_session(&self, token: &str) -> CoreResult<()> {
        self.sessions.write().map_err(lock_err)?.remove(token);
        Ok(())
    }
}

#[async_trait]
impl GrantRepository for InMemoryStore {
    async fn grant(&self, user_id: Uuid, perm: Permission) -> CoreResult<()> {
        self.grants
            .write()
            .map_err(lock_err)?
            .entry(user_id)
            .or_default()
            .insert(perm);
        Ok(())
    }

    async fn revoke(&self, user_id: Uuid, perm: Permission) -> CoreResult<()> {
        if let Some(set) = self.grants.write().map_err(lock_err)?.get_mut(&user_id) {
            set.remove(&perm);
        }
        Ok(())
    }

    async fn grants_for(&self, user_id: Uuid) -> CoreResult<Vec<Permission>> {
        Ok(self
            .grants
            .read()
            .map_err(lock_err)?
            .get(&user_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default())
    }
}

#[async_trait]
impl ApiKeyRepository for InMemoryStore {
    async fn create_api_key(&self, record: ApiKeyRecord) -> CoreResult<()> {
        self.api_keys
            .write()
            .map_err(lock_err)?
            .insert(record.id, (record, false));
        Ok(())
    }

    async fn api_key_user(&self, key_hash: &str) -> CoreResult<Option<Uuid>> {
        Ok(self
            .api_keys
            .read()
            .map_err(lock_err)?
            .values()
            .find(|(r, revoked)| !*revoked && r.key_hash == key_hash)
            .map(|(r, _)| r.user_id))
    }

    async fn list_api_keys(&self, user_id: Uuid) -> CoreResult<Vec<ApiKeyInfo>> {
        let mut v: Vec<ApiKeyInfo> = self
            .api_keys
            .read()
            .map_err(lock_err)?
            .values()
            .filter(|(r, _)| r.user_id == user_id)
            .map(|(r, revoked)| ApiKeyInfo {
                id: r.id,
                name: r.name.clone(),
                created_at: r.created_at,
                revoked: *revoked,
            })
            .collect();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(v)
    }

    async fn revoke_api_key(&self, id: Uuid) -> CoreResult<()> {
        if let Some((_, revoked)) = self.api_keys.write().map_err(lock_err)?.get_mut(&id) {
            *revoked = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use gauss_core::domain::DataSourceKind;

    #[tokio::test]
    async fn users_round_trip() {
        let store = InMemoryStore::new();
        let user = User {
            id: Uuid::new_v4(),
            email: "ada@example.com".into(),
            display_name: "Ada".into(),
            is_admin: true,
            created_at: Utc::now(),
        };
        store
            .create_user(user.clone(), "phc$hash".into())
            .await
            .unwrap();
        assert_eq!(
            store.user_by_email("ada@example.com").await.unwrap(),
            Some(user.clone())
        );
        assert_eq!(
            store.password_hash("ada@example.com").await.unwrap(),
            Some("phc$hash".into())
        );
    }

    #[tokio::test]
    async fn databases_and_tables() {
        let store = InMemoryStore::new();
        let db = Database {
            id: Uuid::new_v4(),
            name: "warehouse".into(),
            kind: DataSourceKind::Postgres,
            is_synced: true,
            connection_uri: None,
            created_at: Utc::now(),
        };
        store.create_database(db.clone()).await.unwrap();
        let table = Table {
            id: Uuid::new_v4(),
            database_id: db.id,
            name: "orders".into(),
            fields: vec![],
        };
        store.upsert_table(table.clone()).await.unwrap();
        assert_eq!(
            store.table_by_name(db.id, "orders").await.unwrap(),
            Some(table)
        );
        assert_eq!(store.list_databases().await.unwrap().len(), 1);
    }
}
