//! A lightweight tabular result type.
//!
//! Phase 1 deliberately avoids a `polars` dependency: SQL results only need to
//! flow into a `DataFrameComponent` (rows + columns) and to CSV. Phase 2 can
//! swap this for `polars::DataFrame` behind the same boundary if richer
//! dataframe operations (joins, aggregations for charting) become necessary.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataFrame {
    pub columns: Vec<String>,
    /// Row-major data; each inner vec aligns with `columns`.
    pub rows: Vec<Vec<Value>>,
}

impl DataFrame {
    pub fn new(columns: Vec<String>, rows: Vec<Vec<Value>>) -> Self {
        Self { columns, rows }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Convert to a vec of `{column: value}` maps — the shape consumed by
    /// `DataFrameComponent::from_records` and by JSON serialization.
    pub fn to_records(&self) -> Vec<Map<String, Value>> {
        self.rows
            .iter()
            .map(|row| {
                self.columns
                    .iter()
                    .cloned()
                    .zip(row.iter().cloned())
                    .collect()
            })
            .collect()
    }

    /// Render as CSV text (header row + data rows). Values are stringified;
    /// strings are emitted without surrounding JSON quotes.
    pub fn to_csv(&self) -> String {
        fn cell(v: &Value) -> String {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            if s.contains([',', '"', '\n']) {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s
            }
        }
        let mut out = String::new();
        out.push_str(&self.columns.join(","));
        out.push('\n');
        for row in &self.rows {
            let line: Vec<String> = row.iter().map(cell).collect();
            out.push_str(&line.join(","));
            out.push('\n');
        }
        out
    }
}
