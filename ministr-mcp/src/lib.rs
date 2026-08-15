//! ministr-mcp — MCP server adapter for ministr.
//!
//! This crate adapts the service traits from `ministr-core` to the MCP protocol
//! using the `rmcp` crate. It handles JSON-RPC routing, tool registration,
//! and request/response mapping.

#![deny(unsafe_code)]
#![allow(clippy::cast_precision_loss)] // intentional for progress/stats ratios

pub mod a2a;
pub mod admin;
pub mod auth;
// arch-backend-seam-promotion (2026-08-15): the backend module was
// promoted to the shared `ministr-backend` crate so the CLI (and any
// future non-MCP surface) consumes the same seam. The re-export keeps
// every pre-existing `ministr_mcp::backend::*` path compiling.
pub use ministr_backend as backend;
pub mod bundle_routes;
pub mod error;
pub mod pg_tls;
pub mod run_digest;
pub mod sampling;
pub mod server;
pub mod sessions;
pub mod task;
pub mod tenant_scope;
mod time;
