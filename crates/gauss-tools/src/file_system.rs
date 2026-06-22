//! File-system tools wrapping the `FileSystem` capability.
//! Mirrors `gauss/tools/file_system.py`.

use async_trait::async_trait;
use gauss_engine::components::{RichComponent, UiComponent};
use gauss_engine::context::{ToolContext, ToolResult};
use gauss_engine::tool::Tool;
use gauss_engine::traits::FileSystem;
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

fn default_dir() -> String {
    ".".to_string()
}
fn default_max() -> usize {
    20
}

// ---- list_files ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListFilesArgs {
    /// Directory to list (relative to the sandbox root). Defaults to the root.
    #[serde(default = "default_dir")]
    pub directory: String,
}

pub struct ListFilesTool {
    fs: Arc<dyn FileSystem>,
}
impl ListFilesTool {
    pub fn new(fs: Arc<dyn FileSystem>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for ListFilesTool {
    type Args = ListFilesArgs;
    fn name(&self) -> &str {
        "list_files"
    }
    fn description(&self) -> &str {
        "List files in a directory within the workspace."
    }
    fn access_groups(&self) -> Vec<String> {
        vec!["user".into(), "admin".into()]
    }
    async fn execute(&self, ctx: &ToolContext, args: ListFilesArgs) -> ToolResult {
        match self.fs.list_files(&args.directory, ctx).await {
            Ok(files) => ToolResult::success(format!(
                "{} item(s) in {}:\n{}",
                files.len(),
                args.directory,
                files.join("\n")
            )),
            Err(e) => ToolResult::error(format!("list_files failed: {e}")),
        }
    }
}

// ---- read_file ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileArgs {
    /// The file to read (relative to the sandbox root).
    pub filename: String,
}

pub struct ReadFileTool {
    fs: Arc<dyn FileSystem>,
}
impl ReadFileTool {
    pub fn new(fs: Arc<dyn FileSystem>) -> Self {
        Self { fs }
    }
}

const MAX_READ_CHARS: usize = 10_000;

#[async_trait]
impl Tool for ReadFileTool {
    type Args = ReadFileArgs;
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read the contents of a text file in the workspace."
    }
    fn access_groups(&self) -> Vec<String> {
        vec!["user".into(), "admin".into()]
    }
    async fn execute(&self, ctx: &ToolContext, args: ReadFileArgs) -> ToolResult {
        match self.fs.read_file(&args.filename, ctx).await {
            Ok(content) => {
                let truncated: String = content.chars().take(MAX_READ_CHARS).collect();
                let lang = args.filename.rsplit('.').next().unwrap_or("").to_string();
                ToolResult::success(format!("Contents of {}:\n{truncated}", args.filename))
                    .with_ui(UiComponent::new(RichComponent::code_block(truncated, lang)))
            }
            Err(e) => ToolResult::error(format!("read_file failed: {e}")),
        }
    }
}

// ---- write_file ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteFileArgs {
    /// The file to write (relative to the sandbox root).
    pub filename: String,
    /// The content to write.
    pub content: String,
    /// Overwrite if the file already exists.
    #[serde(default)]
    pub overwrite: bool,
}

pub struct WriteFileTool {
    fs: Arc<dyn FileSystem>,
}
impl WriteFileTool {
    pub fn new(fs: Arc<dyn FileSystem>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    type Args = WriteFileArgs;
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write content to a file in the workspace."
    }
    fn access_groups(&self) -> Vec<String> {
        vec!["admin".into()]
    }
    async fn execute(&self, ctx: &ToolContext, args: WriteFileArgs) -> ToolResult {
        match self
            .fs
            .write_file(&args.filename, &args.content, ctx, args.overwrite)
            .await
        {
            Ok(()) => ToolResult::success(format!(
                "Wrote {} byte(s) to {}.",
                args.content.len(),
                args.filename
            )),
            Err(e) => ToolResult::error(format!("write_file failed: {e}")),
        }
    }
}

// ---- search_files ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchFilesArgs {
    /// Text to search for in file names and contents.
    pub query: String,
    #[serde(default = "default_max")]
    pub max_results: usize,
}

pub struct SearchFilesTool {
    fs: Arc<dyn FileSystem>,
}
impl SearchFilesTool {
    pub fn new(fs: Arc<dyn FileSystem>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for SearchFilesTool {
    type Args = SearchFilesArgs;
    fn name(&self) -> &str {
        "search_files"
    }
    fn description(&self) -> &str {
        "Search the workspace for files matching a query (by name or content)."
    }
    fn access_groups(&self) -> Vec<String> {
        vec!["user".into(), "admin".into()]
    }
    async fn execute(&self, ctx: &ToolContext, args: SearchFilesArgs) -> ToolResult {
        match self
            .fs
            .search_files(&args.query, ctx, args.max_results)
            .await
        {
            Ok(matches) if matches.is_empty() => {
                ToolResult::success(format!("No files matched '{}'.", args.query))
            }
            Ok(matches) => {
                let lines: Vec<String> = matches
                    .iter()
                    .map(|m| match &m.snippet {
                        Some(s) => format!("{} — {s}", m.path),
                        None => m.path.clone(),
                    })
                    .collect();
                ToolResult::success(format!(
                    "{} match(es) for '{}':\n{}",
                    lines.len(),
                    args.query,
                    lines.join("\n")
                ))
            }
            Err(e) => ToolResult::error(format!("search_files failed: {e}")),
        }
    }
}

/// Convenience: build all read-oriented + write file-system tools.
pub fn create_file_system_tools(
    fs: Arc<dyn FileSystem>,
) -> (ListFilesTool, ReadFileTool, WriteFileTool, SearchFilesTool) {
    (
        ListFilesTool::new(fs.clone()),
        ReadFileTool::new(fs.clone()),
        WriteFileTool::new(fs.clone()),
        SearchFilesTool::new(fs),
    )
}
