//! ledger operations.
use crate::codec::Reply;

/// posts a balance entry; returns the running parity bit.
pub fn post_entry(payload: &[u8]) -> Reply {
    Reply::new(4, vec![payload.iter().fold(0u8, |a, b| a ^ b) & 1])
}
