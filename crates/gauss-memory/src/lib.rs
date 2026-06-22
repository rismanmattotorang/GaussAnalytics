//! Vector-backed [`AgentMemory`](gauss_engine::traits::AgentMemory) implementations.
//!
//! - [`InMemoryVectorMemory`] — embeds + cosine-searches in process (offline,
//!   the verified default for vector memory).
//! - `QdrantAgentMemory` (feature `qdrant`) — persists vectors in a Qdrant
//!   collection over its REST API.

mod in_memory_vector;
pub use in_memory_vector::InMemoryVectorMemory;

#[cfg(feature = "qdrant")]
mod qdrant;
#[cfg(feature = "qdrant")]
pub use qdrant::QdrantAgentMemory;
