//! Audit hooks for MCP tool activity.
//!
//! Every governed action emits an [`AuditEvent`]. The [`AuditSink`] trait lets
//! the host application route these to logs, a database, or a SIEM. This crate
//! ships only a [`NoopAuditSink`]; richer sinks live in the server.

use crate::ToolInvocation;

/// A recordable event in the MCP tool lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEvent {
    /// A tool invocation passed policy and is about to be dispatched.
    ToolRequested { server: String, tool: String },
    /// A tool invocation finished.
    ToolCompleted {
        server: String,
        tool: String,
        ok: bool,
    },
}

impl AuditEvent {
    pub fn tool_requested(inv: &ToolInvocation) -> Self {
        AuditEvent::ToolRequested {
            server: inv.server.clone(),
            tool: inv.tool.clone(),
        }
    }

    pub fn tool_completed(inv: &ToolInvocation, ok: bool) -> Self {
        AuditEvent::ToolCompleted {
            server: inv.server.clone(),
            tool: inv.tool.clone(),
            ok,
        }
    }
}

/// A destination for audit events.
pub trait AuditSink: Send + Sync {
    fn record(&self, event: AuditEvent);
}

/// An audit sink that discards events (used when auditing is not configured).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopAuditSink;

impl AuditSink for NoopAuditSink {
    fn record(&self, _event: AuditEvent) {}
}
