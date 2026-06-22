//! Observability primitives (spans & metrics).
//! Mirrors `gauss/core/observability/models.py`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use uuid::Uuid;

fn now_secs() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// A tracing span. Times are unix-epoch seconds (matching the Python source).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    #[serde(default = "new_id")]
    pub id: String,
    pub name: String,
    #[serde(default = "now_secs")]
    pub start_time: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<f64>,
    #[serde(default)]
    pub attributes: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

impl Span {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            start_time: now_secs(),
            end_time: None,
            attributes: Map::new(),
            parent_id: None,
        }
    }

    pub fn end(&mut self) {
        if self.end_time.is_none() {
            self.end_time = Some(now_secs());
        }
    }

    pub fn duration_ms(&self) -> Option<f64> {
        self.end_time.map(|e| (e - self.start_time) * 1000.0)
    }

    pub fn set_attribute(&mut self, key: impl Into<String>, value: Value) {
        self.attributes.insert(key.into(), value);
    }
}

/// A single metric sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub value: f64,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    #[serde(default = "now_secs")]
    pub timestamp: f64,
}
