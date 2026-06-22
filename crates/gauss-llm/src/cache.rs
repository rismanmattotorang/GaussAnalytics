//! A caching decorator over any [`LlmService`].
//!
//! Wraps an inner service and memoizes responses keyed by the request's
//! cache-relevant fields (model, system prompt, messages, tools, temperature,
//! max tokens). Identical requests — common with retries, repeated questions,
//! and deterministic (temperature 0) pipelines — are served from memory,
//! cutting both cost and latency. Because the trait's default `stream_request`
//! delegates to `send_request`, both call paths are cached.

use async_trait::async_trait;
use gauss_engine::error::Result;
use gauss_engine::model::llm::{LlmRequest, LlmResponse};
use gauss_engine::traits::LlmService;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

const DEFAULT_MAX_ENTRIES: usize = 1024;

pub struct CachingLlmService {
    inner: Arc<dyn LlmService>,
    cache: Mutex<HashMap<u64, LlmResponse>>,
    max_entries: usize,
}

impl CachingLlmService {
    pub fn new(inner: Arc<dyn LlmService>) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    #[must_use]
    pub fn with_capacity(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries.max(1);
        self
    }

    /// Number of cached entries (useful for diagnostics/tests).
    pub fn len(&self) -> usize {
        self.cache.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Hash the request's cache-relevant fields into a stable in-process key.
fn cache_key(model: &str, request: &LlmRequest) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut hasher);
    request.system_prompt.hash(&mut hasher);
    request.temperature.to_bits().hash(&mut hasher);
    request.max_tokens.hash(&mut hasher);
    serde_json::to_string(&request.messages)
        .unwrap_or_default()
        .hash(&mut hasher);
    serde_json::to_string(&request.tools)
        .unwrap_or_default()
        .hash(&mut hasher);
    hasher.finish()
}

#[async_trait]
impl LlmService for CachingLlmService {
    async fn send_request(&self, request: LlmRequest) -> Result<LlmResponse> {
        let key = cache_key(self.inner.model(), &request);
        if let Some(hit) = self.cache.lock().unwrap().get(&key).cloned() {
            return Ok(hit);
        }
        let response = self.inner.send_request(request).await?;
        let mut cache = self.cache.lock().unwrap();
        // Simple bound: clear when full (avoids unbounded growth without an LRU).
        if cache.len() >= self.max_entries {
            cache.clear();
        }
        cache.insert(key, response.clone());
        Ok(response)
    }

    fn model(&self) -> &str {
        self.inner.model()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_engine::model::llm::LlmMessage;
    use gauss_engine::model::user::User;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingLlm {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl LlmService for CountingLlm {
        async fn send_request(&self, _request: LlmRequest) -> Result<LlmResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(LlmResponse {
                content: Some("answer".into()),
                ..Default::default()
            })
        }
        fn model(&self) -> &str {
            "counting"
        }
    }

    fn req(text: &str) -> LlmRequest {
        LlmRequest {
            messages: vec![LlmMessage::new("user", text)],
            tools: None,
            user: User::new("u"),
            stream: false,
            temperature: 0.0,
            max_tokens: None,
            system_prompt: None,
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn identical_requests_hit_cache() {
        let inner = Arc::new(CountingLlm {
            calls: AtomicUsize::new(0),
        });
        let svc = CachingLlmService::new(inner.clone());

        let a = svc.send_request(req("hello")).await.unwrap();
        let b = svc.send_request(req("hello")).await.unwrap();
        assert_eq!(a.content, b.content);
        assert_eq!(
            inner.calls.load(Ordering::SeqCst),
            1,
            "second call should hit cache"
        );
        assert_eq!(svc.len(), 1);
    }

    #[tokio::test]
    async fn different_requests_miss() {
        let inner = Arc::new(CountingLlm {
            calls: AtomicUsize::new(0),
        });
        let svc = CachingLlmService::new(inner.clone());
        svc.send_request(req("one")).await.unwrap();
        svc.send_request(req("two")).await.unwrap();
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
        assert_eq!(svc.len(), 2);
    }
}
