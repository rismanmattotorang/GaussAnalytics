//! `gauss-core` — the shared domain of GaussAnalytics.
//!
//! This crate is the dependency-free (web/DB/AI-free) heart of the platform: it
//! defines the domain entities, the [`gql`] query AST, and the workspace-wide
//! [`error::CoreError`]. Everything else — the server, the TUI, the query
//! compiler, the integration layers — builds on top of these types.
//!
//! GaussAnalytics is owned and operated by Gaussian Technologies.

#![forbid(unsafe_code)]
// Headroom for serde's derived trait resolution over the recursive `gql::Filter`
// enum (nested And/Or/Not).
#![recursion_limit = "256"]

pub mod domain;
pub mod error;
pub mod gql;

pub use error::{CoreError, CoreResult};
