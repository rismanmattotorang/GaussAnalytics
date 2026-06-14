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
use gauss_auth::{verify_password, PermissionSet, Session};
use gauss_core::domain::User;
use gauss_core::error::{CoreError, CoreResult};
use uuid::Uuid;

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

/// The credential a request presents as an API key: the `X-API-Key` header, or
/// failing that the `Authorization: Bearer` value.
pub fn presented_api_key(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-api-key").and_then(|h| h.to_str().ok()) {
        return Some(v.to_string());
    }
    bearer_token(headers)
}

/// Constant-time string comparison (length is not secret here).
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Whether `presented` matches any configured static API key.
pub fn api_key_valid(state: &AppState, presented: &str) -> bool {
    state
        .config
        .security
        .api_keys
        .iter()
        .any(|k| ct_eq(k, presented))
}

/// The synthetic administrator principal granted to valid API-key requests.
fn service_principal() -> CurrentUser {
    CurrentUser {
        user: User {
            id: Uuid::nil(),
            email: "service@apikey".into(),
            display_name: "API key (service)".into(),
            is_admin: true,
            created_at: Utc::now(),
        },
        perms: PermissionSet::admin(),
        token: String::new(),
    }
}

/// Resolve the current principal from the request headers, or fail with
/// [`CoreError::Unauthorized`]. A valid API key authenticates as a service
/// administrator; otherwise a session bearer token is required.
pub async fn authenticate(state: &AppState, headers: &HeaderMap) -> CoreResult<CurrentUser> {
    if let Some(key) = presented_api_key(headers) {
        // 1. Static service key (from configuration) → service administrator.
        if api_key_valid(state, &key) {
            return Ok(service_principal());
        }
        // 2. DB-backed API key → its owning user (with that user's grants).
        let hash = gauss_auth::hash_api_key(&key);
        if let Some(user_id) = state.store.api_key_user(&hash).await? {
            if let Some(user) = state.store.user_by_id(user_id).await? {
                let perms = perms_for_user(state, &user).await?;
                return Ok(CurrentUser {
                    user,
                    perms,
                    token: String::new(),
                });
            }
        }
    }

    // 3. Session bearer token.
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

    let perms = perms_for_user(state, &user).await?;
    Ok(CurrentUser { user, perms, token })
}

/// Build a [`PermissionSet`] for `user`: administrators hold everything; other
/// users get their persisted grants.
pub async fn perms_for_user(state: &AppState, user: &User) -> CoreResult<PermissionSet> {
    if user.is_admin {
        return Ok(PermissionSet::admin());
    }
    let mut set = PermissionSet::empty();
    for perm in state.store.grants_for(user.id).await? {
        set.grant(perm);
    }
    Ok(set)
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
