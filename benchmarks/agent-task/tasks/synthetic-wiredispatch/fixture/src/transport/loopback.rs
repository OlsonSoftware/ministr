//! In-memory client: encodes a frame, routes it, returns the reply.
use crate::codec::{decode_frame, encode_frame, Frame, Reply};
use crate::dispatch::route;

#[derive(Default)]
pub struct LoopbackClient;

impl LoopbackClient {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Send an operation over the loopback and return its reply.
    pub fn call(&self, op: u16, payload: &[u8]) -> Option<Reply> {
        let wire = encode_frame(&Frame { op, payload: payload.to_vec() });
        let frame = decode_frame(&wire)?;
        route(frame.op, &frame.payload)
    }
}
