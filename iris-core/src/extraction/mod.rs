//! Content extraction: claim extraction, relationship detection, and summary generation.
//!
//! This module provides heuristic-based claim extraction, cross-reference
//! relationship detection, and TF-IDF extractive summarization. All operate
//! on plain text without requiring ML models, making them suitable for fast,
//! first-pass ingestion of document corpora.

pub mod abstractive;
pub mod claims;
pub mod relationships;
pub mod strategy;
pub mod summary;

pub use abstractive::{AbstractiveCompressor, CompressError};
pub use claims::{ClaimExtractor, HeuristicClaimExtractor};
pub use relationships::{HeuristicRelationshipDetector, RelationshipDetector};
pub use strategy::{
    AutoCompressor, CompressStrategy, ContentType, ExtractiveStrategy, SalienceWeightedStrategy,
};
pub use summary::{ExtractiveSummaryGenerator, SummaryGenerator};
