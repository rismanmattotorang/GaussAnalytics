//! `gauss-server` — the GaussAnalytics HTTP/JSON API.
//!
//! Built on axum, this crate exposes the contract the reused frontend and the
//! administration TUI both speak. Phase 1 wires the always-on core (health,
//! version, databases, GQL compilation) plus the AI integration endpoints
//! (NL2SQL, MCP) when those are enabled in configuration. Query *execution*
//! against connected sources lands in Phase 2; today the dataset endpoint
//! returns the compiled, parameterized SQL so the engine is verifiable.

#![forbid(unsafe_code)]

pub mod auth;
pub mod error;
pub mod state;

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use gauss_auth::Permission;
use gauss_config::AppConfig;
use gauss_core::domain::{DataSourceKind, Database, Field, FieldType, Table, User};
use gauss_core::error::CoreError;
use gauss_core::gql::Query;
use gauss_db::{DatabaseRepository, InMemoryStore, Store};
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
        .route("/auth/login", post(auth_login))
        .route("/auth/logout", post(auth_logout))
        .route("/auth/me", get(auth_me))
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
// Authentication
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    token: String,
    expires_at: String,
}

async fn auth_login(
    State(st): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let session = auth::login(&st, &req.email, &req.password).await?;
    Ok(Json(LoginResponse {
        token: session.token,
        expires_at: session.expires_at.to_rfc3339(),
    }))
}

async fn auth_logout(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let current = auth::authenticate(&st, &headers).await?;
    st.store.delete_session(&current.token).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Serialize)]
struct MeResponse {
    id: Uuid,
    email: String,
    display_name: String,
    is_admin: bool,
}

async fn auth_me(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, ApiError> {
    let current = auth::authenticate(&st, &headers).await?;
    Ok(Json(MeResponse {
        id: current.user.id,
        email: current.user.email,
        display_name: current.user.display_name,
        is_admin: current.user.is_admin,
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
    headers: HeaderMap,
    Json(req): Json<CompileRequest>,
) -> Result<Json<CompiledQuery>, ApiError> {
    // If the caller presents a session, enforce read permission on the target
    // database. Anonymous calls remain open in this scaffold; Phase 2 makes
    // authentication mandatory once per-database grants are persisted.
    if auth::bearer_token(&headers).is_some() {
        let current = auth::authenticate(&st, &headers).await?;
        current.perms.require(Permission::ReadDatabase {
            database_id: req.database_id,
        })?;
    }

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

/// Ensure an administrator account exists, creating one if absent.
///
/// Used at startup to bootstrap the first admin from configuration so the
/// instance is immediately usable. Idempotent: a no-op if the user exists.
pub async fn ensure_admin(
    store: &dyn Store,
    email: &str,
    password: &str,
) -> gauss_core::CoreResult<()> {
    if store.user_by_email(email).await?.is_some() {
        return Ok(());
    }
    let hash = gauss_auth::hash_password(password)?;
    let user = User {
        id: Uuid::new_v4(),
        email: email.to_string(),
        display_name: "Administrator".to_string(),
        is_admin: true,
        created_at: Utc::now(),
    };
    store.create_user(user, hash).await
}

/// Boxed error used by the server bootstrap (keeps `anyhow` out of the deps).
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Run the HTTP server until the process is terminated.
pub async fn serve(config: AppConfig) -> Result<(), BoxError> {
    let addr = config.bind_addr();
    let store = Arc::new(InMemoryStore::new());
    seed_demo(&store).await?;

    // Bootstrap an administrator from the environment when provided.
    if let (Ok(email), Ok(password)) = (
        std::env::var("GAUSS_ADMIN_EMAIL"),
        std::env::var("GAUSS_ADMIN_PASSWORD"),
    ) {
        ensure_admin(store.as_ref(), &email, &password).await?;
        tracing::info!("ensured administrator account for {email}");
    }

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
            HeaderMap::new(),
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
            HeaderMap::new(),
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

    fn bearer(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        h
    }

    #[tokio::test]
    async fn login_then_me_round_trip() {
        let (st, _db_id) = test_state().await;
        ensure_admin(st.store.as_ref(), "admin@example.com", "supersecret1")
            .await
            .unwrap();

        let login = auth_login(
            State(st.clone()),
            Json(LoginRequest {
                email: "admin@example.com".into(),
                password: "supersecret1".into(),
            }),
        )
        .await
        .unwrap();

        let me = auth_me(State(st), bearer(&login.0.token)).await.unwrap();
        assert_eq!(me.0.email, "admin@example.com");
        assert!(me.0.is_admin);
    }

    #[tokio::test]
    async fn login_rejects_bad_password() {
        let (st, _db_id) = test_state().await;
        ensure_admin(st.store.as_ref(), "admin@example.com", "supersecret1")
            .await
            .unwrap();
        let res = auth_login(
            State(st),
            Json(LoginRequest {
                email: "admin@example.com".into(),
                password: "wrong-password".into(),
            }),
        )
        .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn non_admin_is_denied_database_read() {
        let (st, db_id) = test_state().await;
        // Create a non-admin user and log in.
        let hash = gauss_auth::hash_password("viewerpass12").unwrap();
        st.store
            .create_user(
                User {
                    id: Uuid::new_v4(),
                    email: "viewer@example.com".into(),
                    display_name: "Viewer".into(),
                    is_admin: false,
                    created_at: Utc::now(),
                },
                hash,
            )
            .await
            .unwrap();
        let login = auth::login(&st, "viewer@example.com", "viewerpass12")
            .await
            .unwrap();

        // An authenticated viewer has no ReadDatabase grant -> denied.
        let res = compile_dataset(
            State(st),
            bearer(&login.token),
            Json(CompileRequest {
                database_id: db_id,
                query: Query::new("orders"),
            }),
        )
        .await;
        assert!(res.is_err());
    }
}
