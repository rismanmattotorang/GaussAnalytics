//! Default observability providers.
//!
//! - [`TracingObservabilityProvider`] emits spans/metrics via the `tracing`
//!   crate (so any `tracing-subscriber` sink picks them up).
//! - [`CapturingObservabilityProvider`] records spans/metrics in memory for
//!   tests and assertions.

use crate::model::observability::Span;
use crate::traits::ObservabilityProvider;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

/// Bridges observability calls to `tracing` events.
#[derive(Default)]
pub struct TracingObservabilityProvider;

#[async_trait]
impl ObservabilityProvider for TracingObservabilityProvider {
    async fn record_metric(
        &self,
        name: &str,
        value: f64,
        unit: &str,
        _tags: Option<&HashMap<String, String>>,
    ) {
        tracing::debug!(metric = name, value, unit, "metric");
    }

    async fn end_span(&self, span: &mut Span) {
        span.end();
        tracing::debug!(span = span.name, duration_ms = span.duration_ms(), "span");
    }
}

/// Records spans and metrics in memory.
#[derive(Default)]
pub struct CapturingObservabilityProvider {
    spans: RwLock<Vec<String>>,
    metrics: RwLock<Vec<(String, f64)>>,
}

impl CapturingObservabilityProvider {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn span_names(&self) -> Vec<String> {
        self.spans.read().unwrap().clone()
    }
    pub fn metrics(&self) -> Vec<(String, f64)> {
        self.metrics.read().unwrap().clone()
    }
}

#[async_trait]
impl ObservabilityProvider for CapturingObservabilityProvider {
    async fn record_metric(
        &self,
        name: &str,
        value: f64,
        _unit: &str,
        _tags: Option<&HashMap<String, String>>,
    ) {
        self.metrics
            .write()
            .unwrap()
            .push((name.to_string(), value));
    }

    async fn create_span(&self, name: &str) -> Span {
        self.spans.write().unwrap().push(name.to_string());
        Span::new(name)
    }

    async fn end_span(&self, span: &mut Span) {
        span.end();
    }
}
