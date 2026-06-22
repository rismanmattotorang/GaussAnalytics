//! Default audit-logger implementations: in-memory (for tests/inspection) and
//! file-backed JSONL (one JSON event per line).

use crate::error::{AgentError, Result};
use crate::model::audit::AuditEvent;
use crate::traits::AuditLogger;
use async_trait::async_trait;
use std::sync::RwLock;
use tokio::io::AsyncWriteExt;

/// Collects audit events in memory. Useful for tests and `/audit` inspection.
#[derive(Default)]
pub struct InMemoryAuditLogger {
    events: RwLock<Vec<AuditEvent>>,
}

impl InMemoryAuditLogger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<AuditEvent> {
        self.events.read().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.events.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl AuditLogger for InMemoryAuditLogger {
    async fn log_event(&self, event: AuditEvent) -> Result<()> {
        self.events.write().unwrap().push(event);
        Ok(())
    }
}

/// Appends audit events to a file as JSON Lines (one event per line).
pub struct FileAuditLogger {
    path: String,
}

impl FileAuditLogger {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl AuditLogger for FileAuditLogger {
    async fn log_event(&self, event: AuditEvent) -> Result<()> {
        let mut line = serde_json::to_string(&event)?;
        line.push('\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| AgentError::other(format!("open audit log: {e}")))?;
        file.write_all(line.as_bytes())
            .await
            .map_err(|e| AgentError::other(format!("write audit log: {e}")))?;
        Ok(())
    }
}
