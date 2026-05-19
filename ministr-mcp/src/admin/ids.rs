//! ID generation for admin records.
//!
//! Kept here (rather than reusing `auth::util::generate_id`) so the admin
//! module doesn't depend on auth internals — and so the prefix lets you
//! tell a job id from an OAuth id at a glance in logs.

use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn new_job_id() -> String {
    let mut hasher = Sha256::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(nanos.to_le_bytes());
    let entropy: u64 = std::ptr::from_ref(&hasher) as u64;
    hasher.update(entropy.to_le_bytes());
    let hash = hasher.finalize();
    format!("job_{}", &hex_short(&hash[..8]))
}

fn hex_short(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(hex_nibble((b >> 4) & 0x0f));
        s.push(hex_nibble(b & 0x0f));
    }
    s
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_have_job_prefix() {
        let id = new_job_id();
        assert!(id.starts_with("job_"));
        assert_eq!(id.len(), 4 + 16);
    }
}
