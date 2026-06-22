//! Default system-prompt builder.
//! Mirrors `gauss/core/system_prompt/default.py`.

use crate::model::tool::ToolSchema;
use crate::model::user::User;
use crate::traits::SystemPromptBuilder;
use crate::Result;
use async_trait::async_trait;

/// Builds the agent's system prompt, dynamically appending memory-workflow
/// instructions when the relevant memory tools are available.
#[derive(Default)]
pub struct DefaultSystemPromptBuilder {
    base_prompt: Option<String>,
}

impl DefaultSystemPromptBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_prompt(base_prompt: impl Into<String>) -> Self {
        Self {
            base_prompt: Some(base_prompt.into()),
        }
    }
}

#[async_trait]
impl SystemPromptBuilder for DefaultSystemPromptBuilder {
    async fn build_system_prompt(
        &self,
        _user: &User,
        tools: &[ToolSchema],
    ) -> Result<Option<String>> {
        if let Some(base) = &self.base_prompt {
            return Ok(Some(base.clone()));
        }
        if tools.is_empty() {
            return Ok(None);
        }

        let today = chrono::Utc::now().format("%Y-%m-%d");
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        let has = |n: &str| tool_names.contains(&n);

        let mut prompt = format!(
            "You are GaussAnalytics, an AI data analyst assistant that helps users \
             answer questions about their data. Today's date is {today}.\n\n\
             Guidelines:\n\
             - Use the available tools to answer questions; do not invent data.\n\
             - When a tool returns a table or chart, summarize the insight rather \
             than repeating the raw rows.\n\
             - Be concise and accurate.\n\n\
             You have access to the following tools: {tools}.\n",
            tools = tool_names.join(", ")
        );

        if has("search_saved_correct_tool_uses") {
            prompt.push_str(
                "\nMemory workflow: BEFORE running a query, search saved correct \
                 tool uses for similar past questions and reuse what worked.\n",
            );
        }
        if has("save_question_tool_args") {
            prompt.push_str(
                "AFTER a tool call succeeds and answers the question, save the \
                 successful question→tool-args pair to memory.\n",
            );
        }
        if has("save_text_memory") {
            prompt.push_str(
                "Save durable domain knowledge (schema notes, terminology, \
                 business rules) as text memories when you learn them.\n",
            );
        }

        Ok(Some(prompt))
    }
}
