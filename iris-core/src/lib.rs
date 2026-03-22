//! iris-core — domain logic for the iris context cache controller.
//!
//! This crate contains the core types, error definitions, and service traits
//! for iris. It has no transport dependencies and no knowledge of MCP.

pub mod analytics;
pub mod coherence;
pub mod config;
pub mod embedding;
pub mod error;
pub mod extraction;
pub mod git;
pub mod index;
pub mod ingestion;
pub mod llms_txt;
pub mod parser;
pub mod search;
pub mod service;
pub mod session;
pub mod storage;
pub mod token;
pub mod tracing;
pub mod types;
pub mod web;
