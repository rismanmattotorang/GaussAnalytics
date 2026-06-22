//! A deterministic, offline embedder using the feature-hashing ("hashing
//! trick") bag-of-words. Similar token sets map to nearby unit vectors, so
//! cosine similarity tracks lexical overlap — good enough for dev and tests
//! without any model or network. Uses FNV-1a so output is stable across runs
//! and Rust versions.

use crate::Embedder;
use async_trait::async_trait;
use gauss_engine::error::Result;

const DEFAULT_DIM: usize = 256;

pub struct HashingEmbedder {
    dim: usize,
}

impl Default for HashingEmbedder {
    fn default() -> Self {
        Self { dim: DEFAULT_DIM }
    }
}

impl HashingEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(1) }
    }

    fn embed_text(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dim];
        for token in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            let h = fnv1a(&token.to_lowercase());
            let idx = (h % self.dim as u64) as usize;
            // Use the top bit for a signed contribution to reduce collisions.
            let sign = if (h >> 63) & 1 == 0 { 1.0 } else { -1.0 };
            v[idx] += sign;
        }
        // L2-normalize.
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[async_trait]
impl Embedder for HashingEmbedder {
    fn dimension(&self) -> usize {
        self.dim
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed_text(t)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosine_similarity;

    #[tokio::test]
    async fn similar_texts_are_closer_than_unrelated() {
        let e = HashingEmbedder::default();
        let a = e.embed_one("top customers by revenue").await.unwrap();
        let b = e.embed_one("customers ranked by revenue").await.unwrap();
        let c = e.embed_one("weather forecast tomorrow").await.unwrap();
        assert_eq!(a.len(), 256);
        let ab = cosine_similarity(&a, &b);
        let ac = cosine_similarity(&a, &c);
        assert!(ab > ac, "similar pair {ab} should beat unrelated {ac}");
    }

    #[tokio::test]
    async fn deterministic() {
        let e = HashingEmbedder::default();
        let a = e.embed_one("hello world").await.unwrap();
        let b = e.embed_one("hello world").await.unwrap();
        assert_eq!(a, b);
    }
}
