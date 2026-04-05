//! iris-mcp — MCP server adapter for iris.
//!
//! This crate adapts the service traits from `iris-core` to the MCP protocol
//! using the `rmcp` crate. It handles JSON-RPC routing, tool registration,
//! and request/response mapping.

#![deny(unsafe_code)]
#![allow(clippy::cast_precision_loss)] // intentional for progress/stats ratios

pub mod a2a;
pub mod auth;
pub mod bundle_routes;
pub mod elicitation;
pub mod error;
pub mod proxy;
pub mod sampling;
pub mod server;
pub mod task;
