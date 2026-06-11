//! ministr-core — domain logic for the ministr code intelligence server.
//!
//! This crate contains the core types, error definitions, and service traits
//! for ministr. It has no transport dependencies and no knowledge of MCP.

#![deny(unsafe_code)]
// Pedantic lints that are acceptable in context:
// - cast_precision_loss: intentional f64 casts for progress/stats (counts never exceed 2^52)
// - missing_errors_doc: tracked separately, not blocking
// - doc_markdown: snake_case identifiers in docs don't need backticks everywhere
// - struct_excessive_bools: config/option structs legitimately use multiple bools
#![allow(
    clippy::cast_precision_loss,
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::struct_excessive_bools
)]

pub mod analytics;
pub mod bundle;
pub mod code;
pub mod coherence;
pub mod config;
pub mod corpus_id;
pub mod embedding;
pub mod error;
pub mod extraction;
pub mod freshness;
pub mod fs_util;
pub mod git;
pub mod index;
pub mod ingestion;
pub mod init;
pub mod llms_txt;
pub mod mem_profile;
pub mod parser;
pub mod scaffold;
pub mod search;
pub mod service;
pub mod session;
pub mod storage;
pub mod token;
pub mod tracing;
pub mod types;
pub mod web;
pub mod workspace;
