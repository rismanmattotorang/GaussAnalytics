//! API keys: high-entropy service credentials.
//!
//! Unlike passwords (salted Argon2, verified by re-hashing), API keys must be
//! *looked up* by hash, so we use a fast deterministic digest (SHA-256). The
//! key itself is high-entropy (two v4 UUIDs), so an unsalted digest is safe —
//! there is nothing to brute-force. Only the digest is ever persisted.

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Generate a fresh, prefixed API key (returned to the caller exactly once).
pub fn generate_api_key() -> String {
    let a = Uuid::new_v4().simple().to_string();
    let b = Uuid::new_v4().simple().to_string();
    format!("gauss_{a}{b}")
}

/// Compute the storable SHA-256 hex digest of an API key.
pub fn hash_api_key(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_are_prefixed_and_unique() {
        let a = generate_api_key();
        let b = generate_api_key();
        assert!(a.starts_with("gauss_"));
        assert_ne!(a, b);
    }

    #[test]
    fn hash_is_stable_64_hex() {
        let k = generate_api_key();
        let h = hash_api_key(&k);
        assert_eq!(h.len(), 64);
        assert_eq!(h, hash_api_key(&k));
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
