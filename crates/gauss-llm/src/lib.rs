//! LLM service implementations for GaussAnalytics.
//!
//! - [`MockLlmService`] — deterministic, no external calls (dev/testing).
//! - [`OllamaLlmService`] — local/self-hosted models via Ollama.
//! - [`OpenAiLlmService`] — OpenAI Chat Completions (and OpenAI-compatible /
//!   Azure endpoints via `with_base_url`).
//! - [`AnthropicLlmService`] — Anthropic Claude Messages API.
//!
//! All providers currently use non-streaming `send_request`; the agent wraps it
//! via the trait's default `stream_request`. Token-level streaming (with a
//! shared tool-call delta accumulator) is a planned refinement.

mod accumulator;
mod anthropic;
mod cache;
mod gemini;
mod mock;
mod ollama;
mod openai;
mod vllm;

pub use accumulator::ToolCallAccumulator;
pub use anthropic::AnthropicLlmService;
pub use cache::CachingLlmService;
pub use gemini::GeminiLlmService;
pub use mock::MockLlmService;
pub use ollama::OllamaLlmService;
pub use openai::OpenAiLlmService;
pub use vllm::VllmLlmService;
