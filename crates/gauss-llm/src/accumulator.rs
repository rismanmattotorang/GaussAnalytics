//! A reusable accumulator for streamed tool calls.
//!
//! Streaming chat APIs deliver tool calls as fragments keyed by an index: the
//! id and name usually arrive once, while the JSON arguments arrive in pieces
//! across many chunks. This collects those fragments and assembles complete
//! [`ToolCall`]s when the stream ends.

use gauss_engine::model::tool::ToolCall;
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Default)]
struct PartialCall {
    id: Option<String>,
    name: String,
    args: String,
}

#[derive(Default)]
pub struct ToolCallAccumulator {
    calls: BTreeMap<usize, PartialCall>,
}

impl ToolCallAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// Apply one streamed tool-call delta at `index`.
    pub fn push_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        args_fragment: Option<&str>,
    ) {
        let entry = self.calls.entry(index).or_default();
        if let Some(id) = id {
            entry.id = Some(id.to_string());
        }
        if let Some(name) = name {
            entry.name.push_str(name);
        }
        if let Some(frag) = args_fragment {
            entry.args.push_str(frag);
        }
    }

    /// Assemble the complete tool calls (in index order). Calls without a name
    /// are dropped; un-parseable argument JSON yields an empty object.
    pub fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_values()
            .filter(|c| !c.name.is_empty())
            .map(|c| {
                let arguments = serde_json::from_str::<Value>(&c.args)
                    .ok()
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                ToolCall {
                    id: c.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
                    name: c.name,
                    arguments,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_fragmented_tool_call() {
        let mut acc = ToolCallAccumulator::new();
        // id + name arrive first, then the arguments stream in fragments.
        acc.push_delta(0, Some("call_1"), Some("run_sql"), Some("{\"sql\":"));
        acc.push_delta(0, None, None, Some("\"SELECT "));
        acc.push_delta(0, None, None, Some("1\"}"));
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "run_sql");
        assert_eq!(calls[0].arguments["sql"], "SELECT 1");
    }

    #[test]
    fn handles_multiple_indexed_calls() {
        let mut acc = ToolCallAccumulator::new();
        acc.push_delta(0, Some("a"), Some("t0"), Some("{}"));
        acc.push_delta(1, Some("b"), Some("t1"), Some("{}"));
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "t0");
        assert_eq!(calls[1].name, "t1");
    }
}
