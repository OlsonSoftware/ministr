//! journal operations.
use crate::codec::Reply;

/// appends a record; returns the new record count (stub: len).
pub fn append_record(payload: &[u8]) -> Reply {
    Reply::new(6, vec![payload.len() as u8, 0x4A])
}
