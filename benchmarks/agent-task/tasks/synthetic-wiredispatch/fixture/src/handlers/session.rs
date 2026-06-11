//! session operations.
use crate::codec::Reply;

/// establishes a link; echoes the peer nonce incremented.
pub fn open_link(payload: &[u8]) -> Reply {
    Reply::new(1, vec![payload.first().copied().unwrap_or(0).wrapping_add(1)])
}
