//! `gauss-server` — the GaussAnalytics HTTP/JSON API.
//!
//! Built on axum, this crate exposes the contract the reused frontend and the
//! administration TUI both speak: health/version, authentication, databases,
//! GQL compilation *and execution* against connected sources, plus the AI
//! integration endpoints (NL2SQL, MCP) when enabled in configuration.

#![forbid(unsafe_code)]

pub mod auth;
pub mod error;
pub mod state;

use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::HeaderMap;
use axum::middleware::{self, Next};
use axum::response::Response;
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
        .route("/users", get(list_users))
        .route(
            "/users/{id}/grants",
            get(list_grants).post(add_grant).delete(revoke_grant),
        )
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api-keys/{id}/revoke", post(revoke_api_key))
        .route("/databases", get(list_databases).post(create_database))
        .route("/databases/{id}/sync", post(sync_database))
        .route("/databases/{id}/tables", get(list_database_tables))
        .route("/dataset/compile", post(compile_dataset))
        .route("/dataset/run", post(run_dataset))
        .route("/nl2sql", post(nl2sql_translate))
        .route("/mcp/servers", get(mcp_list_servers))
        .route("/mcp/invoke", post(mcp_invoke))
        .layer(middleware::from_fn_with_state(state.clone(), auth_gate))
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .fallback_service(ServeDir::new(static_dir))
        .layer(TraceLayer::new_for_http())
}

/// Paths under `/api` reachable without authentication even when
/// `require_auth` is enabled. (Paths here are as seen by the nested router, so
/// without the `/api` prefix.)
fn is_public_path(path: &str) -> bool {
    matches!(path, "/health" | "/version" | "/auth/login")
}

