//! `sqlx`-backed SQLite implementation of the repository traits.
//!
//! This is the Phase 2 persistent store. It implements exactly the same
//! [`crate::repository`] traits as the in-memory store, so the rest of the
//! platform is unaffected by the swap — the strangler boundary in action. The
//! same pattern extends to Postgres with a `PgPool` and the `postgres` sqlx
//! feature.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gauss_auth::Session;
use gauss_core::domain::{DataSourceKind, Database, Field, Table, User};
use gauss_core::error::{CoreError, CoreResult};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

use crate::repository::{DatabaseRepository, SessionRepository, UserRepository};

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
        DataSourceKind::Generic => "generic",
    }
}

fn kind_from_str(s: &str) -> CoreResult<DataSourceKind> {
    match s {
        "postgres" => Ok(DataSourceKind::Postgres),
        "mysql" => Ok(DataSourceKind::MySql),
        "sqlite" => Ok(DataSourceKind::Sqlite),
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
            fields: vec![Field {
                id: Uuid::new_v4(),
                name: "total".into(),
                field_type: gauss_core::domain::FieldType::Float,
            }],
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
