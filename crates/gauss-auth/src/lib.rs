//! `gauss-auth` — authentication, sessions, and authorization.
//!
//! This crate holds the security primitives shared across GaussAnalytics:
//! Argon2id password hashing ([`password`]), opaque server-side sessions
//! ([`session`]), and the value-based role/permission model ([`perms`]).

#![forbid(unsafe_code)]

pub mod apikey;
pub mod embed;
pub mod password;
pub mod perms;
pub mod session;

pub use apikey::{generate_api_key, hash_api_key};
pub use embed::{sign_embed, verify_embed};
pub use password::{hash_password, verify_password};
pub use perms::{Permission, PermissionSet, Role};
pub use session::Session;
