//! Signed embedding tokens.
//!
//! For embedded analytics, GaussAnalytics issues a stateless, tamper-evident
//! token that names a resource and an expiry, signed with HMAC-SHA256 under a
//! server secret. Anyone holding the token can render the named resource until
//! it expires; no session is required. The token carries no secrets — only the
//! resource id and expiry, plus a signature — and is verified in constant time.
//!
//! Token format: `{resource_hex}.{exp_unix}.{sig_hex}` where the signature
//! covers `{resource_hex}.{exp_unix}`.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use gauss_core::error::{CoreError, CoreResult};

type HmacSha256 = Hmac<Sha256>;

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

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

fn sign(secret: &[u8], msg: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(msg.as_bytes());
    to_hex(&mac.finalize().into_bytes())
}

/// Issue a signed embed token for `resource`, expiring at `exp_unix`.
pub fn sign_embed(secret: &str, resource: &str, exp_unix: i64) -> CoreResult<String> {
    if secret.is_empty() {
        return Err(CoreError::Config(
            "embedding secret is not configured".into(),
        ));
    }
    let rhex = to_hex(resource.as_bytes());
    let msg = format!("{rhex}.{exp_unix}");
    let sig = sign(secret.as_bytes(), &msg);
    Ok(format!("{msg}.{sig}"))
}

/// Verify a token and return its resource, or fail if malformed/forged/expired.
pub fn verify_embed(secret: &str, token: &str, now_unix: i64) -> CoreResult<String> {
    if secret.is_empty() {
        return Err(CoreError::Unauthorized("embedding is not enabled".into()));
    }
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(CoreError::Unauthorized("malformed embed token".into()));
    }
    let (rhex, exp_s, sig) = (parts[0], parts[1], parts[2]);
    let expected = sign(secret.as_bytes(), &format!("{rhex}.{exp_s}"));
    if !ct_eq(&expected, sig) {
        return Err(CoreError::Unauthorized("invalid embed signature".into()));
    }
    let exp: i64 = exp_s
        .parse()
        .map_err(|_| CoreError::Unauthorized("invalid embed expiry".into()))?;
    if now_unix > exp {
        return Err(CoreError::Unauthorized("embed token expired".into()));
    }
    let bytes =
        from_hex(rhex).ok_or_else(|| CoreError::Unauthorized("invalid embed resource".into()))?;
    String::from_utf8(bytes).map_err(|_| CoreError::Unauthorized("invalid embed resource".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-embedding-secret";

    #[test]
    fn round_trip_valid_token() {
        let token = sign_embed(SECRET, "dashboard:42", 10_000).unwrap();
        assert_eq!(verify_embed(SECRET, &token, 9_999).unwrap(), "dashboard:42");
    }

    #[test]
    fn expired_token_rejected() {
        let token = sign_embed(SECRET, "card:7", 100).unwrap();
        assert!(verify_embed(SECRET, &token, 101).is_err());
    }

    #[test]
    fn tampered_signature_rejected() {
        let token = sign_embed(SECRET, "card:7", 10_000).unwrap();
        let mut bad = token.clone();
        bad.pop();
        bad.push(if token.ends_with('0') { '1' } else { '0' });
        assert!(verify_embed(SECRET, &bad, 1).is_err());
    }

    #[test]
    fn wrong_secret_rejected() {
        let token = sign_embed(SECRET, "card:7", 10_000).unwrap();
        assert!(verify_embed("other-secret", &token, 1).is_err());
    }

    #[test]
    fn disabled_when_secret_empty() {
        assert!(sign_embed("", "x", 1).is_err());
        assert!(verify_embed("", "a.b.c", 1).is_err());
    }
}
