//! `gauss-server` — the GaussAnalytics HTTP/JSON API.
//!
//! Built on axum, this crate exposes the contract the reused frontend and the
//! administration TUI both speak. Phase 1 wires the always-on core (health,
//! version, databases, GQL compilation) plus the AI integration endpoints
//! (NL2SQL, MCP) when those are enabled in configuration. Query *execution*
//! against connected sources lands in Phase 2; today the dataset endpoint
//! returns the compiled, parameterized SQL so the engine is verifiable.

#![forbid(unsafe_code)]

pub mod error;
pub mod state;

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use gauss_config::AppConfig;
use gauss_core::domain::{DataSourceKind, Database, Field, FieldType, Table};
use gauss_core::error::CoreError;
use gauss_core::gql::Query;
use gauss_db::{DatabaseRepository, InMemoryStore};
use gauss_mcp_gateway::{McpServer, ToolInvocation, ToolResult};
use gauss_nl2sql::{GuardedQuery, Nl2SqlRequest, SchemaContext, TableContext};
use gauss_query::CompiledQuery;
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

/// The platform's public name (de-branded — owned by Gaussian Technologies).
pub const PRODUCT_NAME: &str = "GaussAnalytics";

/// Build the application router for a given [`AppState`].
pub fn router(state: AppState) -> Router {
    // The reused React/TypeScript frontend is served as static assets from the
    // configured build directory; unknown paths fall through to it so the SPA
    // can handle client-side routing.
    let static_dir = state.config.server.static_dir.clone();

    let api = Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/databases", get(list_databases))
        .route("/dataset/compile", post(compile_dataset))
        .route("/nl2sql", post(nl2sql_translate))
        .route("/mcp/servers", get(mcp_list_servers))
        .route("/mcp/invoke", post(mcp_invoke))
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .fallback_service(ServeDir::new(static_dir))
        .layer(TraceLayer::new_for_http())
}

// ---------------------------------------------------------------------------
// Health / version
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Health {
    status: &'static str,
    name: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        name: PRODUCT_NAME,
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": PRODUCT_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "owner": "Gaussian Technologies",
    }))
}

// ---------------------------------------------------------------------------
// Databases
// ---------------------------------------------------------------------------

async fn list_databases(State(st): State<AppState>) -> Result<Json<Vec<Database>>, ApiError> {
    Ok(Json(st.store.list_databases().await?))
}

// ---------------------------------------------------------------------------
// Dataset: compile GQL -> parameterized SQL
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CompileRequest {
    database_id: Uuid,
    query: Query,
}

async fn compile_dataset(
    State(st): State<AppState>,
    Json(req): Json<CompileRequest>,
) -> Result<Json<CompiledQuery>, ApiError> {
    let db = st
        .store
        .database_by_id(req.database_id)
        .await?
        .ok_or_else(|| CoreError::NotFound(format!("database {}", req.database_id)))?;

    let table = st
        .store
        .table_by_name(db.id, &req.query.source_table)
        .await?
        .ok_or_else(|| {
            CoreError::InvalidQuery(format!("unknown table `{}`", req.query.source_table))
        })?;

    // Ground the query against synced metadata before compiling.
    req.query.validate(&table)?;

    let dialect = gauss_query::dialect::for_kind(db.kind);
    let compiled = gauss_query::compile(&req.query, dialect.as_ref())?;
    Ok(Json(compiled))
}

// ---------------------------------------------------------------------------
// NL2SQL (integration)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Nl2SqlApiRequest {
    database_id: Uuid,
    prompt: String,
}

async fn nl2sql_translate(
    State(st): State<AppState>,
    Json(req): Json<Nl2SqlApiRequest>,
) -> Result<Json<GuardedQuery>, ApiError> {
    let pipeline = st
        .nl2sql
        .as_ref()
        .ok_or_else(|| CoreError::NotFound("NL2SQL integration is not enabled".into()))?;

    let db = st
        .store
        .database_by_id(req.database_id)
        .await?
        .ok_or_else(|| CoreError::NotFound(format!("database {}", req.database_id)))?;

    // Build grounded schema context from synced metadata.
    let tables = st.store.list_tables(db.id).await?;
    let context = SchemaContext {
        database: db.name,
        tables: tables
            .into_iter()
            .map(|t| TableContext {
                name: t.name,
                columns: t
                    .fields
                    .into_iter()
                    .map(|f| (f.name, field_type_label(f.field_type).to_string()))
                    .collect(),
            })
            .collect(),
    };

    let guarded = pipeline
        .propose(&Nl2SqlRequest {
            prompt: req.prompt,
            context,
        })
        .await?;
    Ok(Json(guarded))
}

