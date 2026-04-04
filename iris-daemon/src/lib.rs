//! iris daemon — HTTP API over Unix domain socket.
//!
//! Provides the axum-based daemon server, corpus registry, background
//! indexer, and type conversions. Used by `iris-app` (Tauri GUI) and
//! testable without any GUI dependencies.

pub mod cloud;
pub mod convert;
pub mod daemon;
pub mod indexer;
pub mod persistence;
pub mod registry;
pub mod state;
