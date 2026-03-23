//! iris-mcp — MCP server adapter for iris.
//!
//! This crate adapts the service traits from `iris-core` to the MCP protocol
//! using the `rmcp` crate. It handles JSON-RPC routing, tool registration,
//! and request/response mapping.

pub mod elicitation;
pub mod error;
pub mod sampling;
pub mod server;
pub mod task;
