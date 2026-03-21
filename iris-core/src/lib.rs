//! iris-core — domain logic for the iris context cache controller.
//!
//! This crate contains the core types, error definitions, and service traits
//! for iris. It has no transport dependencies and no knowledge of MCP.

pub mod config;
pub mod error;
pub mod extraction;
pub mod parser;
pub mod storage;
pub mod token;
pub mod tracing;
pub mod types;
