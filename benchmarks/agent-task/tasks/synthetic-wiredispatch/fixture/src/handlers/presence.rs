//! presence operations.
use crate::codec::Reply;

/// announces availability; returns the roster size byte.
pub fn announce(payload: &[u8]) -> Reply {
    Reply::new(2, vec![payload.len() as u8])
}
