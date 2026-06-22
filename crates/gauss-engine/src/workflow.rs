//! Workflow handler: deterministic, pre-LLM handling of slash-commands and the
//! starter UI. Mirrors `gauss/core/workflow/{base,default}.py`.

use crate::agent::Agent;
use crate::components::{RichComponent, UiComponent};
use crate::context::ToolContext;
use crate::model::conversation::Conversation;
use crate::model::user::User;
use async_trait::async_trait;

/// Outcome of a workflow-handler attempt.
#[derive(Default)]
pub struct WorkflowResult {
    /// If true, the agent skips the LLM entirely and just streams `components`.
    pub should_skip_llm: bool,
    pub components: Vec<UiComponent>,
}

impl WorkflowResult {
    pub fn passthrough() -> Self {
        Self::default()
    }

    pub fn handled(components: Vec<UiComponent>) -> Self {
        Self {
            should_skip_llm: true,
            components,
        }
    }
}

#[async_trait]
pub trait WorkflowHandler: Send + Sync {
    /// Attempt to handle the message before the LLM is consulted.
    async fn try_handle(
        &self,
        agent: &Agent,
        user: &User,
        conversation: &Conversation,
        message: &str,
    ) -> WorkflowResult;

    /// Optional UI shown when a conversation is opened with no message.
    async fn get_starter_ui(
        &self,
        _agent: &Agent,
        _user: &User,
        _conversation: &Conversation,
    ) -> Option<Vec<UiComponent>> {
        None
    }
}

/// Default handler: implements `/help` for everyone and a welcome starter card.
/// (Admin-gated `/status`, `/memories`, `/delete` arrive in phase 3.)
#[derive(Default)]
pub struct DefaultWorkflowHandler;

#[async_trait]
impl WorkflowHandler for DefaultWorkflowHandler {
    async fn try_handle(
        &self,
        agent: &Agent,
        user: &User,
        conversation: &Conversation,
        message: &str,
    ) -> WorkflowResult {
        let trimmed = message.trim();
        let is_admin = user.group_memberships.iter().any(|g| g == "admin");

        if trimmed == "/help" {
            let tools = agent.tool_registry().get_schemas(user);
            let tool_list = if tools.is_empty() {
                "(none)".to_string()
            } else {
                tools
                    .iter()
                    .map(|t| format!("- `{}` — {}", t.name, t.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let mut body = format!(
                "**GaussAnalytics** — ask a question about your data in plain \
                 language and I'll query it for you.\n\nAvailable tools:\n{tool_list}"
            );
            if is_admin {
                body.push_str("\n\nAdmin commands: `/status`, `/memories`, `/delete <id>`.");
            }
            return WorkflowResult::handled(vec![UiComponent::new(RichComponent::card(
                "Help", body, true,
            ))]);
        }

        // The remaining commands are admin-only.
        if trimmed == "/status" || trimmed == "/memories" || trimmed.starts_with("/delete") {
            if !is_admin {
                return WorkflowResult::handled(vec![UiComponent::new(
                    RichComponent::status_card(
                        "Permission denied",
                        "error",
                        Some("This command requires the `admin` group.".into()),
                        Some("🔒".into()),
                        Default::default(),
                    ),
                )]);
            }
            let ctx = ToolContext::new(
                user.clone(),
                conversation.id.clone(),
                "workflow",
                agent.agent_memory().clone(),
            );

            if trimmed == "/status" {
                let tools = agent.tool_registry().get_schemas(user);
                let body = format!(
                    "**Configured tools** ({}):\n{}\n\nMax tool iterations: {}\nStreaming: {}",
                    tools.len(),
                    tools
                        .iter()
                        .map(|t| format!("- `{}`", t.name))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    agent.config().max_tool_iterations,
                    agent.config().stream_responses,
                );
                return WorkflowResult::handled(vec![UiComponent::new(RichComponent::card(
                    "Status", body, true,
                ))]);
            }

            if trimmed == "/memories" {
                let tool_mems = agent
                    .agent_memory()
                    .get_recent_memories(&ctx, 10)
                    .await
                    .unwrap_or_default();
                let text_mems = agent
                    .agent_memory()
                    .get_recent_text_memories(&ctx, 10)
                    .await
                    .unwrap_or_default();
                let mut body = String::from("**Recent tool memories**\n");
                if tool_mems.is_empty() {
                    body.push_str("_(none)_\n");
                }
                for m in &tool_mems {
                    body.push_str(&format!(
                        "- `{}` — {} → {}\n",
                        m.memory_id.as_deref().unwrap_or("?"),
                        m.question,
                        m.tool_name
                    ));
                }
                body.push_str("\n**Recent text memories**\n");
                if text_mems.is_empty() {
                    body.push_str("_(none)_\n");
                }
                for m in &text_mems {
                    body.push_str(&format!(
                        "- `{}` — {}\n",
                        m.memory_id.as_deref().unwrap_or("?"),
                        m.content.chars().take(80).collect::<String>()
                    ));
                }
                body.push_str("\nDelete one with `/delete <id>`.");
                return WorkflowResult::handled(vec![UiComponent::new(RichComponent::card(
                    "Memories", body, true,
                ))]);
            }

            // /delete <id>
            let id = trimmed.strip_prefix("/delete").unwrap_or("").trim();
            if id.is_empty() {
                return WorkflowResult::handled(vec![UiComponent::new(RichComponent::card(
                    "Delete",
                    "Usage: `/delete <memory-id>`".to_string(),
                    true,
                ))]);
            }
            let deleted = agent
                .agent_memory()
                .delete_by_id(&ctx, id)
                .await
                .unwrap_or(false)
                || agent
                    .agent_memory()
                    .delete_text_memory(&ctx, id)
                    .await
                    .unwrap_or(false);
            let msg = if deleted {
                format!("Deleted memory `{id}`.")
            } else {
                format!("No memory found with id `{id}`.")
            };
            return WorkflowResult::handled(vec![UiComponent::new(RichComponent::card(
                "Delete", msg, true,
            ))]);
        }

        WorkflowResult::passthrough()
    }

    async fn get_starter_ui(
        &self,
        _agent: &Agent,
        _user: &User,
        _conversation: &Conversation,
    ) -> Option<Vec<UiComponent>> {
        Some(vec![UiComponent::new(RichComponent::card(
            "Welcome to GaussAnalytics",
            "Ask a question about your data, e.g. *\"How many rows are in the \
             customers table?\"* — or type `/help`.",
            true,
        ))])
    }
}
