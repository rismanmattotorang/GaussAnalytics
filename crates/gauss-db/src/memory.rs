//! In-memory implementation of the repository traits.
//!
//! Used for Phase 1 development and for tests across the workspace. It is fully
//! thread-safe (`RwLock`-guarded maps) and behaves like the future `sqlx`
//! implementation from the caller's perspective.

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use gauss_auth::Session;
use gauss_core::domain::{Database, Table, User};
use gauss_core::error::{CoreError, CoreResult};
use uuid::Uuid;

use crate::repository::{DatabaseRepository, SessionRepository, UserRepository};

/// A thread-safe, process-local application store.
#[derive(Default)]
pub struct InMemoryStore {
    users: RwLock<HashMap<Uuid, User>>,
    password_hashes: RwLock<HashMap<String, String>>, // email -> hash
    databases: RwLock<HashMap<Uuid, Database>>,
    tables: RwLock<HashMap<(Uuid, String), Table>>, // (database_id, name) -> table
    sessions: RwLock<HashMap<String, Session>>,     // token -> session
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
