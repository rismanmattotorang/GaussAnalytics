//! User identity & request context models.
//! Mirrors `gauss/core/user/models.py` and `request_context.py`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// An authenticated user. Identity and group memberships flow through every
/// layer of the agent for permission checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub group_memberships: Vec<String>,
}

impl User {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            username: None,
            email: None,
            metadata: Map::new(),
            group_memberships: Vec::new(),
        }
    }

    pub fn with_groups(mut self, groups: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.group_memberships = groups.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }

    /// True if the user belongs to any of the given access groups.
    /// An empty `access_groups` means "no restriction" (everyone allowed).
    pub fn can_access(&self, access_groups: &[String]) -> bool {
        if access_groups.is_empty() {
            return true;
        }
        access_groups
            .iter()
            .any(|g| self.group_memberships.contains(g))
    }
}

/// HTTP-level context used by a `UserResolver` to identify the user.
/// Mirrors `RequestContext` in `gauss/core/user/request_context.py`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestContext {
    #[serde(default)]
    pub cookies: HashMap<String, String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_addr: Option<String>,
    #[serde(default)]
    pub query_params: HashMap<String, String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

impl RequestContext {
    /// Case-insensitive header lookup (HTTP headers are case-insensitive).
    pub fn get_header(&self, name: &str) -> Option<&str> {
        let lname = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase() == lname)
            .map(|(_, v)| v.as_str())
    }

    pub fn get_cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }
}