/// Middleware enforcing authentication when `security.require_auth` is set.
/// A no-op otherwise, so local development and the open demo are unaffected.
async fn auth_gate(
    State(st): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if !st.config.security.require_auth || is_public_path(req.uri().path()) {
        return Ok(next.run(req).await);
    }
    let headers = req.headers().clone();
    if auth::authenticate(&st, &headers).await.is_ok() {
        Ok(next.run(req).await)
    } else {
        Err(CoreError::Unauthorized("authentication required".into()).into())
    }
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

async fn list_users(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<MeResponse>>, ApiError> {
    require_admin(&st, &headers).await?;
    let users = st
        .store
        .list_users()
        .await?
        .into_iter()
        .map(|u| MeResponse {
            id: u.id,
            email: u.email,
            display_name: u.display_name,
            is_admin: u.is_admin,
        })
        .collect();
    Ok(Json(users))
}

// ---------------------------------------------------------------------------
// Permission grants (admin)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GrantBody {
    kind: String,
    #[serde(default)]
    scope: Option<Uuid>,
}

#[derive(Serialize)]
struct GrantInfo {
    kind: String,
    scope: Option<Uuid>,
}

fn parse_permission(body: &GrantBody) -> Result<Permission, ApiError> {
    Permission::from_parts(&body.kind, body.scope).ok_or_else(|| {
        CoreError::InvalidQuery(format!("unknown permission `{}`", body.kind)).into()
    })
}

async fn add_grant(
    State(st): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<GrantBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&st, &headers).await?;
    st.store.grant(user_id, parse_permission(&body)?).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn revoke_grant(
    State(st): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<GrantBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&st, &headers).await?;
    st.store.revoke(user_id, parse_permission(&body)?).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_grants(
    State(st): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<GrantInfo>>, ApiError> {
    require_admin(&st, &headers).await?;
    let grants = st
        .store
        .grants_for(user_id)
        .await?
        .into_iter()
        .map(|p| {
            let (kind, scope) = p.to_parts();
            GrantInfo {
                kind: kind.to_string(),
                scope,
            }
        })
        .collect();
    Ok(Json(grants))
}

// ---------------------------------------------------------------------------
// API keys (rotatable, DB-backed)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateApiKeyBody {
    name: String,
}

#[derive(Serialize)]
struct CreatedApiKey {
    id: Uuid,
    name: String,
    /// The plaintext key — shown exactly once, at creation.
    key: String,
    created_at: String,
}

#[derive(Serialize)]
struct ApiKeyView {
    id: Uuid,
    name: String,
    created_at: String,
    revoked: bool,
}

async fn create_api_key(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateApiKeyBody>,
) -> Result<Json<CreatedApiKey>, ApiError> {
    let current = auth::authenticate(&st, &headers).await?;
    let plaintext = gauss_auth::generate_api_key();
    let now = Utc::now();
    let record = gauss_db::ApiKeyRecord {
        id: Uuid::new_v4(),
        user_id: current.user.id,
        name: body.name.clone(),
        key_hash: gauss_auth::hash_api_key(&plaintext),
        created_at: now,
    };
    let id = record.id;
    st.store.create_api_key(record).await?;
    Ok(Json(CreatedApiKey {
        id,
        name: body.name,
        key: plaintext,
        created_at: now.to_rfc3339(),
    }))
}

async fn list_api_keys(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ApiKeyView>>, ApiError> {
    let current = auth::authenticate(&st, &headers).await?;
    let keys = st
        .store
        .list_api_keys(current.user.id)
        .await?
        .into_iter()
        .map(|k| ApiKeyView {
            id: k.id,
            name: k.name,
            created_at: k.created_at.to_rfc3339(),
            revoked: k.revoked,
        })
        .collect();
    Ok(Json(keys))
}

async fn revoke_api_key(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    auth::authenticate(&st, &headers).await?;
    st.store.revoke_api_key(id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Databases
// ---------------------------------------------------------------------------

async fn list_databases(State(st): State<AppState>) -> Result<Json<Vec<Database>>, ApiError> {
    Ok(Json(st.store.list_databases().await?))
}

/// Require an authenticated administrator (holds `ManageSettings`).
async fn require_admin(st: &AppState, headers: &HeaderMap) -> Result<auth::CurrentUser, ApiError> {
    let current = auth::authenticate(st, headers).await?;
    current.perms.require(Permission::ManageSettings)?;
    Ok(current)
}

#[derive(Deserialize)]
struct CreateDatabaseRequest {
    name: String,
    kind: DataSourceKind,
    #[serde(default)]
    connection_uri: Option<String>,
}

/// Register a new data source (admin only).
async fn create_database(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateDatabaseRequest>,
) -> Result<Json<Database>, ApiError> {
    require_admin(&st, &headers).await?;
    let db = Database {
        id: Uuid::new_v4(),
        name: req.name,
        kind: req.kind,
        is_synced: false,
        connection_uri: req.connection_uri,
        created_at: Utc::now(),
    };
    st.store.create_database(db.clone()).await?;
    Ok(Json(db))
}

#[derive(Serialize)]
struct SyncedTableSummary {
    name: String,
    columns: usize,
}

#[derive(Serialize)]
struct SyncResponse {
    database_id: Uuid,
    tables: Vec<SyncedTableSummary>,
}

/// Introspect a data source and persist its tables/columns (admin only).
async fn sync_database(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<SyncResponse>, ApiError> {
    require_admin(&st, &headers).await?;

    let db = st
        .store
        .database_by_id(id)
        .await?
        .ok_or_else(|| CoreError::NotFound(format!("database {id}")))?;
    let uri = db.connection_uri.clone().ok_or_else(|| {
        CoreError::InvalidQuery(format!(
            "data source `{}` has no connection configured",
            db.name
        ))
    })?;

    let driver = gauss_drivers::connect(db.kind, &uri).await?;
    let discovered = driver.sync_schema().await?;

    let mut summary = Vec::with_capacity(discovered.len());
    for dt in discovered {
        // Fingerprint the columns to derive value stats + semantic types.
        let col_names: Vec<String> = dt.columns.iter().map(|c| c.name.clone()).collect();
        let prints: std::collections::HashMap<String, gauss_core::domain::Fingerprint> = driver
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
                    f.semantic_type =
                        Some(gauss_core::domain::infer_semantic_type(c.field_type, fp));
                    f.fingerprint = Some(fp.clone());
                }
                f
            })
            .collect();

        st.store
            .upsert_table(Table {
                id: Uuid::new_v4(),
                database_id: db.id,
                name: dt.name.clone(),
                fields,
            })
            .await?;
        summary.push(SyncedTableSummary {
            name: dt.name,
            columns: dt.columns.len(),
        });
    }
    st.store.set_database_synced(db.id, true).await?;

    Ok(Json(SyncResponse {
        database_id: db.id,
        tables: summary,
    }))
}

/// List the synced tables of a data source.
async fn list_database_tables(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Table>>, ApiError> {
    Ok(Json(st.store.list_tables(id).await?))
}

// ---------------------------------------------------------------------------
// Dataset: compile GQL -> parameterized SQL
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CompileRequest {
    database_id: Uuid,
    query: Query,
}

/// Authorize, ground against metadata, and compile a request to SQL.
///
/// Shared by the compile-only and execute endpoints. If the caller presents a
/// session, read permission on the target database is enforced; anonymous calls
/// remain open in this scaffold (Phase 2 makes auth mandatory once per-database
/// grants are persisted).
async fn prepare_query(
    st: &AppState,
    headers: &HeaderMap,
    req: &CompileRequest,
) -> Result<(Database, CompiledQuery), ApiError> {
    if auth::bearer_token(headers).is_some() {
        let current = auth::authenticate(st, headers).await?;
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
    Ok((db, compiled))
}

async fn compile_dataset(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CompileRequest>,
) -> Result<Json<CompiledQuery>, ApiError> {
    let (_db, compiled) = prepare_query(&st, &headers, &req).await?;
    Ok(Json(compiled))
}

/// Compile *and execute* a GQL query against its connected data source,
/// returning the resulting rows.
async fn run_dataset(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CompileRequest>,
) -> Result<Json<gauss_drivers::QueryResult>, ApiError> {
    let (db, compiled) = prepare_query(&st, &headers, &req).await?;
    let uri = db.connection_uri.ok_or_else(|| {
        CoreError::InvalidQuery(format!(
            "data source `{}` has no connection configured",
            db.name
        ))
    })?;
    let driver = gauss_drivers::connect(db.kind, &uri).await?;
    let result = driver.run(&compiled).await?;
    Ok(Json(result))
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
/// (and so `/api/dataset/compile` works out of the box). Works against any
/// store implementation.
pub async fn seed_demo<S: DatabaseRepository + ?Sized>(store: &S) -> gauss_core::CoreResult<Uuid> {
    let db = Database {
        id: Uuid::new_v4(),
        name: "demo".into(),
        kind: DataSourceKind::Postgres,
        is_synced: true,
        connection_uri: None,
        created_at: Utc::now(),
    };
    let db_id = db.id;
    store.create_database(db).await?;

    let mk = |name: &str, ft: FieldType| Field::new(name, ft);
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

/// Build the application store from configuration: a persistent `sqlx`/SQLite
/// store for `sqlite*` URLs (creating the file + running migrations), otherwise
/// the in-memory store.
async fn build_store(config: &AppConfig) -> Result<Arc<dyn Store>, BoxError> {
    let url = &config.database.url;
    if url.starts_with("sqlite") {
        if let Some(path) = sqlite_file_path(url) {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
        }
        Ok(Arc::new(gauss_db::SqliteStore::connect(url).await?))
    } else if url.starts_with("postgres") {
        Ok(Arc::new(gauss_db::PgStore::connect(url).await?))
    } else if url.starts_with("mysql") {
        Ok(Arc::new(gauss_db::MySqlStore::connect(url).await?))
    } else {
        Ok(Arc::new(InMemoryStore::new()))
    }
}

/// Extract the filesystem path from a SQLite URL, or `None` for in-memory.
fn sqlite_file_path(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))?;
    let path = rest.split('?').next().unwrap_or(rest);
    if path.is_empty() || path == ":memory:" {
        None
    } else {
        Some(path.to_string())
    }
}

/// Run the HTTP server until the process is terminated.
pub async fn serve(config: AppConfig) -> Result<(), BoxError> {
    let addr = config.bind_addr();
    let store = build_store(&config).await?;

    // Seed the demo metadata on a fresh instance (idempotent).
    if store.list_databases().await?.is_empty() {
        seed_demo(store.as_ref()).await?;
    }

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
        let db_id = seed_demo(store.as_ref()).await.unwrap();
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

    #[tokio::test]
    async fn run_dataset_executes_against_a_sqlite_source() {
        use gauss_db::DatabaseRepository as _;
        use gauss_drivers::Driver as _;

        // A unique temporary SQLite source database.
        let path = std::env::temp_dir().join(format!("gauss_run_{}.db", Uuid::new_v4()));
        let uri = format!("sqlite://{}", path.display());

        // Create + seed the source.
        let setup = gauss_drivers::SqliteDriver::connect(&uri).await.unwrap();
        sqlx::query("CREATE TABLE orders (id INTEGER PRIMARY KEY, total REAL, status TEXT)")
            .execute(setup.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO orders (total, status) VALUES (?,?),(?,?)")
            .bind(10.5)
            .bind("paid")
            .bind(3.0)
            .bind("refunded")
            .execute(setup.pool())
            .await
            .unwrap();

        // Register the source in the metadata store and sync its schema.
        let store = Arc::new(InMemoryStore::new());
        let db = Database {
            id: Uuid::new_v4(),
            name: "sales".into(),
            kind: DataSourceKind::Sqlite,
            is_synced: true,
            connection_uri: Some(uri.clone()),
            created_at: Utc::now(),
        };
        let db_id = db.id;
        store.create_database(db).await.unwrap();
        for dt in setup.sync_schema().await.unwrap() {
            store
                .upsert_table(Table {
                    id: Uuid::new_v4(),
                    database_id: db_id,
                    name: dt.name,
                    fields: dt
                        .columns
                        .into_iter()
                        .map(|c| Field::new(c.name, c.field_type))
                        .collect(),
                })
                .await
                .unwrap();
        }

        let st = AppState::new(AppConfig::default(), store).unwrap();

        let mut q = Query::new("orders");
        q.fields = vec!["status".into(), "total".into()];
        q.filters = vec![Filter::Compare {
            field: "status".into(),
            op: CompareOp::Eq,
            value: Literal::Text("paid".into()),
        }];

        let resp = run_dataset(
            State(st),
            HeaderMap::new(),
            Json(CompileRequest {
                database_id: db_id,
                query: q,
            }),
        )
        .await
        .unwrap();

        assert_eq!(
            resp.0.columns,
            vec!["status".to_string(), "total".to_string()]
        );
        assert_eq!(resp.0.rows.len(), 1);
        assert_eq!(resp.0.rows[0][0], serde_json::json!("paid"));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn register_sync_and_list_tables() {
        let path = std::env::temp_dir().join(format!("gauss_ds_{}.db", Uuid::new_v4()));
        let uri = format!("sqlite://{}", path.display());
        let setup = gauss_drivers::SqliteDriver::connect(&uri).await.unwrap();
        sqlx::query("CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)")
            .execute(setup.pool())
            .await
            .unwrap();

        let (st, _db) = test_state().await;
        ensure_admin(st.store.as_ref(), "admin@example.com", "supersecret1")
            .await
            .unwrap();
        let login = auth::login(&st, "admin@example.com", "supersecret1")
            .await
            .unwrap();
        let hdr = bearer(&login.token);

        // Anonymous callers cannot register a data source.
        assert!(create_database(
            State(st.clone()),
            HeaderMap::new(),
            Json(CreateDatabaseRequest {
                name: "x".into(),
                kind: DataSourceKind::Sqlite,
                connection_uri: Some(uri.clone()),
            }),
        )
        .await
        .is_err());

        let created = create_database(
            State(st.clone()),
            hdr.clone(),
            Json(CreateDatabaseRequest {
                name: "crm".into(),
                kind: DataSourceKind::Sqlite,
                connection_uri: Some(uri.clone()),
            }),
        )
        .await
        .unwrap();
        let db_id = created.0.id;
        assert!(!created.0.is_synced);

        let synced = sync_database(State(st.clone()), Path(db_id), hdr.clone())
            .await
            .unwrap();
        assert!(synced
            .0
            .tables
            .iter()
            .any(|t| t.name == "customers" && t.columns == 3));

        let tables = list_database_tables(State(st.clone()), Path(db_id))
            .await
            .unwrap();
        let customers = tables.0.iter().find(|t| t.name == "customers").unwrap();
        assert_eq!(customers.fields.len(), 3);

        // The source is now flagged as synced.
        let dbs = list_databases(State(st)).await.unwrap();
        assert!(dbs.0.iter().any(|d| d.id == db_id && d.is_synced));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn require_auth_gates_routes_and_api_key_passes() {
        use axum::body::Body;
        use axum::http::StatusCode;
        use tower::ServiceExt;

        let store = Arc::new(InMemoryStore::new());
        seed_demo(store.as_ref()).await.unwrap();
        let mut cfg = AppConfig::default();
        cfg.security.require_auth = true;
        cfg.security.api_keys = vec!["secret-key".into()];
        let app = router(AppState::new(cfg, store).unwrap());

        // Public path is reachable without credentials.
        let resp = app
            .clone()
            .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Protected path is blocked for anonymous callers.
        let resp = app
            .clone()
            .oneshot(Request::get("/api/databases").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // A valid API key authenticates as a service principal.
        let resp = app
            .oneshot(
                Request::get("/api/databases")
                    .header("x-api-key", "secret-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn persisted_grant_allows_viewer_to_read_database() {
        let (st, db_id) = test_state().await;
        let uid = Uuid::new_v4();
        st.store
            .create_user(
                User {
                    id: uid,
                    email: "viewer@example.com".into(),
                    display_name: "Viewer".into(),
                    is_admin: false,
                    created_at: Utc::now(),
                },
                gauss_auth::hash_password("viewerpass12").unwrap(),
            )
            .await
            .unwrap();

        let login = auth::login(&st, "viewer@example.com", "viewerpass12")
            .await
            .unwrap();

        // Without a grant the viewer is denied.
        let denied = compile_dataset(
            State(st.clone()),
            bearer(&login.token),
            Json(CompileRequest {
                database_id: db_id,
                query: Query::new("orders"),
            }),
        )
        .await;
        assert!(denied.is_err());

        // Grant read on the database; now it succeeds.
        st.store
            .grant(uid, Permission::ReadDatabase { database_id: db_id })
            .await
            .unwrap();
        let mut q = Query::new("orders");
        q.fields = vec!["id".into()];
        let ok = compile_dataset(
            State(st),
            bearer(&login.token),
            Json(CompileRequest {
                database_id: db_id,
                query: q,
            }),
        )
        .await;
        assert!(ok.is_ok());
    }

    #[tokio::test]
    async fn db_api_key_authenticates_as_owner() {
        let (st, _db_id) = test_state().await;
        ensure_admin(st.store.as_ref(), "admin@example.com", "supersecret1")
            .await
            .unwrap();
        let admin = st
            .store
            .user_by_email("admin@example.com")
            .await
            .unwrap()
            .unwrap();

        let key = gauss_auth::generate_api_key();
        st.store
            .create_api_key(gauss_db::ApiKeyRecord {
                id: Uuid::new_v4(),
                user_id: admin.id,
                name: "ci".into(),
                key_hash: gauss_auth::hash_api_key(&key),
                created_at: Utc::now(),
            })
            .await
            .unwrap();

        let mut h = HeaderMap::new();
        h.insert("x-api-key", key.parse().unwrap());
        let me = auth_me(State(st), h).await.unwrap();
        assert_eq!(me.0.email, "admin@example.com");
    }

    /// Contract-compatibility suite: exercises every endpoint the reused
    /// frontend client (`frontend/src/api/client.ts`) depends on, asserting
    /// status codes and JSON shapes so the contract can't silently drift.
    #[tokio::test]
    async fn frontend_contract_surface() {
        use axum::body::{to_bytes, Body};
        use axum::http::StatusCode;
        use serde_json::Value as JsonValue;
        use tower::ServiceExt;

        async fn call(app: axum::Router, req: Request) -> (StatusCode, JsonValue) {
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status();
            let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            let val = if bytes.is_empty() {
                JsonValue::Null
            } else {
                serde_json::from_slice(&bytes).unwrap_or(JsonValue::Null)
            };
            (status, val)
        }

        let (st, db_id) = test_state().await;
        ensure_admin(st.store.as_ref(), "admin@example.com", "supersecret1")
            .await
            .unwrap();
        let app = router(st);

        // health
        let (s, v) = call(
            app.clone(),
            Request::get("/api/health").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["name"], "GaussAnalytics");
        assert_eq!(v["status"], "ok");
        assert!(v["version"].is_string());

        // version
        let (s, v) = call(
            app.clone(),
            Request::get("/api/version").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["owner"], "Gaussian Technologies");

        // login -> token
        let (s, v) = call(
            app.clone(),
            Request::post("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"email":"admin@example.com","password":"supersecret1"}"#,
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let token = v["token"].as_str().unwrap().to_string();
        assert!(v["expires_at"].is_string());

        // me
        let (s, v) = call(
            app.clone(),
            Request::get("/api/auth/me")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["email"], "admin@example.com");
        assert_eq!(v["is_admin"], true);

        // users (admin)
        let (s, v) = call(
            app.clone(),
            Request::get("/api/users")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(v.is_array());

        // databases
        let (s, v) = call(
            app.clone(),
            Request::get("/api/databases").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(v.is_array());

        // dataset/compile
        let body = format!(
            r#"{{"database_id":"{db_id}","query":{{"source_table":"orders","fields":["id"]}}}}"#
        );
        let (s, v) = call(
            app.clone(),
            Request::post("/api/dataset/compile")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(v["sql"].is_string());
        assert!(v["params"].is_array());

        // nl2sql disabled -> 404
        let body = format!(r#"{{"database_id":"{db_id}","prompt":"hi"}}"#);
        let (s, _) = call(
            app.clone(),
            Request::post("/api/nl2sql")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND);

        // mcp disabled -> 404
        let (s, _) = call(
            app,
            Request::get("/api/mcp/servers")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }
}
