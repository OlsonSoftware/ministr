//! Transports. Only the in-memory loopback ships in this crate.
mod loopback;
pub use loopback::LoopbackClient;
