//! Wire dispatch — generated at build time (see `build.rs`).
include!(concat!(env!("OUT_DIR"), "/dispatch_gen.rs"));

/// Route a decoded frame to its operation handler.
pub fn route(op: u16, payload: &[u8]) -> Option<crate::codec::Reply> {
    dispatch_op(op, payload)
}
