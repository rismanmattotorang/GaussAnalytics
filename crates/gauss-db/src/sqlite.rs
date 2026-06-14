//! `sqlx`-backed SQLite implementation of the repository traits.
//!
//! This is the Phase 2 persistent store. It implements exactly the same
//! [`crate::repository`] traits as the in-memory store, so the rest of the
//! platform is unaffected by the swap — the strangler boundary in action. The
//! same pattern extends to Postgres with a `PgPool` and the `postgres` sqlx
//! feature.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gauss_auth::{Permission, Session};
use gauss_core::domain::{DataSourceKind, Database, Field, Table, User};
use gauss_core::error::{CoreError, CoreResult};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

use crate::repository::{
    ApiKeyInfo, ApiKeyRecord, ApiKeyRepository, ContentRecord, ContentRepository,
    DatabaseRepository, GrantRepository, SessionRepository, UserRepository,
};

/// Storage encoding of a permission scope: the UUID string, or '' for unscoped.
pub(crate) fn scope_to_str(scope: Option<Uuid>) -> String {
    scope.map(|u| u.to_string()).unwrap_or_default()
}

/// Reconstruct the optional scope UUID from its stored string form.
pub(crate) fn scope_from_str(s: &str) -> Option<Uuid> {
    if s.is_empty() {
        None
    } else {
        Uuid::parse_str(s).ok()
    }
}

/// A persistent application store backed by SQLite.
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Connect to the application database (creating the file if missing) and
    /// run pending migrations.
    pub async fn connect(url: &str) -> CoreResult<Self> {
        let opts = SqliteConnectOptions::from_str(url)
            .map_err(storage)?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(opts)
            .await
            .map_err(storage)?;
        run_migrations(&pool).await?;
        Ok(Self { pool })
    }

    /// Wrap an existing pool without running migrations.
    pub fn from_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Run all embedded migrations against `pool`.
pub async fn run_migrations(pool: &SqlitePool) -> CoreResult<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|e| CoreError::Storage(format!("migration failed: {e}")))
}

/// Connect to a SQLite database URL (creating the file if missing) and run all
/// pending migrations. Used by `gaussctl migrate`.
pub async fn migrate_url(url: &str) -> CoreResult<()> {
    if !url.starts_with("sqlite") {
        return Err(CoreError::Config(format!(
            "migrations currently support sqlite URLs only; got {url:?}"
        )));
    }
    let opts = SqliteConnectOptions::from_str(url)
        .map_err(storage)?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .map_err(storage)?;
    run_migrations(&pool).await
}

fn storage<E: std::fmt::Display>(e: E) -> CoreError {
    CoreError::Storage(e.to_string())
}

fn parse_uuid(s: &str) -> CoreResult<Uuid> {
    Uuid::parse_str(s).map_err(|e| CoreError::Storage(format!("invalid uuid {s:?}: {e}")))
}

fn parse_ts(s: &str) -> CoreResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| CoreError::Storage(format!("invalid timestamp {s:?}: {e}")))
}

fn kind_to_str(k: DataSourceKind) -> &'static str {
    match k {
        DataSourceKind::Postgres => "postgres",
        DataSourceKind::MySql => "mysql",
        DataSourceKind::Sqlite => "sqlite",
        DataSourceKind::BigQuery => "bigquery",
        DataSourceKind::Snowflake => "snowflake",
        DataSourceKind::ClickHouse => "clickhouse",
        DataSourceKind::Generic => "generic",
    }
}

fn kind_from_str(s: &str) -> CoreResult<DataSourceKind> {
    match s {
        "postgres" => Ok(DataSourceKind::Postgres),
        "mysql" => Ok(DataSourceKind::MySql),
        "sqlite" => Ok(DataSourceKind::Sqlite),
        "bigquery" => Ok(DataSourceKind::BigQuery),
        "snowflake" => Ok(DataSourceKind::Snowflake),
        "clickhouse" => Ok(DataSourceKind::ClickHouse),
        "generic" => Ok(DataSourceKind::Generic),
        other => Err(CoreError::Storage(format!(
            "unknown data source kind {other:?}"
        ))),
    }
}

