//! Python execution tools (run a script, install packages) via the
//! `FileSystem::run_bash` capability. Mirrors `gauss/tools/python.py`.
//!
//! These are admin-gated and run inside the file-system sandbox.

use async_trait::async_trait;
use gauss_engine::components::{RichComponent, UiComponent};
use gauss_engine::context::{ToolContext, ToolResult};
use gauss_engine::tool::Tool;
use gauss_engine::traits::{CommandResult, FileSystem};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

fn render(result: &CommandResult, title: &str) -> ToolResult {
    let body = format!(
        "exit code: {}\n\n--- stdout ---\n{}\n--- stderr ---\n{}",
        result.exit_code,
        result.stdout.trim(),
        result.stderr.trim()
    );
    let tr = ToolResult::success(body.clone());
    let tr = tr.with_ui(UiComponent::new(RichComponent::card(title, body, false)));
    if result.exit_code == 0 {
        tr
    } else {
        ToolResult {
            success: false,
            error: Some(format!("command exited with code {}", result.exit_code)),
            ..tr
        }
    }
}

// ---- run_python_file ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunPythonFileArgs {
    /// The Python file to execute (relative to the sandbox root).
    pub filename: String,
    /// Arguments to pass to the script.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Optional timeout in seconds.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

pub struct RunPythonFileTool {
    fs: Arc<dyn FileSystem>,
}
impl RunPythonFileTool {
    pub fn new(fs: Arc<dyn FileSystem>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for RunPythonFileTool {
    type Args = RunPythonFileArgs;
    fn name(&self) -> &str {
        "run_python_file"
    }
    fn description(&self) -> &str {
        "Run a Python file in the workspace and return its stdout/stderr."
    }
    fn access_groups(&self) -> Vec<String> {
        vec!["admin".into()]
    }
    async fn execute(&self, ctx: &ToolContext, args: RunPythonFileArgs) -> ToolResult {
        let cmd = format!("python3 {} {}", args.filename, args.arguments.join(" "));
        match self
            .fs
            .run_bash(&cmd, ctx, args.timeout_seconds.or(Some(60)))
            .await
        {
            Ok(result) => render(&result, &format!("Ran {}", args.filename)),
            Err(e) => ToolResult::error(format!("run_python_file failed: {e}")),
        }
    }
}

// ---- pip_install ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PipInstallArgs {
    /// Packages to install.
    pub packages: Vec<String>,
    /// Pass `--upgrade`.
    #[serde(default)]
    pub upgrade: bool,
}

pub struct PipInstallTool {
    fs: Arc<dyn FileSystem>,
}
impl PipInstallTool {
    pub fn new(fs: Arc<dyn FileSystem>) -> Self {
        Self { fs }
    }
}

#[async_trait]
impl Tool for PipInstallTool {
    type Args = PipInstallArgs;
    fn name(&self) -> &str {
        "pip_install"
    }
    fn description(&self) -> &str {
        "Install Python packages with pip into the workspace environment."
    }
    fn access_groups(&self) -> Vec<String> {
        vec!["admin".into()]
    }
    async fn execute(&self, ctx: &ToolContext, args: PipInstallArgs) -> ToolResult {
        if args.packages.is_empty() {
            return ToolResult::error("no packages specified");
        }
        let upgrade = if args.upgrade { "--upgrade " } else { "" };
        let cmd = format!(
            "python3 -m pip install {upgrade}{}",
            args.packages.join(" ")
        );
        match self.fs.run_bash(&cmd, ctx, Some(300)).await {
            Ok(result) => render(&result, "pip install"),
            Err(e) => ToolResult::error(format!("pip_install failed: {e}")),
        }
    }
}
