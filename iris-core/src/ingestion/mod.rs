//! Document ingestion pipeline.
//!
//! This module orchestrates the full ingestion flow: file discovery, parsing,
//! section enrichment, claim extraction, embedding, symbol extraction, and
//! storage. The pipeline supports incremental re-indexing via content hashing
//! and mtime checks.
//!
//! # Module structure
//!
//! - [`discovery`] — file discovery, ignore patterns, glob resolution
//! - [`sections`] — section splitting, coalescing, enrichment
//! - [`roots`] — path helpers, language stats, hashing, root management
//! - [`embedding`] — dense/sparse embedding, batch insert, vector deletion
//! - [`symbols`] — code symbol extraction, reference resolution, bridge linking
//! - [`process`] — shared per-document processing core
//! - [`pipeline`] — `IngestionPipeline` orchestrator and public entry points

mod discovery;
mod embedding;
mod pipeline;
mod process;
mod roots;
mod sections;
mod symbols;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

// Primary public API
pub use pipeline::{
    ContentIngestionStats, IngestionPhase, IngestionPipeline, IngestionProgress, IngestionStats,
};

// Re-export utility functions used by other crates
pub use discovery::discover_paths;
pub use roots::{compute_root_id, namespace_path, strip_root_prefix};
pub use sections::coalesce_small_sections;
