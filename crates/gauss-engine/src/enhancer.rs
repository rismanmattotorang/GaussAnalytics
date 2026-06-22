//! Default LLM context enhancer: lightweight RAG over saved text memories.
//! Mirrors `gauss/core/enhancer/default.py` (`DefaultLlmContextEnhancer`).

use crate::context::ToolContext;
use crate::model::user::User;
use crate::traits::{AgentMemory, LlmContextEnhancer};
use async_trait::async_trait;
use std::sync::Arc;

/// Enriches the system prompt with text memories relevant to the user's
/// message. Failures are swallowed so RAG never breaks a request.
pub struct DefaultLlmContextEnhancer {
    agent_memory: Arc<dyn AgentMemory>,
    limit: usize,
    similarity_threshold: f32,
}

impl DefaultLlmContextEnhancer {
    pub fn new(agent_memory: Arc<dyn AgentMemory>) -> Self {
        Self {
            agent_memory,
            limit: 5,
            similarity_threshold: 0.1,
        }
    }

    pub fn with_params(mut self, limit: usize, similarity_threshold: f32) -> Self {
        self.limit = limit;
        self.similarity_threshold = similarity_threshold;
        self
    }
}

#[async_trait]
impl LlmContextEnhancer for DefaultLlmContextEnhancer {
    async fn enhance_system_prompt(
        &self,
        system_prompt: String,
        user_message: &str,
        user: &User,
    ) -> String {
        // A minimal context is sufficient for the in-memory/most backends.
        let ctx = ToolContext::new(
            user.clone(),
            "rag-enhance",
            "rag-enhance",
            self.agent_memory.clone(),
        );
        let results = self
            .agent_memory
            .search_text_memories(user_message, &ctx, self.limit, self.similarity_threshold)
            .await;
        match results {
            Ok(matches) if !matches.is_empty() => {
                let mut out = system_prompt;
                out.push_str("\n\n## Relevant Context from Memory\n");
                for m in matches {
                    out.push_str(&format!("- {}\n", m.memory.content));
                }
                out
            }
            _ => system_prompt,
        }
    }
}
