//! vLLM `LlmService` for on-prem / self-hosted models.
//!
//! vLLM exposes an **OpenAI-compatible** server (`/v1/chat/completions`), so
//! this composes the tested [`OpenAiLlmService`] pointed at the vLLM endpoint.
//! A key is usually not required; vLLM accepts the conventional `EMPTY` token.

use crate::openai::OpenAiLlmService;
use async_trait::async_trait;
use gauss_engine::error::Result;
use gauss_engine::model::llm::{LlmRequest, LlmResponse};
use gauss_engine::traits::{LlmChunkStream, LlmService};

pub struct VllmLlmService {
    inner: OpenAiLlmService,
}

impl VllmLlmService {
    /// `base_url` is the OpenAI-compatible root, e.g. `http://localhost:8000/v1`.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            inner: OpenAiLlmService::new("EMPTY", model).with_base_url(base_url),
        }
    }

    /// Use a bearer token if the vLLM server is configured with `--api-key`.
    pub fn with_api_key(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            inner: OpenAiLlmService::new(api_key, model).with_base_url(base_url),
        }
    }
}

#[async_trait]
impl LlmService for VllmLlmService {
    async fn send_request(&self, request: LlmRequest) -> Result<LlmResponse> {
        self.inner.send_request(request).await
    }

    async fn stream_request<'a>(&'a self, request: LlmRequest) -> LlmChunkStream<'a> {
        self.inner.stream_request(request).await
    }

    fn model(&self) -> &str {
        self.inner.model()
    }
}
