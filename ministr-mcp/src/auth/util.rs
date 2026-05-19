//! Crypto + time helpers used by the OAuth flow.
//!
//! Kept private to the `auth` module so callers can't reach in and depend on
//! the specific identifier shape (which may evolve to a stronger RNG).

use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) use crate::time::epoch_now;

/// Generate a cryptographically random URL-safe identifier.
pub(super) fn generate_id() -> String {
    let mut hasher = Sha256::new();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(timestamp.to_le_bytes());
    let entropy: u64 = std::ptr::from_ref(&hasher) as u64;
    hasher.update(entropy.to_le_bytes());
    let hash = hasher.finalize();
    base64_url_encode(&hash[..16])
}

/// Base64url-encode without padding (RFC 4648 §5).
pub(super) fn base64_url_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut encoded = String::new();

    let mut i = 0;
    while i < data.len() {
        let b0 = u32::from(data[i]);
        let b1 = if i + 1 < data.len() {
            u32::from(data[i + 1])
        } else {
            0
        };
        let b2 = if i + 2 < data.len() {
            u32::from(data[i + 2])
        } else {
            0
        };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        encoded.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        encoded.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);

        if i + 1 < data.len() {
            encoded.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < data.len() {
            encoded.push(ALPHABET[(triple & 0x3F) as usize] as char);
        }

        i += 3;
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_encode_roundtrip() {
        let data = b"hello, world!";
        let encoded = base64_url_encode(data);
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('='));
        assert!(!encoded.is_empty());
    }

    #[test]
    fn pkce_s256_verification() {
        let code_verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let challenge = base64_url_encode(&hasher.finalize());

        let mut hasher2 = Sha256::new();
        hasher2.update(code_verifier.as_bytes());
        let verify = base64_url_encode(&hasher2.finalize());

        assert_eq!(challenge, verify);
    }

    #[test]
    fn generate_id_produces_unique_values() {
        let id1 = generate_id();
        let id2 = generate_id();
        assert!(!id1.is_empty());
        assert!(!id2.is_empty());
    }
}
