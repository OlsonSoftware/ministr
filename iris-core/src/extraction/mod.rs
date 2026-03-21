//! Content extraction: claim extraction and summary generation.
//!
//! This module provides heuristic-based claim extraction and TF-IDF extractive
//! summarization. Both operate on plain text without requiring ML models,
//! making them suitable for fast, first-pass ingestion of document corpora.

pub mod claims;
pub mod summary;

pub use claims::{ClaimExtractor, HeuristicClaimExtractor};
pub use summary::{ExtractiveSummaryGenerator, SummaryGenerator};
