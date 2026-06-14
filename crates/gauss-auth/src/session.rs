//! Opaque, server-side sessions.
//!
//! A session token is a high-entropy opaque string; the server stores the
//! session and looks it up on each request. We deliberately avoid putting
//! authorization claims inside the token so that revocation is immediate and
//! a leaked token reveals nothing about the user.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An authenticated session bound to a user, with an expiry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub token: String,
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl Session {
    /// Create a fresh session for `user_id` valid for `ttl_secs` from `now`.
    pub fn new(user_id: Uuid, ttl_secs: i64, now: DateTime<Utc>) -> Self {
        Self {
            token: generate_token(),
            user_id,
            created_at: now,
            expires_at: now + Duration::seconds(ttl_secs),
        }
    }

    /// Whether the session is still valid at `now`.
    pub fn is_valid_at(&self, now: DateTime<Utc>) -> bool {
        now < self.expires_at
    }
}

/// Generate a ~244-bit opaque token (two v4 UUIDs, hyphen-free).
fn generate_token() -> String {
    let a = Uuid::new_v4().simple().to_string();
    let b = Uuid::new_v4().simple().to_string();
    format!("{a}{b}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_long() {
        let s1 = Session::new(Uuid::new_v4(), 60, Utc::now());
        let s2 = Session::new(Uuid::new_v4(), 60, Utc::now());
        assert_ne!(s1.token, s2.token);
        assert_eq!(s1.token.len(), 64);
    }

    #[test]
    fn expiry_is_respected() {
        let now = Utc::now();
        let s = Session::new(Uuid::new_v4(), 10, now);
        assert!(s.is_valid_at(now));
        assert!(!s.is_valid_at(now + Duration::seconds(11)));
    }
}
