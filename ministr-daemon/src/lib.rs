//! ministr daemon — HTTP API over the platform-native IPC transport
//! (Unix domain sockets on macOS/Linux, named pipes on Windows).
//!
//! Provides the axum-based daemon server, corpus registry, background
//! indexer, and type conversions. Used by `ministr-app` (Tauri GUI) and
//! testable without any GUI dependencies.

pub mod activity;
pub mod ask;
pub mod bootstrap;
pub mod cloud;
pub mod convert;
pub mod daemon;
pub mod indexer;
pub mod inference;
pub mod persistence;
pub mod registry;
pub mod state;
pub mod transport;
