//! In-process vector memory: embeds questions/text and ranks by cosine
//! similarity. A drop-in [`AgentMemory`] upgrade over the lexical default that
//! needs no external service.

use async_trait::async_trait;
use gauss_embed::{dot, normalize, Embedder};
use gauss_engine::context::ToolContext;
use gauss_engine::error::Result;
use gauss_engine::model::memory::{
    TextMemory, TextMemorySearchResult, ToolMemory, ToolMemorySearchResult,
};
use gauss_engine::traits::AgentMemory;
use serde_json::{Map, Value};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

pub struct InMemoryVectorMemory {
    embedder: Arc<dyn Embedder>,
    tool: RwLock<Vec<(ToolMemory, Vec<f32>)>>,
    text: RwLock<Vec<(TextMemory, Vec<f32>)>>,
    max_items: Option<usize>,
}

impl InMemoryVectorMemory {
    pub fn new(embedder: Arc<dyn Embedder>) -> Self {
        Self {
            embedder,
            tool: RwLock::new(Vec::new()),
            text: RwLock::new(Vec::new()),
            max_items: None,
        }
    }

    pub fn with_capacity(mut self, max_items: usize) -> Self {
        self.max_items = Some(max_items);
        self
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[async_trait]
impl AgentMemory for InMemoryVectorMemory {
    async fn save_tool_usage(
        &self,
        question: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        _context: &ToolContext,
        success: bool,
        metadata: Option<&Map<String, Value>>,
    ) -> Result<()> {
        // Embed before locking (no await while holding the lock). Vectors are
        // unit-normalized at insert so search is a plain dot product.
        let mut vector = self.embedder.embed_one(question).await?;
        normalize(&mut vector);
        let memory = ToolMemory {
            memory_id: Some(Uuid::new_v4().to_string()),
            question: question.to_string(),
            tool_name: tool_name.to_string(),
            args: args.clone(),
            timestamp: Some(now_iso()),
            success,
            metadata: metadata.cloned(),
        };
        let mut store = self.tool.write().unwrap();
        store.push((memory, vector));
        if let Some(cap) = self.max_items {
            while store.len() > cap {
                store.remove(0);
            }
        }
        Ok(())
    }

    async fn save_text_memory(&self, content: &str, _context: &ToolContext) -> Result<TextMemory> {
        let mut vector = self.embedder.embed_one(content).await?;
        normalize(&mut vector);
        let memory = TextMemory {
            memory_id: Some(Uuid::new_v4().to_string()),
            content: content.to_string(),
            timestamp: Some(now_iso()),
        };
        let mut store = self.text.write().unwrap();
        store.push((memory.clone(), vector));
        if let Some(cap) = self.max_items {
            while store.len() > cap {
                store.remove(0);
            }
        }
        Ok(memory)
    }

    async fn search_similar_usage(
        &self,
        question: &str,
        _context: &ToolContext,
        limit: usize,
        similarity_threshold: f32,
        tool_name_filter: Option<&str>,
    ) -> Result<Vec<ToolMemorySearchResult>> {
        let mut query = self.embedder.embed_one(question).await?;
        normalize(&mut query);
        let store = self.tool.read().unwrap();
        let mut scored: Vec<(f32, ToolMemory)> = store
            .iter()
            .filter(|(m, _)| m.success)
            .filter(|(m, _)| tool_name_filter.is_none_or(|f| m.tool_name == f))
            .map(|(m, v)| (dot(&query, v), m.clone()))
            .filter(|(s, _)| *s >= similarity_threshold)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(i, (score, memory))| ToolMemorySearchResult {
                memory,
                similarity_score: score,
                rank: i as u32,
            })
            .collect())
    }

