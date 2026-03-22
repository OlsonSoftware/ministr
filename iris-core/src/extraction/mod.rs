//! Content extraction: claim extraction, relationship detection, and summary generation.
//!
//! This module provides heuristic-based claim extraction, cross-reference
//! relationship detection, and TF-IDF extractive summarization. All operate
//! on plain text without requiring ML models, making them suitable for fast,
//! first-pass ingestion of document corpora.

pub mod claims;
pub mod relationships;
pub mod summary;

pub use claims::{ClaimExtractor, HeuristicClaimExtractor};
pub use relationships::{HeuristicRelationshipDetector, RelationshipDetector};
pub use summary::{ExtractiveSummaryGenerator, SummaryGenerator};
