//! HTTP-layer integration tests: drive the real axum router with `tower::oneshot`
//! and assert the wire contract (health JSON, poll chunks, SSE framing + [DONE]).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use gauss_chat::{build_router, build_router_with_db};
use gauss_engine::agent::AgentBuilder;
use gauss_engine::defaults::{InMemoryAgentMemory, StaticUserResolver};
use gauss_engine::tool::ToolRegistry;
use gauss_engine::traits::{AgentMemory, LlmContextEnhancer, LlmService, SqlRunner, UserResolver};
use gauss_llm::MockLlmService;
use gauss_sql::SqliteRunner;
use gauss_tools::{RunSqlTool, SchemaContextEnhancer};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn test_agent() -> Arc<gauss_engine::Agent> {
    let runner: Arc<dyn SqlRunner> = Arc::new(SqliteRunner::new(":memory:"));
    let memory: Arc<dyn AgentMemory> = Arc::new(InMemoryAgentMemory::new());
    let llm: Arc<dyn LlmService> = Arc::new(MockLlmService::new());
    let resolver: Arc<dyn UserResolver> = Arc::new(StaticUserResolver::admin());
    let mut reg = ToolRegistry::new();
    reg.register(RunSqlTool::new(runner));
    Arc::new(AgentBuilder::new(llm, reg, resolver, memory).build())
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn health_returns_ok_json() {
    let app = build_router(test_agent());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "healthy");
    assert_eq!(v["service"], "gaussanalytics");
}

#[tokio::test]
async fn chat_poll_returns_chunks() {
    let app = build_router(test_agent());
    let req = Request::builder()
        .method("POST")
        .uri("/api/gauss/v2/chat_poll")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"message":"SELECT 1 AS x"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
    assert!(v["total_chunks"].as_u64().unwrap() > 0);
    // A dataframe component must be present (run_sql executed the query).
    let chunks = v["chunks"].as_array().unwrap();
    assert!(chunks.iter().any(|c| c["rich"]["type"] == "dataframe"));
    // Every chunk echoes the conversation/request ids and a timestamp.
    assert!(chunks[0]["conversation_id"].is_string());
}

#[tokio::test]
async fn chat_sse_streams_frames_and_done() {
    let app = build_router(test_agent());
    let req = Request::builder()
        .method("POST")
        .uri("/api/gauss/v2/chat_sse")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"message":"SELECT 1 AS x"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(ct.contains("text/event-stream"), "content-type was {ct}");
    let body = body_string(resp).await;
    assert!(body.contains("data:"), "missing SSE data frames");
    assert!(body.contains("[DONE]"), "missing [DONE] terminator");
    assert!(
        body.contains("dataframe"),
        "missing streamed dataframe component"
    );
}

#[tokio::test]
async fn index_serves_self_contained_ui() {
    let app = build_router(test_agent());
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp).await;
    // Our own app, wired to the SSE endpoint...
    assert!(html.contains("GaussAnalytics"));
    assert!(html.contains("/api/gauss/v2/chat_sse"));
    assert!(html.contains("[DONE]"));
    // ...with no external/CDN script dependency.
    assert!(!html.contains("img.gauss.ai"));
    assert!(!html.to_lowercase().contains("<script src"));
}

/// A unique temp SQLite path (file-backed, so upload + agent share state).
fn temp_db() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "gauss_upload_{}_{}.db",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

/// Agent whose SQLite runner points at `path` and which injects the live schema
/// so the mock LLM can query whatever tables exist (incl. uploaded CSVs).
fn agent_with_db(path: &str) -> Arc<gauss_engine::Agent> {
    let runner: Arc<dyn SqlRunner> = Arc::new(SqliteRunner::new(path.to_string()));
    let memory: Arc<dyn AgentMemory> = Arc::new(InMemoryAgentMemory::new());
    let llm: Arc<dyn LlmService> = Arc::new(MockLlmService::new());
    let resolver: Arc<dyn UserResolver> = Arc::new(StaticUserResolver::admin());
    let mut reg = ToolRegistry::new();
    reg.register(RunSqlTool::new(runner.clone()));
    let enhancer: Arc<dyn LlmContextEnhancer> = Arc::new(SchemaContextEnhancer::new(runner));
    Arc::new(
        AgentBuilder::new(llm, reg, resolver, memory)
            .llm_context_enhancer(enhancer)
            .build(),
    )
}

/// End to end: upload a CSV → it lands in SQLite → a chat question returns the
/// uploaded rows. This is the whole feature, exercised over real HTTP.
#[tokio::test]
async fn upload_csv_then_query_it_end_to_end() {
    let db = temp_db();
    let app = build_router_with_db(agent_with_db(&db), Some(db.clone()));

    // 1. Upload.
    let csv = "region,amount\nUS,100\nDE,50\n";
    let up = Request::builder()
        .method("POST")
        .uri("/api/gauss/v2/upload_csv")
        .header("content-type", "text/csv")
        .header("x-table-name", "sales report.csv")
        .body(Body::from(csv))
        .unwrap();
    let resp = app.clone().oneshot(up).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "upload should succeed");
    let v: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
    assert_eq!(v["table"], "sales_report_csv");
    assert_eq!(v["row_count"], 2);
    let cols = v["columns"].as_array().unwrap();
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[1]["type"], "INTEGER"); // amount inferred numeric

    // 2. Ask about it in chat — the schema enhancer + mock turn this into SQL.
    let q = Request::builder()
        .method("POST")
        .uri("/api/gauss/v2/chat_poll")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"message":"what is the total amount in sales_report_csv?"}"#,
        ))
        .unwrap();
    let resp = app.clone().oneshot(q).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let chunks = v["chunks"].as_array().unwrap();
    assert!(
        chunks.iter().any(|c| c["rich"]["type"] == "dataframe"),
        "expected a dataframe from the uploaded table"
    );
    // The uploaded data is actually present in the answer.
    assert!(
        body.contains("US"),
        "uploaded rows should be returned: {body}"
    );

    let _ = std::fs::remove_file(&db);
}

/// Upload is rejected cleanly when the server has no SQLite path configured.
#[tokio::test]
async fn upload_csv_disabled_without_db() {
    let app = build_router(test_agent()); // no db_path
    let req = Request::builder()
        .method("POST")
        .uri("/api/gauss/v2/upload_csv")
        .header("content-type", "text/csv")
        .body(Body::from("a,b\n1,2\n"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn unknown_route_is_404() {
    let app = build_router(test_agent());
    let resp = app
        .oneshot(Request::builder().uri("/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