    async fn search_text_memories(
        &self,
        query_text: &str,
        _context: &ToolContext,
        limit: usize,
        similarity_threshold: f32,
    ) -> Result<Vec<TextMemorySearchResult>> {
        let mut query = self.embedder.embed_one(query_text).await?;
        normalize(&mut query);
        let store = self.text.read().unwrap();
        let mut scored: Vec<(f32, TextMemory)> = store
            .iter()
            .map(|(m, v)| (dot(&query, v), m.clone()))
            .filter(|(s, _)| *s >= similarity_threshold)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(i, (score, memory))| TextMemorySearchResult {
                memory,
                similarity_score: score,
                rank: i as u32,
            })
            .collect())
    }

    async fn get_recent_memories(
        &self,
        _context: &ToolContext,
        limit: usize,
    ) -> Result<Vec<ToolMemory>> {
        let store = self.tool.read().unwrap();
        Ok(store
            .iter()
            .rev()
            .take(limit)
            .map(|(m, _)| m.clone())
            .collect())
    }

    async fn get_recent_text_memories(
        &self,
        _context: &ToolContext,
        limit: usize,
    ) -> Result<Vec<TextMemory>> {
        let store = self.text.read().unwrap();
        Ok(store
            .iter()
            .rev()
            .take(limit)
            .map(|(m, _)| m.clone())
            .collect())
    }

    async fn delete_by_id(&self, _context: &ToolContext, memory_id: &str) -> Result<bool> {
        let mut store = self.tool.write().unwrap();
        let before = store.len();
        store.retain(|(m, _)| m.memory_id.as_deref() != Some(memory_id));
        Ok(store.len() != before)
    }

    async fn delete_text_memory(&self, _context: &ToolContext, memory_id: &str) -> Result<bool> {
        let mut store = self.text.write().unwrap();
        let before = store.len();
        store.retain(|(m, _)| m.memory_id.as_deref() != Some(memory_id));
        Ok(store.len() != before)
    }

    async fn clear_memories(
        &self,
        _context: &ToolContext,
        tool_name: Option<&str>,
        _before_date: Option<&str>,
    ) -> Result<usize> {
        let mut store = self.tool.write().unwrap();
        let before = store.len();
        match tool_name {
            Some(t) => store.retain(|(m, _)| m.tool_name != t),
            None => store.clear(),
        }
        Ok(before - store.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_embed::HashingEmbedder;

    fn ctx(mem: Arc<dyn AgentMemory>) -> ToolContext {
        ToolContext::new(gauss_engine::model::user::User::new("u"), "c", "r", mem)
    }

    #[tokio::test]
    async fn semantic_search_ranks_relevant_first() {
        let embedder: Arc<dyn Embedder> = Arc::new(HashingEmbedder::default());
        let mem = Arc::new(InMemoryVectorMemory::new(embedder));
        let context = ctx(mem.clone());

        let mut args = Map::new();
        args.insert("sql".into(), serde_json::json!("SELECT * FROM customers"));
        mem.save_tool_usage(
            "top customers by revenue",
            "run_sql",
            &args,
            &context,
            true,
            None,
        )
        .await
        .unwrap();
        mem.save_tool_usage(
            "count of pending orders",
            "run_sql",
            &args,
            &context,
            true,
            None,
        )
        .await
        .unwrap();

        let results = mem
            .search_similar_usage("customers ranked by revenue", &context, 5, 0.0, None)
            .await
            .unwrap();
        assert!(!results.is_empty());
        // The revenue/customers question should rank above the orders one.
        assert_eq!(results[0].memory.question, "top customers by revenue");
        assert_eq!(results[0].rank, 0);
    }

    #[tokio::test]
    async fn threshold_filters_out_unrelated() {
        let embedder: Arc<dyn Embedder> = Arc::new(HashingEmbedder::default());
        let mem = Arc::new(InMemoryVectorMemory::new(embedder));
        let context = ctx(mem.clone());
        mem.save_text_memory(
            "the customers table stores account lifetime value",
            &context,
        )
        .await
        .unwrap();

        // A wildly unrelated query with a high threshold returns nothing.
        let none = mem
            .search_text_memories("quantum chromodynamics lecture", &context, 5, 0.5)
            .await
            .unwrap();
        assert!(none.is_empty());
        // A related query returns the memory.
        let some = mem
            .search_text_memories("customers lifetime value", &context, 5, 0.05)
            .await
            .unwrap();
        assert!(!some.is_empty());
    }
}
