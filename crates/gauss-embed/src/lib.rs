//! Text-embedding providers for GaussAnalytics vector memory.
//!
//! The [`Embedder`] trait turns text into fixed-dimension vectors. Implementations:
//! - [`HashingEmbedder`] — deterministic, dependency-free feature hashing
//!   (offline; the default for dev and tests).
//! - [`OllamaEmbedder`] / [`OpenAiEmbedder`] — REST embedding APIs.

use async_trait::async_trait;
use gauss_engine::error::Result;

mod hashing;
mod ollama;
mod openai;

pub use hashing::HashingEmbedder;
pub use ollama::OllamaEmbedder;
pub use openai::OpenAiEmbedder;

/// Produces fixed-dimension embedding vectors for text.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// The dimensionality of vectors this embedder produces.
    fn dimension(&self) -> usize;

    /// Embed a batch of texts, preserving order.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Embed a single text (defaults to a one-element batch).
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut v = self.embed(std::slice::from_ref(&text.to_string())).await?;
        Ok(v.pop().unwrap_or_default())
    }
}

/// Cosine similarity of two equal-length vectors, in `-1.0..=1.0` (0 if either
/// is a zero vector or lengths differ).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Dot product of two equal-length vectors (0 if lengths differ). For two
/// unit-normalized vectors this equals cosine similarity, with no per-call norm
/// recomputation — the basis for fast normalized-vector search.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Scale `v` to unit length in place (no-op for a zero vector).
pub fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}
