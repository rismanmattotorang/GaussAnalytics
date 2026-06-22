//! `LocalFileSystem` — a disk-backed `FileSystem` sandboxed to a root directory.
//! Mirrors GaussAnalytics's `LocalFileSystem` (`integrations/local/file_system.py`).

use crate::context::ToolContext;
use crate::error::{AgentError, Result};
use crate::traits::{CommandResult, FileSearchMatch, FileSystem};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// File system rooted at a directory. All paths are resolved relative to the
/// root and prevented from escaping it.
pub struct LocalFileSystem {
    root: PathBuf,
}

impl LocalFileSystem {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve `path` under the root, rejecting traversal outside it.
    fn resolve(&self, path: &str) -> Result<PathBuf> {
        let candidate = self.root.join(path);
        // Reject `..` components defensively (works for non-existent paths too).
        if candidate
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(AgentError::Permission(format!(
                "path escapes sandbox root: {path}"
            )));
        }
        Ok(candidate)
    }
}

#[async_trait]
impl FileSystem for LocalFileSystem {
    async fn read_file(&self, path: &str, _context: &ToolContext) -> Result<String> {
        let p = self.resolve(path)?;
        tokio::fs::read_to_string(&p)
            .await
            .map_err(|e| AgentError::other(format!("read {path}: {e}")))
    }

    async fn write_file(
        &self,
        path: &str,
        content: &str,
        _context: &ToolContext,
        overwrite: bool,
    ) -> Result<()> {
        let p = self.resolve(path)?;
        if !overwrite && p.exists() {
            return Err(AgentError::other(format!("file exists: {path}")));
        }
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AgentError::other(format!("mkdir: {e}")))?;
        }
        tokio::fs::write(&p, content)
            .await
            .map_err(|e| AgentError::other(format!("write {path}: {e}")))
    }

    async fn list_files(&self, directory: &str, _context: &ToolContext) -> Result<Vec<String>> {
        let dir = self.resolve(directory)?;
        let mut entries = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| AgentError::other(format!("read_dir {directory}: {e}")))?;
        let mut out = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AgentError::other(format!("read_dir entry: {e}")))?
        {
            if let Some(name) = entry.file_name().to_str() {
                out.push(name.to_string());
            }
        }
        out.sort();
        Ok(out)
    }

    async fn exists(&self, path: &str, _context: &ToolContext) -> Result<bool> {
        Ok(Path::new(&self.resolve(path)?).exists())
    }

    async fn search_files(
        &self,
        query: &str,
        _context: &ToolContext,
        max_results: usize,
    ) -> Result<Vec<FileSearchMatch>> {
        let mut out = Vec::new();
        let mut stack = vec![self.root.clone()];
        let needle = query.to_lowercase();
        while let Some(dir) = stack.pop() {
            if out.len() >= max_results {
                break;
            }
            let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| AgentError::other(format!("search read_dir: {e}")))?
            {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let rel = path
                    .strip_prefix(&self.root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                let name_match = rel.to_lowercase().contains(&needle);
                let mut snippet = None;
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Some(line) = content.lines().find(|l| l.to_lowercase().contains(&needle))
                    {
                        snippet = Some(line.trim().chars().take(200).collect());
                    }
                }
                if name_match || snippet.is_some() {
                    out.push(FileSearchMatch { path: rel, snippet });
                    if out.len() >= max_results {
                        break;
                    }
                }
            }
        }
        Ok(out)
    }

    async fn run_bash(
        &self,
        command: &str,
        _context: &ToolContext,
        timeout_secs: Option<u64>,
    ) -> Result<CommandResult> {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(&self.root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = cmd
            .spawn()
            .map_err(|e| AgentError::other(format!("spawn command: {e}")))?;

        let output = match timeout_secs {
            Some(secs) => tokio::time::timeout(
                std::time::Duration::from_secs(secs),
                child.wait_with_output(),
            )
            .await
            .map_err(|_| AgentError::other(format!("command timed out after {secs}s")))?,
            None => child.wait_with_output().await,
        }
        .map_err(|e| AgentError::other(format!("command failed: {e}")))?;

        Ok(CommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}
