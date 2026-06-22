//! Built-in tools for GaussAnalytics.
//!
//! - [`RunSqlTool`] — execute SQL, stream a table, optionally persist CSV.
//! - [`VisualizeDataTool`] — read a CSV and render a chart.
//! - Agent-memory tools — the search-before / save-after self-learning loop.

mod agent_memory;
mod file_system;
mod python;
mod run_sql;
mod schema_context;
mod visualize_data;

pub use agent_memory::{
    SaveQuestionToolArgsTool, SaveTextMemoryTool, SearchSavedCorrectToolUsesTool,
};
pub use file_system::{
    create_file_system_tools, ListFilesTool, ReadFileTool, SearchFilesTool, WriteFileTool,
};
pub use python::{PipInstallTool, RunPythonFileTool};
pub use run_sql::{RunSqlArgs, RunSqlTool};
pub use schema_context::SchemaContextEnhancer;
pub use visualize_data::{parse_csv, VisualizeDataArgs, VisualizeDataTool};
