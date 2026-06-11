//! quota operations.
use crate::codec::Reply;

/// checks remaining quota; returns limit-minus-used.
pub fn check_balance(payload: &[u8]) -> Reply {
    Reply::new(7, vec![64u8.saturating_sub(payload.first().copied().unwrap_or(0))])
}
