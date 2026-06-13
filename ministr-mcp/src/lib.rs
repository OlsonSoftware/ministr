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
pub mod backend;
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
