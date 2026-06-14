//! Authentication helpers for the HTTP layer: login, logout, and resolving the
//! current user from a bearer token.
//!
//! Rather than a custom extractor (whose trait signature shifts across axum
//! versions), handlers that need identity take an [`axum::http::HeaderMap`] and
//! call [`authenticate`]. This keeps auth explicit and easy to test.

use crate::state::AppState;
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;
use chrono::Utc;
use gauss_auth::{verify_password, PermissionSet, Role, Session};
use gauss_core::domain::User;
use gauss_core::error::{CoreError, CoreResult};

/// An authenticated principal plus the permissions resolved for this request.
pub struct CurrentUser {
    pub user: User,
    pub perms: PermissionSet,
    pub token: String,
}

/// Extract a `Bearer <token>` value from the `Authorization` header.
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_string)
}

/// Resolve the current user from the request headers, or fail with
/// [`CoreError::Unauthorized`].
pub async fn authenticate(state: &AppState, headers: &HeaderMap) -> CoreResult<CurrentUser> {
    let token = bearer_token(headers)
        .ok_or_else(|| CoreError::Unauthorized("missing bearer token".into()))?;

    let session = state
        .store
        .session_by_token(&token)
        .await?
        .ok_or_else(|| CoreError::Unauthorized("invalid session".into()))?;

    if !session.is_valid_at(Utc::now()) {
        return Err(CoreError::Unauthorized("session expired".into()));
    }

    let user = state
        .store
        .user_by_id(session.user_id)
        .await?
        .ok_or_else(|| CoreError::Unauthorized("session user no longer exists".into()))?;

    // Until per-entity grants are persisted, admins get everything and other
    // users get the Viewer baseline.
    let perms = if user.is_admin {
        PermissionSet::admin()
    } else {
        PermissionSet::for_role(Role::Viewer)
    };

    Ok(CurrentUser { user, perms, token })
}

/// Verify credentials and create + persist a new session.
pub async fn login(state: &AppState, email: &str, password: &str) -> CoreResult<Session> {
    let invalid = || CoreError::Unauthorized("invalid credentials".into());

    let hash = state
        .store
        .password_hash(email)
        .await?
        .ok_or_else(invalid)?;
    if !verify_password(password, &hash)? {
        return Err(invalid());
    }
    let user = state
        .store
        .user_by_email(email)
        .await?
        .ok_or_else(invalid)?;

    let ttl = state.config.security.session_ttl_secs as i64;
    let session = Session::new(user.id, ttl, Utc::now());
    state.store.insert_session(session.clone()).await?;
    Ok(session)
}
