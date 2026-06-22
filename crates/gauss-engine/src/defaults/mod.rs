//! Zero-dependency default implementations of core traits, suitable for
//! development and single-tenant deployments.

pub mod audit;
pub mod file_system;
pub mod memory;
pub mod observability;
pub mod store;
pub mod store_file;
pub mod user;

pub use audit::{FileAuditLogger, InMemoryAuditLogger};
pub use file_system::LocalFileSystem;
pub use memory::InMemoryAgentMemory;
pub use observability::{CapturingObservabilityProvider, TracingObservabilityProvider};
pub use store::InMemoryConversationStore;
pub use store_file::FileConversationStore;
pub use user::{HeaderUserResolver, StaticUserResolver};
