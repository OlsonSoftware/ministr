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
pub use discovery::{compute_corpus_stat_merkle, discover_paths, is_unreal_corpus};
pub use roots::{compute_root_id, namespace_path, strip_root_prefix};
pub use sections::coalesce_small_sections;

// Re-export for intra-crate use by the coherence engine so file-remove
// events can tear down vectors alongside the SQL cascade delete.
pub(crate) use embedding::delete_document_vectors;

/// Version of the symbol-reference + symbol-table extraction pipeline.
///
/// Stored per file in `file_hashes.extractor_version` and compared on
/// re-ingest: when the stored version is less than this constant, the
/// file is re-parsed even if its content hash hasn't changed. This way
/// the index auto-heals after an extractor change — no manual corpus
/// wipe needed.
///
/// **Bump this when**: `ministr_core::code::refs` or
/// `ministr_core::ingestion::symbols` changes in a way that would
/// produce different `RawRef`s / `SymbolRecord`s for the same source.
/// Purely cosmetic changes (comments, rename of a private helper) don't
/// need a bump.
///
/// Version history:
/// - **1**: Pre-versioning baseline. All rows predating migration V19
///   are treated as version 0 and forced through a full re-extraction
///   on first run after upgrade.
/// - **2**: `extract_call_ref` now emits a `Uses(Parent)` ref alongside
///   `Calls(method)` for every `Parent::method(...)` scoped call, so
///   `ministr_references` on the parent type picks up method call sites.
/// - **3**: C++ grammar swapped from `tree-sitter-cpp` to
///   `tree-sitter-unreal-cpp` (a strict superset). Vanilla C++ ASTs
///   should be byte-identical, but UE reflection macros (`UCLASS` /
///   `UFUNCTION` / `GENERATED_BODY` / ...) now parse as first-class
///   nodes instead of ERROR subtrees — UE corpora will pick up
///   meaningfully more symbols on re-extraction.
pub const EXTRACTOR_VERSION: i64 = 3;
