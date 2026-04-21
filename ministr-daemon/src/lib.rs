//! ministr daemon — HTTP API over Unix domain socket.
//!
//! Provides the axum-based daemon server, corpus registry, background
//! indexer, and type conversions. Used by `ministr-app` (Tauri GUI) and
//! testable without any GUI dependencies.

pub mod activity;
pub mod ask;
pub mod cloud;
pub mod convert;
pub mod daemon;
pub mod indexer;
pub mod inference;
pub mod persistence;
pub mod registry;
pub mod state;
