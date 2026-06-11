//! beacon operations.
use crate::codec::Reply;

/// emits a liveness pulse; returns a fixed heartbeat marker.
pub fn emit_pulse(payload: &[u8]) -> Reply {
    Reply::new(3, vec![0xAA, payload.first().copied().unwrap_or(0)])
}
