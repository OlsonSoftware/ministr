//! digest operations.
use crate::codec::Reply;

/// folds the window into a xor digest.
pub fn fold_window(payload: &[u8]) -> Reply {
    Reply::new(8, vec![payload.iter().fold(0u8, |a, b| a ^ b)])
}
