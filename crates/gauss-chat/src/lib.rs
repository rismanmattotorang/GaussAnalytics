//! axum HTTP server for GaussAnalytics.
//!
//! Endpoints (paths kept as `gauss` for frontend compatibility in phase 1):
//! - `GET  /`                          → HTML index mounting the chat component
//! - `GET  /health`                    → health check JSON
//! - `POST /api/gauss/v2/chat_sse`     → SSE stream of `ChatStreamChunk`s
//! - `POST /api/gauss/v2/chat_poll`    → all chunks collected into one response
//! - `POST /api/gauss/v2/upload_csv`   → ingest a CSV body into a SQLite table

mod models;
mod templates;

pub use models::{ChatRequest, ChatResponse, ChatStreamChunk};

use axum::{
    body::Bytes,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use futures::stream::{self, Stream, StreamExt};
use gauss_engine::model::user::RequestContext;
use gauss_engine::Agent;
use serde_json::json;
use std::convert::Infallible;
use std::sync::Arc;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use uuid::Uuid;

/// Maximum accepted request body (16 MiB) — large enough for CSV uploads while
/// still bounding per-request memory.
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Shared handler state: the agent plus, when configured, the SQLite path that
/// `/upload_csv` ingests into.
#[derive(Clone)]
struct AppState {
    agent: Arc<Agent>,
    db_path: Option<Arc<str>>,
}

/// Bind to `addr` and serve until a shutdown signal (Ctrl-C / SIGTERM).
/// CSV upload is disabled (no SQLite path configured); use [`serve_with_db`].
pub async fn serve(agent: Arc<Agent>, addr: &str) -> std::io::Result<()> {
    serve_router(build_router(agent), addr).await
}

/// Like [`serve`], but wires the SQLite `db_path` so `/upload_csv` can ingest
/// CSV files into queryable tables.
pub async fn serve_with_db(agent: Arc<Agent>, db_path: String, addr: &str) -> std::io::Result<()> {
    serve_router(build_router_with_db(agent, Some(db_path)), addr).await
}

async fn serve_router(app: Router, addr: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
}

/// Resolves when the process receives Ctrl-C or (on Unix) SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received; draining connections");
}

/// Build the application router around a shared [`Agent`].
///
/// Layers (outermost first): panic isolation → request-body limit → CORS.
/// A panic inside a handler becomes a `500` instead of dropping the connection.
pub fn build_router(agent: Arc<Agent>) -> Router {
    build_router_with_db(agent, None)
}

/// Build the router with an optional SQLite `db_path` enabling `/upload_csv`.
pub fn build_router_with_db(agent: Arc<Agent>, db_path: Option<String>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let state = AppState {
        agent,
        db_path: db_path.map(Arc::from),
    };

    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/api/gauss/v2/chat_sse", post(chat_sse))
        .route("/api/gauss/v2/chat_poll", post(chat_poll))
        .route("/api/gauss/v2/chat_websocket", get(chat_websocket))
        .route("/api/gauss/v2/upload_csv", post(upload_csv))
        .layer(cors)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

async fn index() -> Html<String> {
    Html(templates::index_html())
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "healthy", "service": "gaussanalytics" }))
}

/// Build a `RequestContext` from HTTP headers and cookies so production
/// `UserResolver`s have what they need. (The default resolver ignores it.)
fn request_context_from_headers(headers: &HeaderMap) -> RequestContext {
    let mut ctx = RequestContext::default();
    for (name, value) in headers {
        if let Ok(v) = value.to_str() {
            ctx.headers.insert(name.as_str().to_string(), v.to_string());
        }
    }
    if let Some(cookie_header) = ctx.headers.get("cookie").cloned() {
        for pair in cookie_header.split(';') {
            if let Some((k, v)) = pair.trim().split_once('=') {
                ctx.cookies.insert(k.to_string(), v.to_string());
            }
        }
    }
    ctx
}

