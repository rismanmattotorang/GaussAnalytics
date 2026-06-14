//! Roles and the permission model.
//!
//! Authorization is value-based: a request resolves a [`PermissionSet`], and
//! protected operations call [`PermissionSet::require`] before doing work. The
//! server gates query execution this way, so "did we check permission?" is an
//! explicit, testable step rather than a convention.

use std::collections::HashSet;

use gauss_core::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Coarse-grained roles. Finer access is expressed via [`Permission`] grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

/// A single capability, optionally scoped to an entity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Permission {
    /// Change platform-wide settings.
    ManageSettings,
    /// Create or edit content (cards, dashboards).
    CreateContent,
    /// Read/query a specific connected database.
    ReadDatabase { database_id: Uuid },
    /// Read content within a specific collection.
    ReadCollection { collection_id: Uuid },
}

impl Permission {
    /// Decompose into a `(kind, scope)` pair for storage.
    pub fn to_parts(&self) -> (&'static str, Option<Uuid>) {
        match self {
            Permission::ManageSettings => ("manage_settings", None),
            Permission::CreateContent => ("create_content", None),
            Permission::ReadDatabase { database_id } => ("read_database", Some(*database_id)),
            Permission::ReadCollection { collection_id } => {
                ("read_collection", Some(*collection_id))
            }
        }
    }

    /// Reconstruct from a stored `(kind, scope)` pair, or `None` if unknown.
    pub fn from_parts(kind: &str, scope: Option<Uuid>) -> Option<Permission> {
        match (kind, scope) {
            ("manage_settings", _) => Some(Permission::ManageSettings),
            ("create_content", _) => Some(Permission::CreateContent),
            ("read_database", Some(id)) => Some(Permission::ReadDatabase { database_id: id }),
            ("read_collection", Some(id)) => Some(Permission::ReadCollection { collection_id: id }),
            _ => None,
        }
    }
}

/// The set of capabilities held by a principal for the duration of a request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet {
    /// Administrators implicitly hold every permission.
    is_admin: bool,
    grants: HashSet<Permission>,
}

impl PermissionSet {
    /// A set that allows everything.
    pub fn admin() -> Self {
        Self {
            is_admin: true,
            grants: HashSet::new(),
        }
    }

    /// An empty set (allows nothing).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a default permission set for `role`.
    pub fn for_role(role: Role) -> Self {
        match role {
            Role::Admin => Self::admin(),
            Role::Editor => {
                let mut s = Self::empty();
                s.grant(Permission::CreateContent);
                s
            }
            Role::Viewer => Self::empty(),
        }
    }

    /// Add a grant.
    pub fn grant(&mut self, perm: Permission) -> &mut Self {
        self.grants.insert(perm);
        self
    }

    /// Whether this set allows `perm`.
    pub fn allows(&self, perm: &Permission) -> bool {
        self.is_admin || self.grants.contains(perm)
    }

    /// Require `perm`, returning [`CoreError::PermissionDenied`] if absent.
    pub fn require(&self, perm: Permission) -> CoreResult<()> {
        if self.allows(&perm) {
            Ok(())
        } else {
            Err(CoreError::PermissionDenied(format!("{perm:?}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_allows_everything() {
        let p = PermissionSet::admin();
        assert!(p.require(Permission::ManageSettings).is_ok());
        assert!(p
            .require(Permission::ReadDatabase {
                database_id: Uuid::new_v4()
            })
            .is_ok());
    }

    #[test]
    fn viewer_cannot_create_content() {
        let p = PermissionSet::for_role(Role::Viewer);
        assert!(p.require(Permission::CreateContent).is_err());
    }

    #[test]
    fn scoped_grant_is_specific() {
        let db = Uuid::new_v4();
        let other = Uuid::new_v4();
        let mut p = PermissionSet::empty();
        p.grant(Permission::ReadDatabase { database_id: db });
        assert!(p
            .require(Permission::ReadDatabase { database_id: db })
            .is_ok());
        assert!(p
            .require(Permission::ReadDatabase { database_id: other })
            .is_err());
    }
}
