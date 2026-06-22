//! A trivial `UserResolver` for development and single-tenant deployments.

use crate::model::user::{RequestContext, User};
use crate::traits::UserResolver;
use crate::Result;
use async_trait::async_trait;

/// Resolves every request to the same configured user. Useful for local
/// development and demos; replace with a JWT/cookie resolver in production.
pub struct StaticUserResolver {
    user: User,
}

impl StaticUserResolver {
    pub fn new(user: User) -> Self {
        Self { user }
    }

    /// A default admin user (`id = "local"`, groups = [user, admin]).
    pub fn admin() -> Self {
        Self {
            user: User::new("local")
                .with_email("local@gaussanalytics.dev")
                .with_groups(["user", "admin"]),
        }
    }
}

#[async_trait]
impl UserResolver for StaticUserResolver {
    async fn resolve_user(&self, _request_context: &RequestContext) -> Result<User> {
        Ok(self.user.clone())
    }
}

/// Resolves a `User` from request headers — the building block for integrating
/// with an upstream auth gateway / API gateway that injects identity headers
/// (e.g. after validating a JWT or session).
///
/// Defaults read `x-user-id`, `x-user-email`, and a comma-separated
/// `x-user-groups`. The user id header is required; a missing/empty value is
/// rejected so unauthenticated requests cannot proceed.
pub struct HeaderUserResolver {
    pub id_header: String,
    pub email_header: String,
    pub groups_header: String,
    /// Groups granted to every resolved user (e.g. a baseline `user` group).
    pub default_groups: Vec<String>,
}

impl Default for HeaderUserResolver {
    fn default() -> Self {
        Self {
            id_header: "x-user-id".into(),
            email_header: "x-user-email".into(),
            groups_header: "x-user-groups".into(),
            default_groups: vec!["user".into()],
        }
    }
}

impl HeaderUserResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_headers(
        id_header: impl Into<String>,
        email_header: impl Into<String>,
        groups_header: impl Into<String>,
    ) -> Self {
        Self {
            id_header: id_header.into(),
            email_header: email_header.into(),
            groups_header: groups_header.into(),
            default_groups: vec!["user".into()],
        }
    }

    pub fn with_default_groups(mut self, groups: Vec<String>) -> Self {
        self.default_groups = groups;
        self
    }
}

#[async_trait]
impl UserResolver for HeaderUserResolver {
    async fn resolve_user(&self, ctx: &RequestContext) -> Result<User> {
        let id = ctx
            .get_header(&self.id_header)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                crate::AgentError::Permission(format!(
                    "missing required identity header `{}`",
                    self.id_header
                ))
            })?;

        let mut groups = self.default_groups.clone();
        if let Some(raw) = ctx.get_header(&self.groups_header) {
            for g in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if !groups.iter().any(|x| x == g) {
                    groups.push(g.to_string());
                }
            }
        }

        let mut user = User::new(id).with_groups(groups);
        if let Some(email) = ctx
            .get_header(&self.email_header)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            user = user.with_email(email);
        }
        Ok(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(headers: &[(&str, &str)]) -> RequestContext {
        let mut ctx = RequestContext::default();
        for (k, v) in headers {
            ctx.headers.insert(k.to_string(), v.to_string());
        }
        ctx
    }

    #[tokio::test]
    async fn resolves_from_headers() {
        let r = HeaderUserResolver::new();
        let user = r
            .resolve_user(&req(&[
                ("x-user-id", "alice"),
                ("x-user-email", "alice@example.com"),
                ("x-user-groups", "admin, analytics"),
            ]))
            .await
            .unwrap();
        assert_eq!(user.id, "alice");
        assert_eq!(user.email.as_deref(), Some("alice@example.com"));
        assert!(user.group_memberships.contains(&"user".to_string()));
        assert!(user.group_memberships.contains(&"admin".to_string()));
        assert!(user.group_memberships.contains(&"analytics".to_string()));
    }

    #[tokio::test]
    async fn rejects_missing_identity() {
        let r = HeaderUserResolver::new();
        assert!(r.resolve_user(&req(&[])).await.is_err());
    }
}
