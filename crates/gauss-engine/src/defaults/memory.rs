//! In-memory `AgentMemory` with cheap lexical similarity — the zero-dependency
//! default. Mirrors GaussAnalytics's `DemoAgentMemory` (`integrations/local`).
//!
//! Phase 2 introduces vector-backed memory (`pt-memory`) with real embeddings;
//! this keeps development friction-free with no external services.

use crate::context::ToolContext;
use crate::model::memory::{
    TextMemory, TextMemorySearchResult, ToolMemory, ToolMemorySearchResult,
};
use crate::traits::AgentMemory;
use crate::Result;
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct InMemoryAgentMemory {
    tool_memories: RwLock<Vec<ToolMemory>>,
    text_memories: RwLock<Vec<TextMemory>>,
    /// Optional cap; oldest entries are evicted first.
    max_items: Option<usize>,
}

impl InMemoryAgentMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(max_items: usize) -> Self {
        Self {
            max_items: Some(max_items),
            ..Default::default()
        }
    }
}

fn tokenize(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Jaccard similarity over word sets, in `0.0..=1.0`.
fn similarity(a: &str, b: &str) -> f32 {
    let (sa, sb) = (tokenize(a), tokenize(b));
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    if union == 0 {
        0.0
    } else {
        inter as f32 / union as f32
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[async_trait]
impl AgentMemory for InMemoryAgentMemory {
    async fn save_tool_usage(
        &self,
        question: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        _context: &ToolContext,
        success: bool,
        metadata: Option<&Map<String, Value>>,
    ) -> Result<()> {
        let mut mems = self.tool_memories.write().unwrap();
        mems.push(ToolMemory {
            memory_id: Some(Uuid::new_v4().to_string()),
            question: question.to_string(),
            tool_name: tool_name.to_string(),
            args: args.clone(),
            timestamp: Some(now_iso()),
            success,
            metadata: metadata.cloned(),
        });
        if let Some(cap) = self.max_items {
            while mems.len() > cap {
                mems.remove(0);
            }
        }
        Ok(())
    }

    async fn save_text_memory(&self, content: &str, _context: &ToolContext) -> Result<TextMemory> {
        let mem = TextMemory {
            memory_id: Some(Uuid::new_v4().to_string()),
            content: content.to_string(),
            timestamp: Some(now_iso()),
        };
        let mut mems = self.text_memories.write().unwrap();
        mems.push(mem.clone());
        if let Some(cap) = self.max_items {
            while mems.len() > cap {
                mems.remove(0);
            }
        }
        Ok(mem)
    }

    async fn search_similar_usage(
        &self,
        question: &str,
        _context: &ToolContext,
        limit: usize,
        similarity_threshold: f32,
        tool_name_filter: Option<&str>,
    ) -> Result<Vec<ToolMemorySearchResult>> {
        let mems = self.tool_memories.read().unwrap();
        let mut scored: Vec<(f32, ToolMemory)> = mems
            .iter()
            .filter(|m| m.success)
            .filter(|m| tool_name_filter.is_none_or(|f| m.tool_name == f))
            .map(|m| (similarity(question, &m.question), m.clone()))
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
        query: &str,
        _context: &ToolContext,
        limit: usize,
        similarity_threshold: f32,
    ) -> Result<Vec<TextMemorySearchResult>> {
        let mems = self.text_memories.read().unwrap();
        let mut scored: Vec<(f32, TextMemory)> = mems
            .iter()
            .map(|m| (similarity(query, &m.content), m.clone()))
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
        let mems = self.tool_memories.read().unwrap();
        Ok(mems.iter().rev().take(limit).cloned().collect())
    }

    async fn get_recent_text_memories(
        &self,
        _context: &ToolContext,
        limit: usize,
    ) -> Result<Vec<TextMemory>> {
        let mems = self.text_memories.read().unwrap();
        Ok(mems.iter().rev().take(limit).cloned().collect())
    }

    async fn delete_by_id(&self, _context: &ToolContext, memory_id: &str) -> Result<bool> {
        let mut mems = self.tool_memories.write().unwrap();
        let before = mems.len();
        mems.retain(|m| m.memory_id.as_deref() != Some(memory_id));
        Ok(mems.len() != before)
    }

    async fn delete_text_memory(&self, _context: &ToolContext, memory_id: &str) -> Result<bool> {
        let mut mems = self.text_memories.write().unwrap();
        let before = mems.len();
        mems.retain(|m| m.memory_id.as_deref() != Some(memory_id));
        Ok(mems.len() != before)
    }

    async fn clear_memories(
        &self,
        _context: &ToolContext,
        tool_name: Option<&str>,
        _before_date: Option<&str>,
    ) -> Result<usize> {
        let mut mems = self.tool_memories.write().unwrap();
        let before = mems.len();
        match tool_name {
            Some(t) => mems.retain(|m| m.tool_name != t),
            None => mems.clear(),
        }
        Ok(before - mems.len())
    }
}
