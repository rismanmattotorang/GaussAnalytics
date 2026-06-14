//! Password hashing and verification using Argon2id.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use gauss_core::error::{CoreError, CoreResult};
use rand_core::OsRng;

/// Minimum acceptable password length (a basic guard; policy can tighten this).
pub const MIN_PASSWORD_LEN: usize = 10;

/// Hash a plaintext password, returning a PHC-format string safe to persist.
pub fn hash_password(plaintext: &str) -> CoreResult<String> {
    if plaintext.chars().count() < MIN_PASSWORD_LEN {
        return Err(CoreError::Unauthorized(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plaintext.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| CoreError::Internal(format!("password hashing failed: {e}")))
}

/// Verify a plaintext password against a stored PHC hash.
///
/// Returns `Ok(true)`/`Ok(false)` for a match/mismatch, and `Err` only if the
/// stored hash is malformed.
pub fn verify_password(plaintext: &str, phc_hash: &str) -> CoreResult<bool> {
    let parsed = PasswordHash::new(phc_hash)
        .map_err(|e| CoreError::Internal(format!("stored hash is invalid: {e}")))?;
    Ok(Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrip() {
        let hash = hash_password("correct horse battery").unwrap();
        assert!(verify_password("correct horse battery", &hash).unwrap());
        assert!(!verify_password("wrong password here", &hash).unwrap());
    }

    #[test]
    fn short_passwords_rejected() {
        assert!(hash_password("short").is_err());
    }
}
