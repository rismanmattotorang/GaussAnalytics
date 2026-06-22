//! Agent-memory tools: the search-before / save-after self-learning loop.
//! Mirrors `gauss/tools/agent_memory.py`.

use async_trait::async_trait;
use gauss_engine::components::{RichComponent, UiComponent};
use gauss_engine::context::{ToolContext, ToolResult};
use gauss_engine::tool::Tool;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

fn default_limit() -> usize {
    10
}
fn default_threshold() -> f32 {
    0.5
}

// ---- search_saved_correct_tool_uses ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchSavedToolUsesArgs {
    /// The natural-language question to find similar past tool uses for.
    pub question: String,
    /// Maximum number of matches to return.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Minimum similarity score (0.0–1.0) to include a match.
    #[serde(default = "default_threshold")]
    pub similarity_threshold: f32,
    /// Optional: restrict matches to a specific tool name.
    #[serde(default)]
    pub tool_name_filter: Option<String>,
}

/// Searches saved correct tool uses for questions similar to the current one.
#[derive(Default)]
pub struct SearchSavedCorrectToolUsesTool;

#[async_trait]
impl Tool for SearchSavedCorrectToolUsesTool {
    type Args = SearchSavedToolUsesArgs;

    fn name(&self) -> &str {
        "search_saved_correct_tool_uses"
    }
    fn description(&self) -> &str {
        "Search previously saved correct tool uses for questions similar to the user's, \
         so you can reuse the tool and arguments that worked before. Call this BEFORE running a query."
    }
    fn access_groups(&self) -> Vec<String> {
        vec!["user".into(), "admin".into()]
    }

    async fn execute(&self, context: &ToolContext, args: SearchSavedToolUsesArgs) -> ToolResult {
        let results = context
            .agent_memory
            .search_similar_usage(
                &args.question,
                context,
                args.limit,
                args.similarity_threshold,
                args.tool_name_filter.as_deref(),
            )
            .await;
        match results {
            Ok(matches) if matches.is_empty() => {
                ToolResult::success("No similar saved tool uses were found.")
            }
            Ok(matches) => {
                let mut lines = Vec::new();
                for m in &matches {
                    lines.push(format!(
                        "- (score {:.2}) tool `{}` for \"{}\" with args {}",
                        m.similarity_score,
                        m.memory.tool_name,
                        m.memory.question,
                        Value::Object(m.memory.args.clone())
                    ));
                }
                let body = lines.join("\n");
                ToolResult::success(format!(
                    "Found {} similar saved tool use(s):\n{body}",
                    matches.len()
                ))
                .with_ui(UiComponent::new(RichComponent::card(
                    "Relevant past queries",
                    body,
                    true,
                )))
            }
            Err(e) => ToolResult::error(format!("Memory search failed: {e}")),
        }
    }
}

// ---- save_question_tool_args ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveQuestionToolArgsArgs {
    /// The user's question this tool use answered.
    pub question: String,
    /// The name of the tool that was used.
    pub tool_name: String,
    /// The arguments (JSON object) that were passed to the tool.
    pub args: Value,
}

/// Saves a successful question→tool-args pair to memory for future reuse.
#[derive(Default)]
pub struct SaveQuestionToolArgsTool;

#[async_trait]
impl Tool for SaveQuestionToolArgsTool {
    type Args = SaveQuestionToolArgsArgs;

    fn name(&self) -> &str {
        "save_question_tool_args"
    }
    fn description(&self) -> &str {
        "Save a successful question and the tool + arguments that answered it, so similar \
         questions can reuse it later. Call this AFTER a tool call succeeds."
    }
    fn access_groups(&self) -> Vec<String> {
        vec!["admin".into()]
    }

    async fn execute(&self, context: &ToolContext, args: SaveQuestionToolArgsArgs) -> ToolResult {
        let tool_args: Map<String, Value> = args.args.as_object().cloned().unwrap_or_else(Map::new);
        match context
            .agent_memory
            .save_tool_usage(
                &args.question,
                &args.tool_name,
                &tool_args,
                context,
                true,
                None,
            )
            .await
        {
            Ok(()) => ToolResult::success(format!(
                "Saved tool use for question: \"{}\".",
                args.question
            ))
            .with_ui(UiComponent::new(RichComponent::status_bar(
                "idle",
                "Saved to memory",
                Some(format!("Remembered how to answer: {}", args.question)),
            ))),
            Err(e) => ToolResult::error(format!("Failed to save memory: {e}")),
        }
    }
}

// ---- save_text_memory ----

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveTextMemoryArgs {
    /// The domain knowledge to remember (schema notes, terminology, business rules).
    pub content: String,
}

/// Saves free-form domain knowledge as a searchable text memory.
#[derive(Default)]
pub struct SaveTextMemoryTool;

#[async_trait]
impl Tool for SaveTextMemoryTool {
    type Args = SaveTextMemoryArgs;

    fn name(&self) -> &str {
        "save_text_memory"
    }
    fn description(&self) -> &str {
        "Save durable domain knowledge (table/column meanings, terminology, business rules) \
         as a text memory that will enrich future answers."
    }
    fn access_groups(&self) -> Vec<String> {
        vec!["admin".into()]
    }

    async fn execute(&self, context: &ToolContext, args: SaveTextMemoryArgs) -> ToolResult {
        match context
            .agent_memory
            .save_text_memory(&args.content, context)
            .await
        {
            Ok(mem) => ToolResult::success(format!(
                "Saved text memory{}.",
                mem.memory_id
                    .map(|id| format!(" (id {id})"))
                    .unwrap_or_default()
            )),
            Err(e) => ToolResult::error(format!("Failed to save text memory: {e}")),
        }
    }
}