fn field_type_label(t: FieldType) -> &'static str {
    match t {
        FieldType::Integer => "integer",
        FieldType::Float => "float",
        FieldType::Text => "text",
        FieldType::Boolean => "boolean",
        FieldType::DateTime => "datetime",
        FieldType::Unknown => "unknown",
    }
}

// ---------------------------------------------------------------------------
// MCP gateway (integration)
// ---------------------------------------------------------------------------

async fn mcp_list_servers(State(st): State<AppState>) -> Result<Json<Vec<McpServer>>, ApiError> {
    let gw = st
        .mcp
        .as_ref()
        .ok_or_else(|| CoreError::NotFound("MCP gateway is not enabled".into()))?;
    Ok(Json(gw.list_servers().await?))
}

async fn mcp_invoke(
    State(st): State<AppState>,
    Json(invocation): Json<ToolInvocation>,
) -> Result<Json<ToolResult>, ApiError> {
    let gw = st
        .mcp
        .as_ref()
        .ok_or_else(|| CoreError::NotFound("MCP gateway is not enabled".into()))?;
    Ok(Json(gw.invoke(invocation).await?))
}

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

/// Seed a small demo data source so a fresh instance is immediately explorable
/// (and so `/api/dataset/compile` works out of the box).
pub async fn seed_demo(store: &InMemoryStore) -> gauss_core::CoreResult<Uuid> {
    let db = Database {
        id: Uuid::new_v4(),
        name: "demo".into(),
        kind: DataSourceKind::Postgres,
        is_synced: true,
        created_at: Utc::now(),
    };
    let db_id = db.id;
    store.create_database(db).await?;

    let mk = |name: &str, ft: FieldType| Field {
        id: Uuid::new_v4(),
        name: name.into(),
        field_type: ft,
    };
    store
        .upsert_table(Table {
            id: Uuid::new_v4(),
            database_id: db_id,
            name: "orders".into(),
            fields: vec![
                mk("id", FieldType::Integer),
                mk("total", FieldType::Float),
                mk("status", FieldType::Text),
                mk("created_at", FieldType::DateTime),
            ],
        })
        .await?;
    Ok(db_id)
}

/// Boxed error used by the server bootstrap (keeps `anyhow` out of the deps).
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Run the HTTP server until the process is terminated.
pub async fn serve(config: AppConfig) -> Result<(), BoxError> {
    let addr = config.bind_addr();
    let store = Arc::new(InMemoryStore::new());
    seed_demo(&store).await?;
    let state = AppState::new(config, store)?;
    let app = router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("{PRODUCT_NAME} listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_core::gql::{CompareOp, Filter, Literal};

    async fn test_state() -> (AppState, Uuid) {
        let store = Arc::new(InMemoryStore::new());
        let db_id = seed_demo(&store).await.unwrap();
        let cfg = AppConfig::default();
        (AppState::new(cfg, store).unwrap(), db_id)
    }

    #[tokio::test]
    async fn compile_endpoint_grounds_and_compiles() {
        let (st, db_id) = test_state().await;
        let mut q = Query::new("orders");
        q.fields = vec!["id".into(), "total".into()];
        q.filters = vec![Filter::Compare {
            field: "status".into(),
            op: CompareOp::Eq,
            value: Literal::Text("paid".into()),
        }];
        let resp = compile_dataset(
            State(st),
            Json(CompileRequest {
                database_id: db_id,
                query: q,
            }),
        )
        .await
        .unwrap();
        assert!(resp.0.sql.contains("WHERE"));
        assert!(resp.0.sql.contains("$1"));
        assert_eq!(resp.0.params.len(), 1);
    }

    #[tokio::test]
    async fn compile_rejects_unknown_field() {
        let (st, db_id) = test_state().await;
        let mut q = Query::new("orders");
        q.fields = vec!["nonexistent".into()];
        let err = compile_dataset(
            State(st),
            Json(CompileRequest {
                database_id: db_id,
                query: q,
            }),
        )
        .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn health_reports_product_name() {
        let h = health().await;
        assert_eq!(h.0.name, "GaussAnalytics");
        assert_eq!(h.0.status, "ok");
    }
}
