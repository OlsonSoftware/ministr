//! mirror operations.
use crate::codec::Reply;

/// reflects the caller's state block verbatim.
pub fn reflect_state(payload: &[u8]) -> Reply {
    Reply::new(5, payload.to_vec())
}
