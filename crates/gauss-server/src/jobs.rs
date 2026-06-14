//! Background jobs: schema refresh and query-based alerts.
//!
//! These implement [`gauss_scheduler::Job`] so the [`crate::serve`] loop runs
//! them on an interval. The same `sync_one` routine backs both the manual
//! `/api/databases/{id}/sync` endpoint and the periodic refresh job.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use gauss_core::domain::{infer_semantic_type, Database, Field, Table};
use gauss_core::error::{CoreError, CoreResult};
use gauss_core::gql::Query;
use gauss_db::Store;
use gauss_scheduler::{Job, Notifier};
use uuid::Uuid;

/// Introspect one source, fingerprint its columns, persist the tables, and flag
/// it synced. Returns `(table_name, column_count)` per discovered table.
pub async fn sync_one(store: &Arc<dyn Store>, db: &Database) -> CoreResult<Vec<(String, usize)>> {
    let uri = db.connection_uri.clone().ok_or_else(|| {
        CoreError::InvalidQuery(format!(
            "data source `{}` has no connection configured",
            db.name
        ))
    })?;
    let driver = gauss_drivers::connect(db.kind, &uri).await?;
    let discovered = driver.sync_schema().await?;

    let mut out = Vec::with_capacity(discovered.len());
    for dt in discovered {
        let col_names: Vec<String> = dt.columns.iter().map(|c| c.name.clone()).collect();
        let prints: HashMap<String, gauss_core::domain::Fingerprint> = driver
            .fingerprint(&dt.name, &col_names)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();

        let fields: Vec<Field> = dt
            .columns
            .iter()
            .map(|c| {
                let mut f = Field::new(c.name.clone(), c.field_type);
                if let Some(fp) = prints.get(&c.name) {
                    f.semantic_type = Some(infer_semantic_type(c.field_type, fp));
                    f.fingerprint = Some(fp.clone());
                }
                f
            })
            .collect();

        store
            .upsert_table(Table {
                id: Uuid::new_v4(),
                database_id: db.id,
                name: dt.name.clone(),
                fields,
            })
            .await?;
        out.push((dt.name, col_names.len()));
    }
    store.set_database_synced(db.id, true).await?;
    Ok(out)
}

/// Periodically re-sync every connected, already-synced source.
pub struct RefreshJob {
    pub store: Arc<dyn Store>,
}

#[async_trait]
impl Job for RefreshJob {
    fn name(&self) -> &str {
        "refresh-schemas"
    }
    async fn run(&self) -> CoreResult<()> {
        for db in self.store.list_databases().await? {
            if db.is_synced && db.connection_uri.is_some() {
                // Best-effort: a single source failing must not abort the sweep.
                let _ = sync_one(&self.store, &db).await;
            }
        }
        Ok(())
    }
}

/// Run a query and notify when it returns at least `min_rows` rows.
pub struct AlertJob {
    pub name: String,
    pub store: Arc<dyn Store>,
    pub database_id: Uuid,
    pub query: Query,
    pub min_rows: usize,
    pub notifier: Arc<dyn Notifier>,
}

#[async_trait]
impl Job for AlertJob {
    fn name(&self) -> &str {
        &self.name
    }
    async fn run(&self) -> CoreResult<()> {
        let db = self
            .store
            .database_by_id(self.database_id)
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("database {}", self.database_id)))?;
        let uri = db
            .connection_uri
            .clone()
            .ok_or_else(|| CoreError::InvalidQuery("alert source has no connection".into()))?;
        let driver = gauss_drivers::connect(db.kind, &uri).await?;
        let dialect = gauss_query::dialect::for_kind(db.kind);
        let compiled = gauss_query::compile(&self.query, dialect.as_ref())?;
        let result = driver.run(&compiled).await?;
        if result.rows.len() >= self.min_rows {
            self.notifier
                .notify(
                    &format!("Alert: {}", self.name),
                    &format!(
                        "{} row(s) matched (threshold {})",
                        result.rows.len(),
                        self.min_rows
                    ),
                )
                .await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_core::domain::DataSourceKind;
    use gauss_db::InMemoryStore;
    use std::sync::Mutex;

    struct CollectingNotifier {
        messages: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Notifier for CollectingNotifier {
        async fn notify(&self, subject: &str, body: &str) {
            self.messages
                .lock()
                .unwrap()
                .push(format!("{subject} | {body}"));
        }
    }

    #[tokio::test]
    async fn alert_fires_when_threshold_met() {
        // A real SQLite source with 3 rows.
        let path = std::env::temp_dir().join(format!("gauss_alert_{}.db", Uuid::new_v4()));
        let uri = format!("sqlite://{}", path.display());
        let setup = gauss_drivers::SqliteDriver::connect(&uri).await.unwrap();
        sqlx::query("CREATE TABLE errors (id INTEGER PRIMARY KEY)")
            .execute(setup.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO errors (id) VALUES (1),(2),(3)")
            .execute(setup.pool())
            .await
            .unwrap();

        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let db = Database {
            id: Uuid::new_v4(),
            name: "logs".into(),
            kind: DataSourceKind::Sqlite,
            is_synced: true,
            connection_uri: Some(uri),
            created_at: chrono::Utc::now(),
        };
        store.create_database(db.clone()).await.unwrap();

        let notifier = Arc::new(CollectingNotifier {
            messages: Mutex::new(Vec::new()),
        });
        let alert = AlertJob {
            name: "too-many-errors".into(),
            store: store.clone(),
            database_id: db.id,
            query: Query::new("errors"),
            min_rows: 2,
            notifier: notifier.clone(),
        };

        alert.run().await.unwrap();
        let msgs = notifier.messages.lock().unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].contains("too-many-errors"));

        let _ = std::fs::remove_file(&path);
    }
}
