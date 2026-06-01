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
//! - [`parse_pool`] — dedicated rayon CPU pool for off-runtime tree-sitter parsing
//! - [`pipeline`] — `IngestionPipeline` orchestrator and public entry points

mod discovery;
mod embed_stage;
mod embedding;
mod occurrences;
mod parse_pool;
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
    BatchIngestionConfig, ContentIngestionStats, IngestionPhase, IngestionPipeline,
    IngestionProgress, IngestionStats,
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
/// - **4**: Bridge framework detection now also scans manifests in
///   *subdirectories* of the corpus (`FrameworkDetector::detect_in_files`),
///   not just the upward walk from the root. Monorepos with a subdir app
///   (e.g. a Tauri app under `<repo>/app/src-tauri/`) now detect their
///   bridge framework and link cross-language endpoints on re-extraction.
/// - **5**: The JS/TS/TSX ref extractor (`code::refs::extract_refs_js_ts`)
///   now emits `Calls`/`Implements`/`Uses` edges (class `extends`/`implements`,
///   call sites, `new`, and type annotations), not import-only. TS/JS corpora
///   pick up a real reference graph (so `ministr_references`/`ministr_solid`
///   stop starving) on re-extraction.
/// - **6**: The Java ref extractor (`code::refs::extract_refs_java`) likewise
///   emits `Calls`/`Implements`/`Uses` edges (class `extends`/`implements`,
///   interface `extends`, `method_invocation`, `new`, and declared type
///   positions), not import-only. Java corpora gain a real reference graph
///   on re-extraction.
/// - **7**: The C# ref extractor (`code::refs::extract_refs_csharp`) likewise
///   emits `Calls`/`Implements`/`Uses` edges (`base_list` heritage,
///   `invocation_expression`, `new`, and declared type/return positions),
///   not import-only. C# corpora gain a real reference graph on re-extraction.
/// - **8**: The Kotlin ref extractor (`code::refs::extract_refs_kotlin`)
///   likewise emits `Calls`/`Implements`/`Uses` edges (`delegation_specifier`
///   supertypes, `call_expression`, and `user_type` positions), not
///   import-only. Kotlin corpora gain a real reference graph on re-extraction.
/// - **9**: The Swift ref extractor (`code::refs::extract_refs_swift`)
///   likewise emits `Calls`/`Implements`/`Uses` edges (`inheritance_specifier`
///   supertypes, `call_expression`, and `user_type` positions), not
///   import-only. Swift corpora gain a real reference graph on re-extraction.
/// - **10**: The Python ref extractor (`code::refs::extract_refs_python`)
///   likewise emits `Calls`/`Implements`/`Uses` edges (class base classes as
///   `Implements`, `call` sites, and `type` annotations), not import-only.
///   Python corpora gain a real reference graph on re-extraction.
/// - **11**: The PHP ref extractor (`code::refs::extract_refs_php`) likewise
///   emits `Calls`/`Implements`/`Uses` edges (`extends`/`implements` clauses,
///   call/method/static-call sites, `new`, and `named_type` hints), not
///   import-only. PHP corpora gain a real reference graph on re-extraction.
/// - **12**: The Ruby ref extractor (`code::refs::extract_refs_ruby`) now
///   emits `Calls`/`Implements`/`Uses` edges (superclass + `include`/`prepend`/
///   `extend` mixins as `Implements`, method calls as `Calls`, `Constant.new`
///   as `Uses`), not import-only. Ruby corpora gain a real reference graph on
///   re-extraction.
/// - **13**: The Scala ref extractor (`code::refs::extract_refs_scala`) now
///   emits `Calls`/`Implements`/`Uses` edges (`extends_clause` base + `with`
///   mixin traits as `Implements`, `call_expression` callees as `Calls`,
///   `new` `instance_expression` + declared `type_identifier` positions as
///   `Uses`), not import-only. Scala corpora gain a real reference graph on
///   re-extraction.
/// - **14**: The Go ref extractor (`code::refs::extract_refs_go`) now emits
///   `Calls`/`Uses` edges (`call_expression` callees — bare or
///   `selector_expression` method — as `Calls`; `type_identifier` in declared
///   type positions + `composite_literal` types as `Uses`), not import-only.
///   Go is intentionally `Calls`+`Uses` only — interface conformance is
///   structural/implicit, so there is no `Implements` signal in the AST. Go
///   corpora gain a reference graph on re-extraction. This completes the
///   per-language edge-graph rollout (all 10 formerly import-only languages
///   now emit a real graph).
pub const EXTRACTOR_VERSION: i64 = 14;

/// Version of the symbol-reference *resolution* pipeline.
///
/// Stored per file in `file_hashes.resolver_version` and compared on
/// re-ingest / daemon startup: when the stored version is less than this
/// constant, the file's `symbol_refs` rows are rebuilt by re-running the
/// resolver against the existing stored symbols — without re-parsing
/// from scratch and without touching embeddings. The index auto-heals
/// after a resolver-logic change, no manual corpus wipe needed.
///
/// This is the resolver-side counterpart to [`EXTRACTOR_VERSION`]:
/// `EXTRACTOR_VERSION` invalidates symbol + raw-ref *extraction*;
/// `RESOLVER_VERSION` invalidates the *name-binding* step that turns raw
/// refs into stored `SymbolRefRecord`s.
///
/// **Bump this when**: `ministr_core::ingestion::symbols::resolve_and_store_refs`,
/// `disambiguate_target`, or the `PRIMITIVE_TYPES` denylist in
/// `ministr_core::code::refs` change in a way that would produce
/// different `SymbolRefRecord`s for the same set of stored symbols and
/// the same `RawRef`s.
///
/// Version history:
/// - **0**: Pre-versioning baseline. All rows predating migration V22
///   compare as `0` and trigger a re-resolution on first daemon startup
///   after upgrade.
/// - **1**: Resolver scoping fixes — file-anchor `from_context`
///   replaced with line-range enclosing-symbol lookup; same-crate
///   preferred over cross-file in `disambiguate_target`; expanded
///   `PRIMITIVE_TYPES` denylist covers the Rust prelude + common stdlib
///   names (`Command`, `Result`, `Vec`, `HashMap`, `Option`, ...) so
///   stdlib references no longer phantom-bind to same-named user types.
/// - **2**: Same-language constraint on ref resolution. Targets whose
///   file extension resolves to a different `GrammarRegistry` language
///   than the source's are dropped before disambiguation. This
///   eliminates cross-language phantom bindings like a Rust
///   `bundle::header` use site getting resolved to a TSX `<Header>`
///   component. Intentional cross-language paths still live in
///   `RefKind::Bridge` (handled by the bridge linker), which never
///   goes through `disambiguate_target`.
/// - **3**: `filter_primary` (and the `file_anchor` lookup) now allow
///   `"module"`-kind symbols as resolution targets / anchors. Both
///   previously tested for `"mod"`, but `ItemKind::Module.as_str()` is
///   `"module"` — so every module-kind target was silently dropped. This
///   fixes mixin-as-interface resolution (Ruby `include M`/`prepend M`
///   onto a `module`, Scala packages) so those `Implements`/`Uses` refs
///   bind to their definition instead of going permanently pending.
pub const RESOLVER_VERSION: i64 = 3;