async fn chat_sse(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let agent = state.agent;
    let mut ctx = request_context_from_headers(&headers);
    ctx.metadata = req.metadata.clone();

    let conversation_id = req
        .conversation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let request_id = req
        .request_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let conv_for_chunks = conversation_id.clone();
    let req_for_chunks = request_id.clone();

    let component_stream = agent.send_message(ctx, req.message, Some(conversation_id));

    let event_stream = component_stream
        .map(move |component| {
            let chunk =
                ChatStreamChunk::from_component(&component, &conv_for_chunks, &req_for_chunks);
            let data = serde_json::to_string(&chunk).unwrap_or_else(|_| "{}".to_string());
            Ok::<_, Infallible>(Event::default().data(data))
        })
        .chain(stream::once(async {
            Ok::<_, Infallible>(Event::default().data("[DONE]"))
        }));

    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

async fn chat_websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state.agent, headers))
}

/// Each inbound text frame is a `ChatRequest`; the agent's components are sent
/// back as text frames, terminated by a `[DONE]` frame.
async fn handle_socket(mut socket: WebSocket, agent: Arc<Agent>, headers: HeaderMap) {
    while let Some(Ok(msg)) = socket.recv().await {
        let Message::Text(text) = msg else { continue };
        let Ok(req) = serde_json::from_str::<ChatRequest>(text.as_str()) else {
            continue;
        };
        let mut ctx = request_context_from_headers(&headers);
        ctx.metadata = req.metadata.clone();
        let conversation_id = req
            .conversation_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let request_id = req
            .request_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let mut stream = Box::pin(agent.clone().send_message(
            ctx,
            req.message,
            Some(conversation_id.clone()),
        ));
        while let Some(component) = stream.next().await {
            let chunk = ChatStreamChunk::from_component(&component, &conversation_id, &request_id);
            let data = serde_json::to_string(&chunk).unwrap_or_else(|_| "{}".to_string());
            if socket.send(Message::Text(data.into())).await.is_err() {
                return;
            }
        }
        if socket.send(Message::Text("[DONE]".into())).await.is_err() {
            return;
        }
    }
}

async fn chat_poll(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let agent = state.agent;
    let mut ctx = request_context_from_headers(&headers);
    ctx.metadata = req.metadata.clone();

    let conversation_id = req
        .conversation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let request_id = req
        .request_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut stream = Box::pin(agent.send_message(ctx, req.message, Some(conversation_id.clone())));
    let mut chunks = Vec::new();
    while let Some(component) = stream.next().await {
        chunks.push(ChatStreamChunk::from_component(
            &component,
            &conversation_id,
            &request_id,
        ));
    }

    let total = chunks.len();
    Json(ChatResponse {
        chunks,
        conversation_id,
        request_id,
        total_chunks: total,
    })
}

/// Ingest a CSV request body into a SQLite table. The desired table name comes
/// from the `x-table-name` header (falling back to `x-filename`, else a
/// default); it is sanitized server-side. Responds with the created table, its
/// columns, and the row count so the UI can confirm and prompt the user.
async fn upload_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(db_path) = state.db_path.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "CSV upload is not configured on this server" })),
        )
            .into_response();
    };

    let header = |k: &str| headers.get(k).and_then(|v| v.to_str().ok());
    let table_hint = header("x-table-name")
        .or_else(|| header("x-filename"))
        .unwrap_or("uploaded_table")
        .to_string();

    let Ok(csv_data) = String::from_utf8(body.to_vec()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "CSV must be valid UTF-8 text" })),
        )
            .into_response();
    };
    if csv_data.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "the uploaded file is empty" })),
        )
            .into_response();
    }

    // rusqlite is synchronous; ingest on the blocking pool.
    let db_path = db_path.to_string();
    let result = tokio::task::spawn_blocking(move || {
        gauss_sql::ingest_csv(&db_path, &table_hint, &csv_data)
    })
    .await;

    match result {
        Ok(Ok(summary)) => Json(json!({
            "table": summary.table,
            "row_count": summary.row_count,
            "columns": summary
                .columns
                .iter()
                .map(|c| json!({ "name": c.name, "type": c.sql_type }))
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("ingest task failed: {e}") })),
        )
            .into_response(),
    }
}
