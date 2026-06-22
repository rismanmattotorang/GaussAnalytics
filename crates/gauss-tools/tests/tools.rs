//! Integration tests for the phase-2 tools, exercised through the registry
//! (so argument validation and dynamic dispatch are covered too).

use std::sync::Arc;

use gauss_engine::context::ToolContext;
use gauss_engine::defaults::{InMemoryAgentMemory, LocalFileSystem};
use gauss_engine::model::tool::ToolCall;
use gauss_engine::model::user::User;
use gauss_engine::tool::ToolRegistry;
use gauss_engine::traits::{AgentMemory, FileSystem};
use gauss_tools::{
    RunSqlTool, SaveQuestionToolArgsTool, SearchSavedCorrectToolUsesTool, VisualizeDataTool,
};
use serde_json::{json, Map, Value};

fn args(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

fn ctx(memory: Arc<dyn AgentMemory>) -> ToolContext {
    ToolContext::new(
        User::new("u").with_groups(["user", "admin"]),
        "conv",
        "req",
        memory,
    )
}

#[tokio::test]
async fn visualize_reads_csv_and_charts() {
    let dir = std::env::temp_dir().join(format!("pt_vis_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fs: Arc<dyn FileSystem> = Arc::new(LocalFileSystem::new(&dir));
    let memory: Arc<dyn AgentMemory> = Arc::new(InMemoryAgentMemory::new());
    let context = ctx(memory);

    fs.write_file("data.csv", "name,value\nA,10\nB,20\n", &context, true)
        .await
        .unwrap();

    let mut reg = ToolRegistry::new();
    reg.register(VisualizeDataTool::new(fs));

    let call = ToolCall {
        id: "1".into(),
        name: "visualize_data".into(),
        arguments: args(json!({ "filename": "data.csv" })),
    };
    let result = reg.execute(&call, &context).await;
    assert!(result.success, "{:?}", result.error);
    let ui = result.ui_component.expect("chart component");
    assert_eq!(ui.rich_component.data["chart_type"], "bar");
}

#[tokio::test]
async fn memory_save_then_search_roundtrip() {
    let memory: Arc<dyn AgentMemory> = Arc::new(InMemoryAgentMemory::new());
    let context = ctx(memory);
    let mut reg = ToolRegistry::new();
    reg.register(SaveQuestionToolArgsTool);
    reg.register(SearchSavedCorrectToolUsesTool);

    let save = ToolCall {
        id: "1".into(),
        name: "save_question_tool_args".into(),
        arguments: args(json!({
            "question": "who are the top customers",
            "tool_name": "run_sql",
            "args": { "sql": "SELECT * FROM customers ORDER BY lifetime_value DESC" }
        })),
    };
    assert!(reg.execute(&save, &context).await.success);

    let search = ToolCall {
        id: "2".into(),
        name: "search_saved_correct_tool_uses".into(),
        arguments: args(json!({ "question": "top customers", "similarity_threshold": 0.1 })),
    };
    let result = reg.execute(&search, &context).await;
    assert!(result.success);
    assert!(
        result.result_for_llm.contains("run_sql"),
        "expected a saved run_sql match, got: {}",
        result.result_for_llm
    );
}

#[tokio::test]
async fn run_sql_without_filesystem_still_works() {
    // RunSqlTool with no file system should not error on the CSV-write path.
    struct OneRow;
    #[async_trait::async_trait]
    impl gauss_engine::traits::SqlRunner for OneRow {
        async fn run_sql(
            &self,
            _sql: &str,
            _ctx: &ToolContext,
        ) -> gauss_engine::Result<gauss_engine::DataFrame> {
            Ok(gauss_engine::DataFrame::new(
                vec!["n".into()],
                vec![vec![json!(1)]],
            ))
        }
    }
    let memory: Arc<dyn AgentMemory> = Arc::new(InMemoryAgentMemory::new());
    let context = ctx(memory);
    let mut reg = ToolRegistry::new();
    reg.register(RunSqlTool::new(Arc::new(OneRow)));
    let call = ToolCall {
        id: "1".into(),
        name: "run_sql".into(),
        arguments: args(json!({ "sql": "SELECT 1 AS n" })),
    };
    let result = reg.execute(&call, &context).await;
    assert!(result.success);
    assert!(result.ui_component.is_some());
}