#[async_trait]
impl UserRepository for SqliteStore {
    async fn create_user(&self, user: User, password_hash: String) -> CoreResult<()> {
        sqlx::query(
            "INSERT INTO users (id, email, display_name, is_admin, password_hash, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user.id.to_string())
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(user.is_admin as i64)
        .bind(password_hash)
        .bind(user.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn user_by_email(&self, email: &str) -> CoreResult<Option<User>> {
        let row = sqlx::query(
            "SELECT id, email, display_name, is_admin, created_at FROM users WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        row.map(user_from_row).transpose()
    }

    async fn user_by_id(&self, id: Uuid) -> CoreResult<Option<User>> {
        let row = sqlx::query(
            "SELECT id, email, display_name, is_admin, created_at FROM users WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        row.map(user_from_row).transpose()
    }

    async fn password_hash(&self, email: &str) -> CoreResult<Option<String>> {
        let row = sqlx::query("SELECT password_hash FROM users WHERE email = ?")
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;
        row.map(|r| r.try_get::<String, _>("password_hash").map_err(storage))
            .transpose()
    }

    async fn list_users(&self) -> CoreResult<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, email, display_name, is_admin, created_at FROM users ORDER BY email",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter().map(user_from_row).collect()
    }
}

fn user_from_row(row: sqlx::sqlite::SqliteRow) -> CoreResult<User> {
    Ok(User {
        id: parse_uuid(&row.try_get::<String, _>("id").map_err(storage)?)?,
        email: row.try_get("email").map_err(storage)?,
        display_name: row.try_get("display_name").map_err(storage)?,
        is_admin: row.try_get::<i64, _>("is_admin").map_err(storage)? != 0,
        created_at: parse_ts(&row.try_get::<String, _>("created_at").map_err(storage)?)?,
    })
}

#[async_trait]
impl DatabaseRepository for SqliteStore {
    async fn create_database(&self, db: Database) -> CoreResult<()> {
        sqlx::query(
            "INSERT INTO data_sources (id, name, kind, is_synced, connection_uri, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(db.id.to_string())
        .bind(&db.name)
        .bind(kind_to_str(db.kind))
        .bind(db.is_synced as i64)
        .bind(db.connection_uri)
        .bind(db.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn list_databases(&self) -> CoreResult<Vec<Database>> {
        let rows = sqlx::query(
            "SELECT id, name, kind, is_synced, connection_uri, created_at FROM data_sources ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter().map(database_from_row).collect()
    }

    async fn database_by_id(&self, id: Uuid) -> CoreResult<Option<Database>> {
        let row = sqlx::query(
            "SELECT id, name, kind, is_synced, connection_uri, created_at FROM data_sources WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        row.map(database_from_row).transpose()
    }

    async fn set_database_synced(&self, id: Uuid, synced: bool) -> CoreResult<()> {
        sqlx::query("UPDATE data_sources SET is_synced = ? WHERE id = ?")
            .bind(synced as i64)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn upsert_table(&self, table: Table) -> CoreResult<()> {
        let fields_json =
            serde_json::to_string(&table.fields).map_err(|e| CoreError::Storage(e.to_string()))?;
        sqlx::query(
            "INSERT INTO source_tables (id, database_id, name, fields_json) VALUES (?, ?, ?, ?) \
             ON CONFLICT (database_id, name) DO UPDATE SET fields_json = excluded.fields_json",
        )
        .bind(table.id.to_string())
        .bind(table.database_id.to_string())
        .bind(&table.name)
        .bind(fields_json)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn table_by_name(&self, database_id: Uuid, name: &str) -> CoreResult<Option<Table>> {
        let row = sqlx::query(
            "SELECT id, database_id, name, fields_json FROM source_tables WHERE database_id = ? AND name = ?",
        )
        .bind(database_id.to_string())
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        row.map(table_from_row).transpose()
    }

    async fn list_tables(&self, database_id: Uuid) -> CoreResult<Vec<Table>> {
        let rows = sqlx::query(
            "SELECT id, database_id, name, fields_json FROM source_tables WHERE database_id = ? ORDER BY name",
        )
        .bind(database_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter().map(table_from_row).collect()
    }
}

fn database_from_row(row: sqlx::sqlite::SqliteRow) -> CoreResult<Database> {
    Ok(Database {
        id: parse_uuid(&row.try_get::<String, _>("id").map_err(storage)?)?,
        name: row.try_get("name").map_err(storage)?,
        kind: kind_from_str(&row.try_get::<String, _>("kind").map_err(storage)?)?,
        is_synced: row.try_get::<i64, _>("is_synced").map_err(storage)? != 0,
        connection_uri: row.try_get("connection_uri").map_err(storage)?,
        created_at: parse_ts(&row.try_get::<String, _>("created_at").map_err(storage)?)?,
    })
}

fn table_from_row(row: sqlx::sqlite::SqliteRow) -> CoreResult<Table> {
    let fields: Vec<Field> =
        serde_json::from_str(&row.try_get::<String, _>("fields_json").map_err(storage)?)
            .map_err(|e| CoreError::Storage(e.to_string()))?;
    Ok(Table {
        id: parse_uuid(&row.try_get::<String, _>("id").map_err(storage)?)?,
        database_id: parse_uuid(&row.try_get::<String, _>("database_id").map_err(storage)?)?,
        name: row.try_get("name").map_err(storage)?,
        fields,
    })
}

#[async_trait]
impl SessionRepository for SqliteStore {
    async fn insert_session(&self, session: Session) -> CoreResult<()> {
        sqlx::query(
            "INSERT INTO sessions (token, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&session.token)
        .bind(session.user_id.to_string())
        .bind(session.created_at.to_rfc3339())
        .bind(session.expires_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn session_by_token(&self, token: &str) -> CoreResult<Option<Session>> {
        let row = sqlx::query(
            "SELECT token, user_id, created_at, expires_at FROM sessions WHERE token = ?",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        match row {
            None => Ok(None),
            Some(r) => Ok(Some(Session {
                token: r.try_get("token").map_err(storage)?,
                user_id: parse_uuid(&r.try_get::<String, _>("user_id").map_err(storage)?)?,
                created_at: parse_ts(&r.try_get::<String, _>("created_at").map_err(storage)?)?,
                expires_at: parse_ts(&r.try_get::<String, _>("expires_at").map_err(storage)?)?,
            })),
        }
    }

    async fn delete_session(&self, token: &str) -> CoreResult<()> {
        sqlx::query("DELETE FROM sessions WHERE token = ?")
            .bind(token)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }
}

#[async_trait]
impl GrantRepository for SqliteStore {
    async fn grant(&self, user_id: Uuid, perm: Permission) -> CoreResult<()> {
        let (kind, scope) = perm.to_parts();
        sqlx::query(
            "INSERT OR IGNORE INTO permission_grants (user_id, kind, scope) VALUES (?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(kind)
        .bind(scope_to_str(scope))
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn revoke(&self, user_id: Uuid, perm: Permission) -> CoreResult<()> {
        let (kind, scope) = perm.to_parts();
        sqlx::query("DELETE FROM permission_grants WHERE user_id = ? AND kind = ? AND scope = ?")
            .bind(user_id.to_string())
            .bind(kind)
            .bind(scope_to_str(scope))
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn grants_for(&self, user_id: Uuid) -> CoreResult<Vec<Permission>> {
        let rows = sqlx::query("SELECT kind, scope FROM permission_grants WHERE user_id = ?")
            .bind(user_id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(storage)?;
        let mut out = Vec::new();
        for r in &rows {
            let kind: String = r.try_get("kind").map_err(storage)?;
            let scope: String = r.try_get("scope").map_err(storage)?;
            if let Some(p) = Permission::from_parts(&kind, scope_from_str(&scope)) {
                out.push(p);
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl ApiKeyRepository for SqliteStore {
    async fn create_api_key(&self, record: ApiKeyRecord) -> CoreResult<()> {
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, name, key_hash, created_at, revoked) \
             VALUES (?, ?, ?, ?, ?, 0)",
        )
        .bind(record.id.to_string())
        .bind(record.user_id.to_string())
        .bind(&record.name)
        .bind(&record.key_hash)
        .bind(record.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn api_key_user(&self, key_hash: &str) -> CoreResult<Option<Uuid>> {
        let row = sqlx::query("SELECT user_id FROM api_keys WHERE key_hash = ? AND revoked = 0")
            .bind(key_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;
        match row {
            None => Ok(None),
            Some(r) => Ok(Some(parse_uuid(
                &r.try_get::<String, _>("user_id").map_err(storage)?,
            )?)),
        }
    }

    async fn list_api_keys(&self, user_id: Uuid) -> CoreResult<Vec<ApiKeyInfo>> {
        let rows = sqlx::query(
            "SELECT id, name, created_at, revoked FROM api_keys WHERE user_id = ? ORDER BY created_at",
        )
        .bind(user_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            out.push(ApiKeyInfo {
                id: parse_uuid(&r.try_get::<String, _>("id").map_err(storage)?)?,
                name: r.try_get("name").map_err(storage)?,
                created_at: parse_ts(&r.try_get::<String, _>("created_at").map_err(storage)?)?,
                revoked: r.try_get::<i64, _>("revoked").map_err(storage)? != 0,
            });
        }
        Ok(out)
    }

    async fn revoke_api_key(&self, id: Uuid) -> CoreResult<()> {
        sqlx::query("UPDATE api_keys SET revoked = 1 WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }
}

#[async_trait]
impl ContentRepository for SqliteStore {
    async fn put_content(&self, record: ContentRecord) -> CoreResult<()> {
        sqlx::query(
            "INSERT INTO content (id, kind, collection_id, name, body_json, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (id) DO UPDATE SET \
               kind = excluded.kind, collection_id = excluded.collection_id, \
               name = excluded.name, body_json = excluded.body_json",
        )
        .bind(record.id.to_string())
        .bind(&record.kind)
        .bind(record.collection_id.map(|c| c.to_string()))
        .bind(&record.name)
        .bind(&record.body_json)
        .bind(record.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn get_content(&self, id: Uuid) -> CoreResult<Option<ContentRecord>> {
        let row = sqlx::query(
            "SELECT id, kind, collection_id, name, body_json, created_at FROM content WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        row.map(content_from_row).transpose()
    }

    async fn list_content(&self, kind: &str) -> CoreResult<Vec<ContentRecord>> {
        let rows = sqlx::query(
            "SELECT id, kind, collection_id, name, body_json, created_at FROM content \
             WHERE kind = ? ORDER BY created_at DESC",
        )
        .bind(kind)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter().map(content_from_row).collect()
    }

    async fn delete_content(&self, id: Uuid) -> CoreResult<()> {
        sqlx::query("DELETE FROM content WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }
}

fn content_from_row(row: sqlx::sqlite::SqliteRow) -> CoreResult<ContentRecord> {
    let collection_id: Option<String> = row.try_get("collection_id").map_err(storage)?;
    Ok(ContentRecord {
        id: parse_uuid(&row.try_get::<String, _>("id").map_err(storage)?)?,
        kind: row.try_get("kind").map_err(storage)?,
        collection_id: collection_id.as_deref().map(parse_uuid).transpose()?,
        name: row.try_get("name").map_err(storage)?,
        body_json: row.try_get("body_json").map_err(storage)?,
        created_at: parse_ts(&row.try_get::<String, _>("created_at").map_err(storage)?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn store() -> SqliteStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        run_migrations(&pool).await.unwrap();
        SqliteStore::from_pool(pool)
    }

    #[tokio::test]
    async fn user_persist_and_lookup() {
        let s = store().await;
        let user = User {
            id: Uuid::new_v4(),
            email: "ada@example.com".into(),
            display_name: "Ada".into(),
            is_admin: true,
            created_at: Utc::now(),
        };
        s.create_user(user.clone(), "phc$hash".into())
            .await
            .unwrap();

        let fetched = s.user_by_email("ada@example.com").await.unwrap().unwrap();
        assert_eq!(fetched.id, user.id);
        assert!(fetched.is_admin);
        assert_eq!(
            s.password_hash("ada@example.com").await.unwrap(),
            Some("phc$hash".into())
        );
    }

    #[tokio::test]
    async fn database_and_table_persist() {
        let s = store().await;
        let db = Database {
            id: Uuid::new_v4(),
            name: "warehouse".into(),
            kind: DataSourceKind::Postgres,
            is_synced: true,
            connection_uri: Some("sqlite://warehouse.db".into()),
            created_at: Utc::now(),
        };
        s.create_database(db.clone()).await.unwrap();

        let table = Table {
            id: Uuid::new_v4(),
            database_id: db.id,
            name: "orders".into(),
            fields: vec![Field::new("total", gauss_core::domain::FieldType::Float)],
        };
        s.upsert_table(table.clone()).await.unwrap();
        // Upsert again to confirm ON CONFLICT works.
        s.upsert_table(table.clone()).await.unwrap();

        let fetched = s.table_by_name(db.id, "orders").await.unwrap().unwrap();
        assert_eq!(fetched.fields.len(), 1);
        assert_eq!(fetched.fields[0].name, "total");
        assert_eq!(s.list_databases().await.unwrap().len(), 1);
        assert_eq!(s.list_tables(db.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn grants_persist_and_revoke() {
        use gauss_auth::Permission;
        let s = store().await;
        let uid = Uuid::new_v4();
        let db = Uuid::new_v4();
        s.grant(uid, Permission::ReadDatabase { database_id: db })
            .await
            .unwrap();
        s.grant(uid, Permission::CreateContent).await.unwrap();
        // idempotent
        s.grant(uid, Permission::CreateContent).await.unwrap();
        let mut g = s.grants_for(uid).await.unwrap();
        g.sort_by_key(|p| format!("{p:?}"));
        assert_eq!(g.len(), 2);
        s.revoke(uid, Permission::CreateContent).await.unwrap();
        assert_eq!(s.grants_for(uid).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn api_keys_lifecycle() {
        use crate::repository::ApiKeyRecord;
        let s = store().await;
        let uid = Uuid::new_v4();
        s.create_api_key(ApiKeyRecord {
            id: Uuid::new_v4(),
            user_id: uid,
            name: "ci".into(),
            key_hash: "abc123".into(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
        assert_eq!(s.api_key_user("abc123").await.unwrap(), Some(uid));
        let keys = s.list_api_keys(uid).await.unwrap();
        assert_eq!(keys.len(), 1);
        assert!(!keys[0].revoked);
        s.revoke_api_key(keys[0].id).await.unwrap();
        assert_eq!(s.api_key_user("abc123").await.unwrap(), None);
        assert!(s.list_api_keys(uid).await.unwrap()[0].revoked);
    }

    #[tokio::test]
    async fn content_upsert_list_delete() {
        use crate::repository::ContentRecord;
        let s = store().await;
        let id = Uuid::new_v4();
        s.put_content(ContentRecord {
            id,
            kind: "card".into(),
            collection_id: None,
            name: "Revenue".into(),
            body_json: r#"{"q":1}"#.into(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
        // Upsert (rename) keeps the same id.
        s.put_content(ContentRecord {
            id,
            kind: "card".into(),
            collection_id: None,
            name: "Revenue v2".into(),
            body_json: r#"{"q":2}"#.into(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

        let got = s.get_content(id).await.unwrap().unwrap();
        assert_eq!(got.name, "Revenue v2");
        assert_eq!(s.list_content("card").await.unwrap().len(), 1);
        assert!(s.list_content("dashboard").await.unwrap().is_empty());
        s.delete_content(id).await.unwrap();
        assert!(s.get_content(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_lifecycle() {
        let s = store().await;
        let session = Session::new(Uuid::new_v4(), 60, Utc::now());
        let token = session.token.clone();
        s.insert_session(session).await.unwrap();
        assert!(s.session_by_token(&token).await.unwrap().is_some());
        s.delete_session(&token).await.unwrap();
        assert!(s.session_by_token(&token).await.unwrap().is_none());
    }
}
