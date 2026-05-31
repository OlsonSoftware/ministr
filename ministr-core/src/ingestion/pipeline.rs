//! `IngestionPipeline` — the main orchestrator for document ingestion.
//!
//! All four public entry points delegate their core processing to
//! [`process::store_enriched_document`], keeping each method focused on its
//! specific I/O and embedding strategy.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::stream::{self, StreamExt};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, trace, warn};

use crate::code::AstParser;
use crate::code::bridge::linker::BridgeLinker;
use crate::code::bridge::{BridgeKind, create_linker_for_kinds, detector};
use crate::code::package_graph::PackageGraph;
use crate::embedding::Embedder;
use crate::error::IngestionError;
use crate::extraction::claims::HeuristicClaimExtractor;
use crate::extraction::relationships::HeuristicRelationshipDetector;
use crate::extraction::summary::ExtractiveSummaryGenerator;
use crate::index::{NullVectorIndex, VectorIndex};
use crate::mem_profile;
use crate::parser::{
    DocumentParser, MarkdownParser, ParserKind, create_parser, detect_parser_kind,
};
use crate::storage::traits::{FileHashRecord, Storage, SymbolFilter};
use crate::types::{CorpusRoot, RootKind, VectorId};

use super::discovery::{discover_files, discover_paths, is_in_ignored_dir};
use super::embedding::{
    EMBED_FLUSH_THRESHOLD, batch_embed_and_insert, collect_document_embeddings, embed_document,
};
use super::process::{ProcessOptions, store_enriched_document};
use super::roots::{
    accumulate_language_stats, all_files_unchanged_by_mtime, compute_content_hash,
    compute_relative_path, compute_root_id, file_mtime_nanos, find_root_entry_for_file,
    namespace_path, strip_root_prefix, update_root_stats,
};
use super::symbols::{
    PendingRef, extract_code_symbols, persist_pending_refs, rebuild_bridge_endpoints,
    repair_missing_refs, resolve_and_store_refs, resolve_pending_refs, store_bridge_links,
};

// ── Stats types ──────────────────────────────────────────────────────────────

/// Result of ingesting a corpus directory.
///
/// # Examples
///
/// ```
/// use ministr_core::ingestion::IngestionStats;
///
/// let stats = IngestionStats {
///     files_discovered: 10,
///     files_skipped: 5,
///     files_indexed: 4,
///     files_removed: 0,
///     files_failed: 1,
///     total_sections: 20,
///     total_claims: 45,
///     total_embeddings: 65,
///     failed_files: vec![],
/// };
///
/// assert_eq!(stats.files_indexed + stats.files_skipped + stats.files_failed, 10);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestionStats {
    pub files_discovered: usize,
    pub files_skipped: usize,
    pub files_indexed: usize,
    pub files_removed: usize,
    pub files_failed: usize,
    pub total_sections: usize,
    pub total_claims: usize,
    pub total_embeddings: usize,
    /// Per-file failure records: `(relative_path, error_message)`. Populated
    /// whenever the producer logs a per-file ingest failure so callers can
    /// surface which files broke without scraping logs.
    pub failed_files: Vec<(String, String)>,
}

impl IngestionStats {
    /// Create a new `IngestionStats` with the given discovered file count.
    #[must_use]
    fn new(files_discovered: usize) -> Self {
        Self {
            files_discovered,
            files_skipped: 0,
            files_indexed: 0,
            files_removed: 0,
            files_failed: 0,
            total_sections: 0,
            total_claims: 0,
            total_embeddings: 0,
            failed_files: Vec::new(),
        }
    }
}

/// Result of ingesting raw content via [`IngestionPipeline::ingest_content`].
///
/// # Examples
///
/// ```
/// use ministr_core::ingestion::ContentIngestionStats;
///
/// let stats = ContentIngestionStats {
///     sections: 5,
///     claims: 12,
///     skipped: false,
/// };
/// assert!(!stats.skipped);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentIngestionStats {
    pub sections: usize,
    pub claims: usize,
    pub skipped: bool,
}

// ── Progress tracker ─────────────────────────────────────────────────────────

/// Ingestion phase for granular progress tracking.
///
/// These correspond to the major stages of the ingestion pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IngestionPhase {
    /// Waiting to start.
    Idle = 0,
    /// Walking directories to discover files.
    Discovering = 1,
    /// Parsing files, extracting sections and symbols.
    Parsing = 2,
    /// Generating embeddings for sections.
    Embedding = 3,
    /// Resolving cross-references and cleaning up stale data.
    Finalizing = 4,
}

impl IngestionPhase {
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Discovering,
            2 => Self::Parsing,
            3 => Self::Embedding,
            4 => Self::Finalizing,
            _ => Self::Idle,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Discovering => "discovering",
            Self::Parsing => "parsing",
            Self::Embedding => "embedding",
            Self::Finalizing => "finalizing",
        }
    }
}

/// Shared progress tracker for background ingestion.
///
/// Uses atomics so it can be read from tool handlers while the ingestion
/// task updates it concurrently. Status values: 0=pending, 1=running, 2=complete.
///
/// # Examples
///
/// ```
/// use ministr_core::ingestion::{IngestionProgress, IngestionPhase};
/// use std::sync::Arc;
///
/// let progress = Arc::new(IngestionProgress::new());
/// progress.start(42);
/// assert_eq!(progress.files_total(), 42);
/// assert_eq!(progress.files_done(), 0);
/// assert_eq!(progress.embeddings_total(), 0);
/// assert!(progress.is_running());
///
/// progress.set_phase(IngestionPhase::Parsing);
/// assert_eq!(progress.phase(), IngestionPhase::Parsing);
///
/// progress.set_current_file("src/main.rs");
/// assert_eq!(progress.current_file(), "src/main.rs");
///
/// progress.increment_done();
/// assert_eq!(progress.files_done(), 1);
///
/// progress.add_sections_done(5);
/// assert_eq!(progress.sections_done(), 5);
///
/// progress.add_embeddings_total(10);
/// progress.add_embeddings_done(5);
/// assert_eq!(progress.embeddings_total(), 10);
/// assert_eq!(progress.embeddings_done(), 5);
///
/// progress.complete();
/// assert!(!progress.is_running());
/// ```
pub struct IngestionProgress {
    status: std::sync::atomic::AtomicU8,
    phase: std::sync::atomic::AtomicU8,
    files_total: AtomicUsize,
    files_done: AtomicUsize,
    sections_done: AtomicUsize,
    embeddings_total: AtomicUsize,
    embeddings_done: AtomicUsize,
    current_file: parking_lot::Mutex<String>,
}

impl IngestionProgress {
    #[must_use]
    pub fn new() -> Self {
        Self {
            status: std::sync::atomic::AtomicU8::new(0),
            phase: std::sync::atomic::AtomicU8::new(0),
            files_total: AtomicUsize::new(0),
            files_done: AtomicUsize::new(0),
            sections_done: AtomicUsize::new(0),
            embeddings_total: AtomicUsize::new(0),
            embeddings_done: AtomicUsize::new(0),
            current_file: parking_lot::Mutex::new(String::new()),
        }
    }

    pub fn start(&self, total_files: usize) {
        self.files_total.store(total_files, Ordering::Relaxed);
        self.files_done.store(0, Ordering::Relaxed);
        self.sections_done.store(0, Ordering::Relaxed);
        self.embeddings_total.store(0, Ordering::Relaxed);
        self.embeddings_done.store(0, Ordering::Relaxed);
        self.set_phase(IngestionPhase::Parsing);
        self.status.store(1, Ordering::Relaxed);
    }

    pub fn set_phase(&self, phase: IngestionPhase) {
        self.phase.store(phase as u8, Ordering::Relaxed);
    }

    pub fn set_current_file(&self, file: &str) {
        let mut guard = self.current_file.lock();
        guard.clear();
        guard.push_str(file);
    }

    pub fn increment_done(&self) {
        self.files_done.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_sections_done(&self, count: usize) {
        self.sections_done.fetch_add(count, Ordering::Relaxed);
    }

    pub fn add_embeddings_total(&self, count: usize) {
        self.embeddings_total.fetch_add(count, Ordering::Relaxed);
    }

    pub fn add_embeddings_done(&self, count: usize) {
        self.embeddings_done.fetch_add(count, Ordering::Relaxed);
    }

    pub fn complete(&self) {
        self.set_phase(IngestionPhase::Idle);
        self.set_current_file("");
        self.status.store(2, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.status.load(Ordering::Relaxed) == 1
    }

    #[must_use]
    pub fn phase(&self) -> IngestionPhase {
        IngestionPhase::from_u8(self.phase.load(Ordering::Relaxed))
    }

    #[must_use]
    pub fn current_file(&self) -> String {
        self.current_file.lock().clone()
    }

    #[must_use]
    pub fn files_total(&self) -> usize {
        self.files_total.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn files_done(&self) -> usize {
        self.files_done.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn sections_done(&self) -> usize {
        self.sections_done.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn embeddings_total(&self) -> usize {
        self.embeddings_total.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn embeddings_done(&self) -> usize {
        self.embeddings_done.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn status(&self) -> u8 {
        self.status.load(Ordering::Relaxed)
    }
}

impl Default for IngestionProgress {
    fn default() -> Self {
        Self::new()
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Default concurrency for the producer-side parse fan-out.
///
/// `buffer_unordered(concurrency)` controls how many parse-and-store
/// futures are in flight at once on the tokio runtime; the embedder
/// consumer is bounded separately by the mpsc channel capacity.
///
/// Previously hard-capped at 16, which throttled UE5-class corpora on
/// boxes with >16 cores. Now scales with `available_parallelism()` up
/// to 32 — high-core machines keep all parsers busy without letting an
/// absurdly large core count create memory pressure from too many
/// parallel parse trees.
fn default_concurrency() -> usize {
    std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(32)
}

/// Result of processing a single file.
pub(super) enum FileResult {
    Skipped,
    Indexed {
        sections: usize,
        claims: usize,
        pending_refs: Vec<PendingRef>,
        embedding_pairs: Vec<(VectorId, String)>,
    },
}

/// A file to be ingested, with its resolved relative path and optional root ID.
struct FileItem {
    path: PathBuf,
    relative: String,
    root_id: Option<String>,
    /// Absolute corpus root the file was discovered under (when known).
    /// Used by `parse_and_store_file` to scope the ignore-dir guard to
    /// components inside the root — without this, a corpus rooted under
    /// an always-ignored ancestor name (e.g. `~/.ministr/remote/<hash>/`)
    /// would have every file rejected.
    root_path: Option<PathBuf>,
}

// ── BatchIngestionConfig ─────────────────────────────────────────────────────

/// Tuning knobs for streaming ingestion (PHASE4 chunk 3 scaffolding).
///
/// The pipeline conceptually runs in four phases:
///
/// 1. **Discover** — walk the input paths and filter for ingestable files
///    ([`discover_paths`](crate::ingestion::discover_paths)).
/// 2. **Parse** — tree-sit each file, split it into sections, extract
///    claims/symbols/refs, and persist the document + section rows to
///    SQLite. Per-file; already streams concurrently via
///    `buffer_unordered`.
/// 3. **Embed** — pull batches of `(VectorId, String)` pairs off the
///    producer's mpsc channel, call the embedder, and insert vectors
///    into the HNSW index in-memory. Already batched at
///    `EMBED_FLUSH_THRESHOLD`.
/// 4. **Persist** — flush the HNSW graph to disk via
///    [`VectorIndex::persist`]. Today this happens **once**, after
///    ingestion ends; the resulting peak rss (everything held in
///    memory until the very end) is what motivates PHASE4 chunk 4.
///
/// # HNSW + persistence
///
/// - `HnswIndex::insert` works incrementally — the graph keeps growing
///   across calls; no "build mode" needs to be closed first.
/// - `HnswIndex::persist` is atomic (stage-into-tmp + fsync + rename)
///   per the CHANGELOG entry `atomic HNSW persist with crash-recovery
///   backup`. Calling it mid-build is safe: it snapshots the current
///   graph state into a tmp dir and rename-swaps, leaving the
///   in-memory graph untouched.
///
/// So the substrate for "persist every N files" already exists in the
/// trait surface and the HNSW backend. This struct just gives chunk 4
/// a knob to read; today it's plumbed through but **not consumed**.
///
/// # Defaults
///
/// [`BatchIngestionConfig::default`] preserves PHASE3-era behaviour:
/// `persist_every: None`, i.e. flush only at end-of-ingest. Chunk 4 will
/// change the default to `Some(4)` once the per-batch persist hook
/// lands and benchmarks settle.
///
/// # Example (chunk-4-shape, not wired today)
///
/// ```no_run
/// use ministr_core::ingestion::{BatchIngestionConfig, IngestionPipeline};
///
/// let pipeline = IngestionPipeline::new()
///     .with_batch_config(BatchIngestionConfig {
///         batch_size: 4,
///         persist_every: Some(4),
///     });
/// # let _ = pipeline;
/// ```
#[derive(Debug, Clone, Copy)]
pub struct BatchIngestionConfig {
    /// Files per parse/embed batch. The producer is already
    /// `buffer_unordered(concurrency)` over per-file futures, so this
    /// is read as a hint for how many files' embedding pairs to gather
    /// before flushing the consumer's HNSW insert. Defaults to 4 in
    /// the chunk-4 spec; chunk 3 leaves it advisory.
    pub batch_size: usize,
    /// When `Some(n)`, persist the HNSW index to disk after every `n`
    /// files indexed. `None` preserves PHASE3 behaviour: persist once
    /// at end-of-ingest. Chunk 4 will consume this; chunk 3 only
    /// scaffolds the surface.
    pub persist_every: Option<usize>,
}

impl Default for BatchIngestionConfig {
    fn default() -> Self {
        // Preserve PHASE3 behaviour by default: no mid-run persist.
        // Chunk 4 will flip persist_every once the consume site lands.
        Self {
            batch_size: 4,
            persist_every: None,
        }
    }
}

// ── IngestionPipeline ────────────────────────────────────────────────────────

/// Ingestion pipeline orchestrator.
///
/// # Examples
///
/// ```no_run
/// use ministr_core::ingestion::IngestionPipeline;
/// use ministr_core::storage::SqliteStorage;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let storage = SqliteStorage::open_in_memory()?;
/// let pipeline = IngestionPipeline::new();
/// let stats = pipeline.ingest_directory(std::path::Path::new("docs/"), &storage).await?;
/// println!("Indexed {} files", stats.files_indexed);
/// # Ok(())
/// # }
/// ```
pub struct IngestionPipeline {
    parser_override: Option<ParserKind>,
    min_section_tokens: usize,
    claim_extractor: HeuristicClaimExtractor,
    summary_generator: ExtractiveSummaryGenerator,
    relationship_detector: HeuristicRelationshipDetector,
    progress: Option<Arc<IngestionProgress>>,
    package_graph: Option<PackageGraph>,
    concurrency: Option<usize>,
    /// Optional dual embedder for two-stage Matryoshka retrieval.
    /// When set, the embedding consumer stores full-dim vectors alongside
    /// truncated ones during ingestion.
    dual_embedder: Option<Arc<dyn crate::embedding::DualEmbedder>>,
    /// Storage handle for full-dim vectors (used only when `dual_embedder` is set).
    full_dim_storage: Option<crate::storage::SqliteStorage>,
    /// Streaming-ingestion knobs. See [`BatchIngestionConfig`] for the
    /// four-phase model + HNSW persistence notes; consumed by
    /// `run_producer_consumer`'s per-batch persist hook (PHASE4 chunk 4).
    batch_config: BatchIngestionConfig,
    /// On-disk location to flush the HNSW index to when
    /// `batch_config.persist_every` fires. `None` (default) disables
    /// mid-run persist regardless of `persist_every`; callers that want
    /// streaming persistence opt in via [`Self::with_corpus_dir`]. The
    /// path is forwarded straight to `VectorIndex::persist`, so it must
    /// be the same directory the caller hands off to bundle export.
    corpus_dir: Option<PathBuf>,
}

impl Default for IngestionPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl IngestionPipeline {
    #[must_use]
    pub fn new() -> Self {
        Self {
            parser_override: None,
            min_section_tokens: 50,
            claim_extractor: HeuristicClaimExtractor::new(),
            summary_generator: ExtractiveSummaryGenerator::new(),
            relationship_detector: HeuristicRelationshipDetector::new(),
            progress: None,
            package_graph: None,
            concurrency: None,
            dual_embedder: None,
            full_dim_storage: None,
            batch_config: BatchIngestionConfig::default(),
            corpus_dir: None,
        }
    }

    /// Configure streaming-ingestion knobs. With
    /// [`BatchIngestionConfig::persist_every`] set AND a corpus dir
    /// configured via [`Self::with_corpus_dir`], the producer flushes
    /// the HNSW index to disk every N files indexed. Either knob
    /// missing skips the mid-run flush — useful for tests + the local
    /// `ministr index` path that bundles at end-of-ingest.
    #[must_use]
    pub fn with_batch_config(mut self, cfg: BatchIngestionConfig) -> Self {
        self.batch_config = cfg;
        self
    }

    /// Provide the on-disk location for [`VectorIndex::persist`] to
    /// write to when [`BatchIngestionConfig::persist_every`] fires.
    /// Must match the directory the caller will bundle/upload at the
    /// end of ingest — otherwise the streamed snapshots are pointing
    /// to a dir that nobody reads.
    #[must_use]
    pub fn with_corpus_dir(mut self, dir: PathBuf) -> Self {
        self.corpus_dir = Some(dir);
        self
    }

    /// Enable two-stage Matryoshka retrieval during ingestion.
    ///
    /// When set, the embedding consumer calls `embed_dual()` to produce both
    /// truncated vectors (inserted into HNSW) and full-dimension vectors
    /// (stored in SQLite for query-time reranking).
    #[must_use]
    pub fn with_dual_embedder(
        mut self,
        dual_embedder: Arc<dyn crate::embedding::DualEmbedder>,
        storage: crate::storage::SqliteStorage,
    ) -> Self {
        self.dual_embedder = Some(dual_embedder);
        self.full_dim_storage = Some(storage);
        self
    }

    #[must_use]
    pub fn with_parser(kind: ParserKind) -> Self {
        Self {
            parser_override: Some(kind),
            ..Self::new()
        }
    }

    #[must_use]
    pub fn with_progress(mut self, progress: Arc<IngestionProgress>) -> Self {
        self.progress = Some(progress);
        self
    }

    #[must_use]
    pub fn with_min_section_tokens(mut self, min_tokens: usize) -> Self {
        self.min_section_tokens = min_tokens;
        self
    }

    #[must_use]
    pub fn with_package_graph(mut self, graph: PackageGraph) -> Self {
        self.package_graph = Some(graph);
        self
    }

    #[must_use]
    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.concurrency = Some(n);
        self
    }

    fn parser_for(&self, path: &Path) -> Box<dyn DocumentParser> {
        if let Some(kind) = self.parser_override {
            return create_parser(kind);
        }
        if let Some(kind) = detect_parser_kind(path) {
            return create_parser(kind);
        }
        Box::new(MarkdownParser::new())
    }

    // ── Entry point 1: ingest_directory (no embeddings) ──────────────────

    #[instrument(skip(self, storage), fields(dir = %dir.display()))]
    pub async fn ingest_directory<S: Storage>(
        &self,
        dir: &Path,
        storage: &S,
    ) -> Result<IngestionStats, IngestionError> {
        let files = discover_files(dir)?;
        let mut stats = IngestionStats::new(files.len());

        if files.is_empty() {
            warn!("discovered 0 files for ingestion");
        } else {
            info!(count = files.len(), "discovered files for ingestion");
        }

        for file_path in &files {
            let relative = file_path
                .strip_prefix(dir)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            match self.ingest_file(file_path, &relative, storage).await {
                Ok(FileResult::Skipped) => {
                    debug!(path = %relative, "unchanged, skipping");
                    stats.files_skipped += 1;
                }
                Ok(FileResult::Indexed {
                    sections, claims, ..
                }) => {
                    debug!(path = %relative, sections, claims, "indexed");
                    stats.files_indexed += 1;
                    stats.total_sections += sections;
                    stats.total_claims += claims;
                }
                Err(e) => {
                    warn!(path = %relative, error = %e, "failed to ingest file");
                    stats.files_failed += 1;
                }
            }
        }

        // Remove documents for files that no longer exist
        let existing_docs = storage
            .list_documents()
            .await
            .map_err(IngestionError::from)?;
        for doc in &existing_docs {
            let full_path = dir.join(&doc.source_path);
            if !full_path.exists() {
                debug!(path = %doc.source_path, "file removed, deleting from index");
                storage
                    .delete_document(&doc.id)
                    .await
                    .map_err(IngestionError::from)?;
                storage
                    .delete_file_hash(&doc.source_path)
                    .await
                    .map_err(IngestionError::from)?;
                stats.files_removed += 1;
            }
        }

        info!(
            indexed = stats.files_indexed,
            skipped = stats.files_skipped,
            removed = stats.files_removed,
            failed = stats.files_failed,
            "ingestion complete"
        );

        Ok(stats)
    }

    /// Ingest a single file without embeddings.
    #[instrument(skip(self, storage), fields(path = %relative_path))]
    async fn ingest_file<S: Storage>(
        &self,
        file_path: &Path,
        relative_path: &str,
        storage: &S,
    ) -> Result<FileResult, IngestionError> {
        let file_mtime_ns = file_mtime_nanos(file_path).await;

        let existing_hash = storage
            .get_file_hash(relative_path)
            .await
            .map_err(IngestionError::from)?;

        // Both skip paths below require the cached record to have been
        // produced by the CURRENT extractor AND resolver versions. When
        // either logic changes (bumping `EXTRACTOR_VERSION` or
        // `RESOLVER_VERSION`), the stored row compares < current, so we
        // fall through and re-parse — the index auto-heals without a
        // manual corpus wipe. Resolver-stale + extractor-fresh files
        // could in principle skip re-parse and re-resolve in place; for
        // bulk auto-heal of an already-indexed corpus that path lives
        // in `re_resolve_stale_files` and runs on daemon startup. Here
        // in the per-file ingest path we conservatively re-parse —
        // tree-sitter is cheap and this keeps the file-watcher
        // semantics simple.
        let extractor_fresh = existing_hash
            .as_ref()
            .is_some_and(|e| e.extractor_version >= super::EXTRACTOR_VERSION);
        let resolver_fresh = existing_hash
            .as_ref()
            .is_some_and(|e| e.resolver_version >= super::RESOLVER_VERSION);
        let cache_fresh = extractor_fresh && resolver_fresh;

        if cache_fresh
            && let Some(ref existing) = existing_hash
            && let (Some(stored_mtime), Some(current_mtime)) = (existing.mtime_ns, file_mtime_ns)
            && stored_mtime == current_mtime
        {
            return Ok(FileResult::Skipped);
        }

        let content = tokio::fs::read(file_path)
            .await
            .map_err(|e| IngestionError::Io {
                path: file_path.to_path_buf(),
                source: e,
            })?;

        let content_str = String::from_utf8(content).map_err(|_| IngestionError::Encoding {
            path: file_path.to_path_buf(),
        })?;

        let hash = compute_content_hash(&content_str);

        if cache_fresh
            && let Some(ref existing) = existing_hash
            && existing.content_hash == hash
        {
            storage
                .upsert_file_hash(&FileHashRecord {
                    path: relative_path.to_string(),
                    content_hash: hash,
                    mtime_ns: file_mtime_ns,
                    extractor_version: super::EXTRACTOR_VERSION,
                    resolver_version: super::RESOLVER_VERSION,
                })
                .await
                .map_err(IngestionError::from)?;
            return Ok(FileResult::Skipped);
        }

        let parser = self.parser_for(Path::new(relative_path));
        let mut doc = parser.parse(Path::new(relative_path), &content_str)?;

        let result = store_enriched_document(
            &mut doc,
            relative_path,
            storage,
            &self.claim_extractor,
            &self.summary_generator,
            &self.relationship_detector,
            self.min_section_tokens,
            existing_hash.is_some(),
            None::<&NullVectorIndex>,
            ProcessOptions {
                hash_path: Some(relative_path),
                content_hash: Some(hash),
                mtime_ns: file_mtime_ns,
            },
        )
        .await?;

        Ok(FileResult::Indexed {
            sections: result.section_count,
            claims: result.claim_count,
            pending_refs: Vec::new(),
            embedding_pairs: Vec::new(),
        })
    }

    // ── Entry point 2: ingest_content (no embeddings) ────────────────────

    #[instrument(skip(self, content, storage), fields(source = %source_path))]
    pub async fn ingest_content<S: Storage>(
        &self,
        source_path: &str,
        content: &str,
        parser_kind: ParserKind,
        storage: &S,
    ) -> Result<ContentIngestionStats, IngestionError> {
        let hash = compute_content_hash(content);

        let existing_hash = storage
            .get_file_hash(source_path)
            .await
            .map_err(IngestionError::from)?;

        if let Some(ref existing) = existing_hash
            && existing.content_hash == hash
        {
            return Ok(ContentIngestionStats {
                sections: 0,
                claims: 0,
                skipped: true,
            });
        }

        let parser = create_parser(parser_kind);
        let mut doc = parser.parse(Path::new(source_path), content)?;

        let result = store_enriched_document(
            &mut doc,
            source_path,
            storage,
            &self.claim_extractor,
            &self.summary_generator,
            &self.relationship_detector,
            self.min_section_tokens,
            existing_hash.is_some(),
            None::<&NullVectorIndex>,
            ProcessOptions {
                hash_path: Some(source_path),
                content_hash: Some(hash),
                mtime_ns: None,
            },
        )
        .await?;

        info!(source = %source_path, result.section_count, result.claim_count, "ingested content");

        Ok(ContentIngestionStats {
            sections: result.section_count,
            claims: result.claim_count,
            skipped: false,
        })
    }

    // ── Entry point 3: ingest_content_with_embeddings ────────────────────

    #[instrument(skip(self, content, storage, embedder, index), fields(source = %source_path))]
    pub async fn ingest_content_with_embeddings<S, E, I>(
        &self,
        source_path: &str,
        content: &str,
        parser_kind: ParserKind,
        storage: &S,
        embedder: &E,
        index: &I,
    ) -> Result<ContentIngestionStats, IngestionError>
    where
        S: Storage + ?Sized,
        E: Embedder + ?Sized,
        I: VectorIndex + ?Sized,
    {
        let hash = compute_content_hash(content);

        let existing_hash = storage
            .get_file_hash(source_path)
            .await
            .map_err(IngestionError::from)?;

        if let Some(ref existing) = existing_hash
            && existing.content_hash == hash
        {
            return Ok(ContentIngestionStats {
                sections: 0,
                claims: 0,
                skipped: true,
            });
        }

        let parser = create_parser(parser_kind);
        let mut doc = parser.parse(Path::new(source_path), content)?;

        let result = store_enriched_document(
            &mut doc,
            source_path,
            storage,
            &self.claim_extractor,
            &self.summary_generator,
            &self.relationship_detector,
            self.min_section_tokens,
            existing_hash.is_some(),
            Some(index),
            ProcessOptions {
                hash_path: Some(source_path),
                content_hash: Some(hash),
                mtime_ns: None,
            },
        )
        .await?;

        // Embed all resolution levels (immediate)
        embed_document(&doc, embedder, index)?;

        // For code files: extract symbols and embed immediately
        if parser_kind == ParserKind::Code {
            let sym_result = extract_code_symbols(source_path, content, storage, None).await?;
            if !sym_result.embedding_pairs.is_empty() {
                batch_embed_and_insert(&sym_result.embedding_pairs, embedder, index).await?;
            }
        }

        info!(source = %source_path, result.section_count, result.claim_count, "ingested content with embeddings");

        Ok(ContentIngestionStats {
            sections: result.section_count,
            claims: result.claim_count,
            skipped: false,
        })
    }

    // ── Entry point 4: directory with embeddings ─────────────────────────

    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, storage, embedder, index), fields(dir = %dir.display()))]
    pub async fn ingest_directory_with_embeddings<S, E, I>(
        &self,
        dir: &Path,
        storage: &S,
        embedder: &E,
        index: &I,
    ) -> Result<IngestionStats, IngestionError>
    where
        S: Storage + ?Sized,
        E: Embedder + ?Sized,
        I: VectorIndex + ?Sized,
    {
        self.ingest_directory_with_embeddings_rooted(dir, storage, embedder, index, None, None)
            .await
    }

    #[allow(clippy::too_many_lines)] // orchestration entry point — each step is unique
    pub async fn ingest_directory_with_embeddings_rooted<S, E, I>(
        &self,
        dir: &Path,
        storage: &S,
        embedder: &E,
        index: &I,
        root_id: Option<&str>,
        ct: Option<&CancellationToken>,
    ) -> Result<IngestionStats, IngestionError>
    where
        S: Storage + ?Sized,
        E: Embedder + ?Sized,
        I: VectorIndex + ?Sized,
    {
        let detected_graph = if self.package_graph.is_none() {
            crate::workspace::detect_workspace(dir).and_then(|ws| {
                let graph = PackageGraph::from_cargo_workspace(dir, &ws.members);
                if graph.is_empty() {
                    None
                } else {
                    info!(
                        packages = graph.packages().len(),
                        kind = %ws.kind,
                        "detected workspace, built package graph for cross-crate resolution"
                    );
                    Some(graph)
                }
            })
        } else {
            None
        };
        let active_graph = self.package_graph.as_ref().or(detected_graph.as_ref());

        if let Some(ref progress) = self.progress {
            progress.set_phase(IngestionPhase::Discovering);
        }

        // Phase 7 (corpus-root stat-merkle short-circuit). Walk once,
        // computing both the file list and a sorted BLAKE3 over each
        // file's (rel_path, mtime_ns, size). When the freshly-computed
        // hash matches the stored value for this corpus_id, we know
        // *no file has been touched* since the last successful index
        // and can return immediately — no parse, no embed, no SQL
        // churn. Saves minutes-to-hours on `git pull && reindex` of
        // large corpora that didn't actually change.
        //
        // When root_id is None (test / unrooted callers) we keep the
        // legacy `discover_files` path — there's no key under which to
        // remember the fingerprint.
        let (root_hash, files): (Option<String>, Vec<PathBuf>) = if root_id.is_some() {
            let (h, f) = super::discovery::compute_corpus_stat_merkle(dir)?;
            (Some(h), f)
        } else {
            (None, discover_files(dir)?)
        };

        if let (Some(rid), Some(hash)) = (root_id, &root_hash)
            && let Ok(Some(prior)) = storage.get_corpus_merkle(rid).await
            && prior.root_hash == *hash
        {
            if prior.extractor_version == super::EXTRACTOR_VERSION {
                info!(
                    corpus_id = rid,
                    file_count = files.len(),
                    extractor_version = super::EXTRACTOR_VERSION,
                    "corpus stat-merkle unchanged — short-circuiting reindex"
                );
                let mut stats = IngestionStats::new(files.len());
                stats.files_skipped = files.len();
                if let Some(ref progress) = self.progress {
                    progress.complete();
                }
                return Ok(stats);
            }
            info!(
                corpus_id = rid,
                stored_extractor_version = prior.extractor_version,
                current_extractor_version = super::EXTRACTOR_VERSION,
                "corpus stat-merkle matches but extractor version differs — re-extracting"
            );
        }

        let mut stats = IngestionStats::new(files.len());

        if files.is_empty() {
            warn!("discovered 0 files for ingestion (with embeddings)");
        } else {
            info!(
                count = files.len(),
                "discovered files for ingestion (with embeddings)"
            );
        }

        if let Some(ref progress) = self.progress {
            progress.start(files.len());
        }

        // Register this corpus root *before* the producer/consumer loop:
        // run_producer_consumer calls set_document_root per file, and that
        // UPDATE is FK-constrained to corpus_roots, so the row must already
        // exist or root_id silently stays NULL (the error is logged at
        // debug and swallowed). The rooted entry point is otherwise the one
        // ingestion path that never registers its root — only
        // ingest_paths_with_embeddings did — which left root_id NULL and
        // list_documents_by_root / list_corpus_roots empty for every
        // rooted-path corpus. Guard on a non-empty discovery so a transient
        // unreadable root can't stomp a prior good file_count with 0 (same
        // footing as the orphan-sweep guard below).
        if let Some(rid) = root_id
            && !files.is_empty()
        {
            let roots = [(dir.to_path_buf(), rid.to_string())];
            let mut root_lang_stats: std::collections::HashMap<
                String,
                std::collections::HashMap<String, usize>,
            > = std::collections::HashMap::new();
            let mut root_file_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            accumulate_language_stats(&files, &roots, &mut root_lang_stats, &mut root_file_counts);
            update_root_stats(storage, &roots, &root_lang_stats, &root_file_counts).await;
        }

        // Union the upward walk from the root with a scan of every manifest
        // discovered *below* it — otherwise a monorepo's subdirectory app
        // (e.g. a Tauri app under `<repo>/app/src-tauri/`) never registers its
        // bridge framework and its commands never link.
        let detected_kinds: Vec<BridgeKind> = {
            let mut set: std::collections::BTreeSet<BridgeKind> =
                detector::FrameworkDetector::detect(dir)
                    .into_iter()
                    .collect();
            set.extend(detector::FrameworkDetector::detect_in_files(&files));
            set.into_iter().collect()
        };
        let bridge_linker = create_linker_for_kinds(&detected_kinds);
        if !detected_kinds.is_empty() {
            info!(
                kinds = ?detected_kinds,
                "detected bridge frameworks for cross-language linking"
            );
        }

        mem_profile::checkpoint("ingestion loop start (rooted)");

        let file_items: Vec<FileItem> = files
            .iter()
            .map(|file_path| {
                let raw_relative = file_path
                    .strip_prefix(dir)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .to_string();
                let relative = match root_id {
                    Some(rid) => namespace_path(rid, &raw_relative),
                    None => raw_relative,
                };
                FileItem {
                    path: file_path.clone(),
                    relative,
                    root_id: root_id.map(String::from),
                    root_path: Some(dir.to_path_buf()),
                }
            })
            .collect();

        // Snapshot (abs_path, relative_path) pairs for the bridge rebuild
        // pass. run_producer_consumer consumes file_items, but
        // finalize_ingestion needs the full set even when files were
        // fast-skipped by content hash.
        let all_files_for_bridges: Vec<(PathBuf, String)> = file_items
            .iter()
            .map(|f| (f.path.clone(), f.relative.clone()))
            .collect();

        let (was_cancelled, embed_count, pending_refs) = self
            .run_producer_consumer(
                file_items,
                storage,
                embedder,
                index,
                active_graph,
                &mut stats,
                ct,
            )
            .await?;

        if was_cancelled {
            info!(indexed = stats.files_indexed, "ingestion cancelled");
            return Err(IngestionError::Cancelled);
        }
        stats.total_embeddings = embed_count;

        self.finalize_ingestion(
            &pending_refs,
            &all_files_for_bridges,
            bridge_linker.as_ref(),
            active_graph,
            storage,
        )
        .await;

        // Cleanup stale docs (orphan GC) — keying-aware global sweep.
        //
        // Enumerate *every* document, not just those under the root being
        // ingested. A document whose `root_id` is NULL (ingested via the
        // unrooted entry point, by the CLI/worker, or before roots existed)
        // is invisible to `list_documents_by_root(rid)` and would otherwise
        // orphan forever — surviving every reindex of its own corpus. We
        // mirror the global discovered-set diff the multi-path entry point
        // uses (F32) so behaviour is consistent across entry points, but
        // scope *deletion* by document ownership so a sibling root's
        // documents in a shared multi-root index are never false-deleted.
        //
        // Guard: skip the sweep entirely on empty discovery. An empty
        // `files` set is far more likely a transient unreadable / unmounted
        // root than a genuinely emptied corpus, and a global diff would
        // then delete every attributable document in the index.
        if files.is_empty() {
            warn!(
                "skipping stale-doc cleanup: discovery returned 0 files (likely a transient unreadable root, not an emptied corpus)"
            );
        } else {
            // Both stored forms a document under this dir can take:
            // namespaced (`rid/rel`, root_id = Some) and bare-relative
            // (`rel`, root_id = NULL). Keyed exactly as ingestion keys them
            // above, so membership is an equality test, not a `stat`.
            let discovered: std::collections::HashSet<String> = files
                .iter()
                .flat_map(|file_path| {
                    let raw = file_path
                        .strip_prefix(dir)
                        .unwrap_or(file_path)
                        .to_string_lossy()
                        .into_owned();
                    match root_id {
                        Some(rid) => vec![namespace_path(rid, &raw), raw],
                        None => vec![raw],
                    }
                })
                .collect();

            let existing_docs = storage
                .list_documents()
                .await
                .map_err(IngestionError::from)?;

            // Ownership is read from the source_path namespace prefix
            // (`rid/…`), set by `namespace_path` at ingest. The `root_id`
            // *column* is not written on this code path, so scope must not
            // key off it. `this_prefix` is what this run namespaces under
            // (None for an unrooted reindex).
            let this_prefix = root_id.map(|rid| format!("{rid}/"));

            // Does a *foreign* explicit root share this index? A bare-
            // relative (NULL-keyed) document carries no attribution, so we
            // reclaim such documents only when none is present — i.e. a
            // single-corpus CLI/worker index, the only place NULL-keyed
            // documents legitimately arise. In a genuine multi-root index
            // they could belong to any sibling corpus.
            let has_foreign_root = existing_docs.iter().any(|d| {
                strip_root_prefix(&d.source_path).is_some()
                    && this_prefix
                        .as_deref()
                        .is_none_or(|p| !d.source_path.starts_with(p))
            });

            for doc in &existing_docs {
                // Never delete a document this run isn't authoritative for:
                //   namespaced under our rid → ours.
                //   namespaced under another → a sibling root, skip.
                //   bare-relative (NULL-keyed) → ours only on an unrooted
                //     reindex, or when no sibling root shares the index
                //     (the F32.1 fix).
                let attributable = if strip_root_prefix(&doc.source_path).is_some() {
                    this_prefix
                        .as_deref()
                        .is_some_and(|p| doc.source_path.starts_with(p))
                } else {
                    root_id.is_none() || !has_foreign_root
                };
                if !attributable || discovered.contains(&doc.source_path) {
                    continue;
                }
                debug!(path = %doc.source_path, "file removed, deleting from index");
                super::embedding::delete_document_vectors(&doc.id, storage, index).await?;
                storage
                    .delete_document(&doc.id)
                    .await
                    .map_err(IngestionError::from)?;
                storage
                    .delete_file_hash(&doc.source_path)
                    .await
                    .map_err(IngestionError::from)?;
                // Symbols live in their own table keyed by file_path (NOT
                // cascaded by delete_document); without this they'd keep
                // surfacing in symbol search forever. Runs after
                // delete_document_vectors so the HNSW teardown could still
                // enumerate them. symbol_refs cascade off symbols via FK.
                storage
                    .delete_symbols_for_file(&doc.source_path)
                    .await
                    .map_err(IngestionError::from)?;
                stats.files_removed += 1;
            }
        }

        info!(
            indexed = stats.files_indexed,
            skipped = stats.files_skipped,
            removed = stats.files_removed,
            failed = stats.files_failed,
            "ingestion with embeddings complete"
        );

        // Phase 7 — record the new stat-merkle so the *next* reindex can
        // short-circuit. Only persist when (a) we have a corpus key to
        // store under and (b) we successfully computed a hash earlier
        // (i.e. we took the rooted code path).
        if let (Some(rid), Some(hash)) = (root_id, root_hash) {
            let now_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_nanos()).ok())
                .unwrap_or(0);
            let record = crate::storage::traits::CorpusMerkleRecord {
                corpus_id: rid.to_string(),
                root_hash: hash,
                file_count: i64::try_from(stats.files_discovered).unwrap_or(i64::MAX),
                last_indexed_ns: now_ns,
                extractor_version: super::EXTRACTOR_VERSION,
            };
            if let Err(e) = storage.upsert_corpus_merkle(&record).await {
                // Don't fail the ingestion just because the merkle
                // upsert blew up — the corpus is correctly indexed,
                // we just lose the short-circuit on the next run.
                tracing::warn!(error = ?e, "failed to upsert corpus_merkle (non-fatal)");
            }
        }

        if let Some(ref progress) = self.progress {
            progress.complete();
        }

        Ok(stats)
    }

    // ── Entry point 5: multi-path with embeddings ────────────────────────

    #[instrument(skip(self, storage, embedder, index), fields(path_count = paths.len()))]
    #[allow(clippy::too_many_lines)] // orchestration entry point — each step is unique
    pub async fn ingest_paths_with_embeddings<S, E, I>(
        &self,
        paths: &[PathBuf],
        storage: &S,
        embedder: &E,
        index: &I,
    ) -> Result<IngestionStats, IngestionError>
    where
        S: Storage + ?Sized,
        E: Embedder + ?Sized,
        I: VectorIndex + ?Sized,
    {
        let active_graph = self.package_graph.as_ref();

        if let Some(ref progress) = self.progress {
            progress.set_phase(IngestionPhase::Discovering);
        }

        let files = discover_paths(paths)?;
        let mut stats = IngestionStats::new(files.len());

        if files.is_empty() {
            warn!("discovered 0 files from multiple paths (with embeddings)");
        } else {
            info!(
                count = files.len(),
                "discovered files from multiple paths (with embeddings)"
            );
        }

        // Register corpus roots
        let roots: Vec<(PathBuf, String)> = paths
            .iter()
            .filter(|p| p.is_dir())
            .map(|p| {
                let root_id = compute_root_id(p);
                (p.clone(), root_id)
            })
            .collect();

        for (root_path, root_id) in &roots {
            let display_name = root_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string());
            let root = CorpusRoot {
                id: root_id.clone(),
                path: root_path.to_string_lossy().to_string(),
                kind: RootKind::Local,
                display_name,
                file_count: 0,
                language_stats: std::collections::HashMap::new(),
                repo_url: None,
                branch: None,
                commit_sha: None,
                clone_timestamp: None,
                sparse_paths: Vec::new(),
            };
            if let Err(e) = storage.upsert_corpus_root(&root).await {
                warn!(root_id = %root_id, error = %e, "failed to register corpus root");
            }
        }

        let mut root_lang_stats: std::collections::HashMap<
            String,
            std::collections::HashMap<String, usize>,
        > = std::collections::HashMap::new();
        let mut root_file_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        // Detect bridge frameworks upfront — needed both for the normal path
        // *and* to decide whether the mtime fast-skip below is safe (bug #3:
        // the fast-skip used to bypass bridge linking entirely, so a corpus
        // with detected bridges could never rebuild its cross-language links
        // on an unchanged rebuild).
        let mut all_bridge_kinds = std::collections::BTreeSet::new();
        for path in paths {
            if path.is_dir() {
                let kinds = detector::FrameworkDetector::detect(path);
                all_bridge_kinds.extend(kinds);
            }
        }
        // Also scan manifests in subdirectories of the discovered file set, so
        // a monorepo app declared below the indexed path(s) still links (see
        // FrameworkDetector::detect_in_files).
        all_bridge_kinds.extend(detector::FrameworkDetector::detect_in_files(&files));
        let bridge_kinds: Vec<BridgeKind> = all_bridge_kinds.into_iter().collect();
        let bridge_linker = create_linker_for_kinds(&bridge_kinds);
        if !bridge_kinds.is_empty() {
            info!(
                kinds = ?bridge_kinds,
                "detected bridge frameworks for cross-language linking"
            );
        }

        // Manifest-level mtime fast skip — only taken when no bridge kinds
        // were detected, since bridge linking runs only inside the full pass.
        if bridge_kinds.is_empty()
            && !files.is_empty()
            && let Ok(true) = all_files_unchanged_by_mtime(&files, paths, storage).await
        {
            info!(
                files = files.len(),
                "all files unchanged (mtime fast skip) — skipping ingestion"
            );
            stats.files_skipped = files.len();
            if let Some(ref progress) = self.progress {
                progress.start(files.len());
                for _ in 0..files.len() {
                    progress.increment_done();
                }
                progress.complete();
            }
            repair_missing_refs(storage, None).await;

            accumulate_language_stats(&files, &roots, &mut root_lang_stats, &mut root_file_counts);
            update_root_stats(storage, &roots, &root_lang_stats, &root_file_counts).await;
            return Ok(stats);
        }

        if let Some(ref progress) = self.progress {
            progress.start(files.len());
        }

        mem_profile::checkpoint("ingestion loop start (paths)");

        accumulate_language_stats(&files, &roots, &mut root_lang_stats, &mut root_file_counts);

        let file_items: Vec<FileItem> = files
            .iter()
            .map(|file_path| {
                let relative = compute_relative_path(file_path, paths);
                let (root_path, root_id) = match find_root_entry_for_file(file_path, &roots) {
                    Some((p, id)) => (Some(p.clone()), Some(id.clone())),
                    None => (None, None),
                };
                FileItem {
                    path: file_path.clone(),
                    relative,
                    root_id,
                    root_path,
                }
            })
            .collect();

        // Snapshot (abs_path, relative_path) pairs for the bridge rebuild
        // pass. run_producer_consumer consumes file_items, but
        // finalize_ingestion needs the full set even when files were
        // fast-skipped by content hash.
        let all_files_for_bridges: Vec<(PathBuf, String)> = file_items
            .iter()
            .map(|f| (f.path.clone(), f.relative.clone()))
            .collect();

        let (_was_cancelled, embed_count, pending_refs) = self
            .run_producer_consumer(
                file_items,
                storage,
                embedder,
                index,
                active_graph,
                &mut stats,
                None,
            )
            .await?;
        stats.total_embeddings = embed_count;

        self.finalize_ingestion(
            &pending_refs,
            &all_files_for_bridges,
            bridge_linker.as_ref(),
            active_graph,
            storage,
        )
        .await;

        // Cleanup stale docs (orphan GC). Enumerate *every* document in
        // the corpus, not just those under a currently-registered root.
        // A document whose `root_id` is NULL, points at a root no longer
        // in `paths`, or whose root directory was deleted out-of-band
        // (a bulk `git rm`, a branch switch, a crate moved to another
        // repo) is never returned by `list_documents_by_root` and would
        // otherwise orphan forever — its sections, symbols, vectors and
        // `file_hashes` row surviving every reindex. A single global diff
        // against the discovered set catches all of those and is O(docs)
        // instead of the old O(roots × docs × files).
        //
        // Guard: skip the sweep entirely when discovery turned up zero
        // files. An empty `files` set is far more likely a transient
        // unreadable / unmounted root than a genuinely emptied corpus,
        // and proceeding would delete every document in the index.
        if files.is_empty() {
            warn!(
                "skipping stale-doc cleanup: discovery returned 0 files (likely a transient unreadable root, not an emptied corpus)"
            );
        } else {
            let discovered: std::collections::HashSet<String> = files
                .iter()
                .map(|f| compute_relative_path(f, paths))
                .collect();
            let existing_docs = storage
                .list_documents()
                .await
                .map_err(IngestionError::from)?;
            for doc in &existing_docs {
                if discovered.contains(&doc.source_path) {
                    continue;
                }
                debug!(path = %doc.source_path, "file removed, deleting from index");
                super::embedding::delete_document_vectors(&doc.id, storage, index).await?;
                storage
                    .delete_document(&doc.id)
                    .await
                    .map_err(IngestionError::from)?;
                storage
                    .delete_file_hash(&doc.source_path)
                    .await
                    .map_err(IngestionError::from)?;
                // Symbols live in their own table keyed by file_path (NOT
                // cascaded by delete_document); without this they'd keep
                // surfacing in symbol search forever. Runs after
                // delete_document_vectors so the HNSW teardown could still
                // enumerate them. symbol_refs cascade off symbols via FK.
                storage
                    .delete_symbols_for_file(&doc.source_path)
                    .await
                    .map_err(IngestionError::from)?;
                stats.files_removed += 1;
            }
        }

        update_root_stats(storage, &roots, &root_lang_stats, &root_file_counts).await;

        info!(
            indexed = stats.files_indexed,
            skipped = stats.files_skipped,
            removed = stats.files_removed,
            failed = stats.files_failed,
            "multi-path ingestion with embeddings complete"
        );

        if let Some(ref progress) = self.progress {
            progress.complete();
        }

        Ok(stats)
    }

    // ── Shared producer/consumer pipeline ────────────────────────────────

    /// Run the concurrent producer/consumer pipeline shared by both ingestion
    /// entry points. Returns `(was_cancelled, embedding_count, pending_refs, bridge_endpoints)`.
    ///
    /// ## Failure semantics
    ///
    /// If the embedding consumer errors mid-stream, the shared
    /// [`CancellationToken`] is tripped so the producer stops queueing new
    /// parses. Every document the producer persisted but whose embeddings
    /// didn't complete is then **rolled back** (storage records + any
    /// already-written vectors) before the error is returned, so SQLite and
    /// the vector index never disagree about whether a file was indexed.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn run_producer_consumer<S, E, I>(
        &self,
        file_items: Vec<FileItem>,
        storage: &S,
        embedder: &E,
        index: &I,
        active_graph: Option<&PackageGraph>,
        stats: &mut IngestionStats,
        ct: Option<&CancellationToken>,
    ) -> Result<(bool, usize, Vec<PendingRef>), IngestionError>
    where
        S: Storage + ?Sized,
        E: Embedder + ?Sized,
        I: VectorIndex + ?Sized,
    {
        let concurrency = self.concurrency.unwrap_or_else(default_concurrency);
        info!(
            concurrency,
            files = file_items.len(),
            "starting concurrent file ingestion"
        );

        let (embed_tx, embed_rx) = tokio::sync::mpsc::channel::<Vec<(VectorId, String)>>(16);

        let mut all_pending_refs = Vec::new();
        // PHASE4 chunk 4: `all_bridge_endpoints` used to be accumulated
        // here, returned, and immediately discarded by both callers —
        // `finalize_ingestion` rebuilds bridge data from `all_files`
        // (see its doc comment) so the per-file batch is dead weight on
        // a code-heavy corpus. Removed entirely.

        // Shared cancel signal: when the consumer errors it trips this token
        // so the producer stops scheduling new parses and exits promptly.
        // Combined with any caller-supplied token via a parent/child check.
        let internal_ct = CancellationToken::new();
        let external_ct = ct;

        // Track every document we actually persisted so we can roll back on
        // embedding failure (bug #1: partial-write corpus corruption).
        let indexed_doc_ids: std::sync::Mutex<Vec<crate::types::ContentId>> =
            std::sync::Mutex::new(Vec::new());

        let producer = async {
            let mut cancelled = false;
            let mut parse_stream = std::pin::pin!(
                stream::iter(file_items)
                    .take_while(|_| {
                        let external_stop =
                            external_ct.is_some_and(CancellationToken::is_cancelled);
                        let internal_stop = internal_ct.is_cancelled();
                        let stop = external_stop || internal_stop;
                        async move { !stop }
                    })
                    .map(|item| {
                        // Bug #6: announce the file as *started* — before the
                        // parse kicks off — so the UI shows work in progress,
                        // not the previous finished file.
                        if let Some(ref progress) = self.progress {
                            progress.set_current_file(&item.relative);
                        }
                        let internal_ct = internal_ct.clone();
                        async move {
                            // Bug #2 (partial): check cancellation at parse
                            // entry so futures that `buffer_unordered` queued
                            // before a cancel fires don't spend CPU parsing a
                            // file the caller has already abandoned. Inner
                            // parse steps remain non-cancelable — threading
                            // the token through tree-sitter + extractors is
                            // a follow-up.
                            if internal_ct.is_cancelled()
                                || external_ct.is_some_and(CancellationToken::is_cancelled)
                            {
                                return (item, Ok(FileResult::Skipped));
                            }
                            let result = self
                                .parse_and_store_file(
                                    &item.path,
                                    &item.relative,
                                    item.root_path.as_deref(),
                                    storage,
                                    index,
                                    active_graph,
                                )
                                .await;
                            (item, result)
                        }
                    })
                    .buffer_unordered(concurrency)
            );

            while let Some((item, result)) = parse_stream.next().await {
                match result {
                    Ok(FileResult::Skipped) => {
                        debug!(path = %item.relative, "unchanged, skipping");
                        stats.files_skipped += 1;
                    }
                    Ok(FileResult::Indexed {
                        sections,
                        claims,
                        pending_refs,
                        embedding_pairs,
                    }) => {
                        debug!(path = %item.relative, sections, claims, "parsed and stored");
                        stats.files_indexed += 1;
                        stats.total_sections += sections;
                        stats.total_claims += claims;
                        all_pending_refs.extend(pending_refs);

                        // PHASE4 chunk 4: periodic HNSW persist. Fires
                        // only when *both* `persist_every` and a
                        // `corpus_dir` are configured (callers that
                        // bundle at end-of-ingest leave corpus_dir
                        // unset — see [`with_corpus_dir`]). HNSW
                        // persist is atomic (tmp-rename + fsync), so
                        // we hold the in-memory graph for ongoing
                        // inserts and the persisted snapshot is a
                        // recoverable point-in-time copy. Sync call
                        // inside an async block: persist is fast
                        // enough on hot-disk to not warrant
                        // spawn_blocking for the current corpus sizes.
                        //
                        // PHASE5 chunk 2: gate the call on
                        // `!index.is_empty()`. `stats.files_indexed`
                        // is bumped by the producer the moment a file
                        // is parsed + stored, but the embedder runs
                        // concurrently and may not have flushed any
                        // vectors yet — the parser can race ahead by
                        // many files. Calling `index.persist()` on an
                        // empty HNSW returns "nb point 0" + a WARN log
                        // every persist_every boundary until the
                        // consumer catches up, which is loud and
                        // useless. Mirrors PHASE3 Fix A's spirit for
                        // the streaming path.
                        if let (Some(n), Some(dir)) =
                            (self.batch_config.persist_every, self.corpus_dir.as_ref())
                            && n != 0
                            && stats.files_indexed.is_multiple_of(n)
                        {
                            if index.is_empty() {
                                trace!(
                                    files_indexed = stats.files_indexed,
                                    "skipping mid-run HNSW persist: index has no vectors yet",
                                );
                            } else {
                                match index.persist(dir) {
                                    Ok(()) => debug!(
                                        files_indexed = stats.files_indexed,
                                        dir = %dir.display(),
                                        "mid-run HNSW persist snapshot"
                                    ),
                                    Err(e) => warn!(
                                        files_indexed = stats.files_indexed,
                                        error = %e,
                                        "mid-run HNSW persist failed; continuing"
                                    ),
                                }
                            }
                        }

                        // Track this doc for rollback on consumer failure.
                        let doc_id = crate::types::ContentId(item.relative.clone());
                        if let Ok(mut guard) = indexed_doc_ids.lock() {
                            guard.push(doc_id.clone());
                        }

                        if let Some(ref progress) = self.progress {
                            progress.add_sections_done(sections);
                        }

                        if let Some(ref rid) = item.root_id
                            && let Err(e) = storage.set_document_root(&doc_id, rid).await
                        {
                            debug!(path = %item.relative, error = %e, "failed to set document root");
                        }

                        if !embedding_pairs.is_empty() {
                            if let Some(ref progress) = self.progress {
                                progress.add_embeddings_total(embedding_pairs.len());
                            }
                            if embed_tx.send(embedding_pairs).await.is_err() {
                                // Consumer dropped rx — it errored. Stop.
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        // Bug #5: record the failing path + reason so callers
                        // can surface the failure without scraping logs.
                        let reason = e.to_string();
                        tracing::error!(path = %item.relative, error = %reason, "failed to ingest file");
                        stats.files_failed += 1;
                        stats.failed_files.push((item.relative.clone(), reason));
                    }
                }

                if let Some(ref progress) = self.progress {
                    progress.increment_done();
                }
            }
            drop(embed_tx);

            if external_ct.is_some_and(CancellationToken::is_cancelled)
                || internal_ct.is_cancelled()
            {
                cancelled = true;
            }
            cancelled
        };

        let dual = self
            .dual_embedder
            .as_ref()
            .zip(self.full_dim_storage.as_ref());
        let progress_ref = self.progress.as_ref();
        let internal_ct_for_consumer = internal_ct.clone();
        let consumer = async move {
            let result = if let Some((dual_emb, full_storage)) = dual {
                Self::run_embedding_consumer_dual(
                    embed_rx,
                    dual_emb.as_ref(),
                    index,
                    full_storage,
                    progress_ref,
                )
                .await
            } else {
                Self::run_embedding_consumer(embed_rx, embedder, index, progress_ref).await
            };
            // Bug #4: trip the shared cancel so the producer exits instead of
            // continuing to persist files that will immediately be rolled back.
            if result.is_err() {
                internal_ct_for_consumer.cancel();
            }
            result
        };

        let (was_cancelled, embed_result) = futures::join!(producer, consumer);

        // If the embedding side failed, roll back every document we persisted
        // so SQLite never has sections/claims/symbols without matching vectors.
        if let Err(ref err) = embed_result {
            let docs_to_rollback = indexed_doc_ids
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            tracing::error!(
                docs = docs_to_rollback.len(),
                error = %err,
                "embedding failed — rolling back partially-indexed documents",
            );
            for doc_id in &docs_to_rollback {
                if let Err(e) =
                    super::embedding::delete_document_vectors(doc_id, storage, index).await
                {
                    warn!(doc_id = %doc_id, error = %e, "rollback: delete vectors failed");
                }
                if let Err(e) = storage.delete_document(doc_id).await {
                    warn!(doc_id = %doc_id, error = %e, "rollback: delete document failed");
                }
                if let Err(e) = storage.delete_file_hash(&doc_id.0).await {
                    warn!(doc_id = %doc_id, error = %e, "rollback: delete file hash failed");
                }
            }
        }

        let embed_count = embed_result?;

        if let Some(ref progress) = self.progress {
            progress.set_phase(IngestionPhase::Finalizing);
            progress.set_current_file("");
        }

        Ok((was_cancelled, embed_count, all_pending_refs))
    }

    /// Consume embedding pairs from the producer channel, batch them, and insert.
    async fn run_embedding_consumer<E, I>(
        mut embed_rx: tokio::sync::mpsc::Receiver<Vec<(VectorId, String)>>,
        embedder: &E,
        index: &I,
        progress: Option<&Arc<IngestionProgress>>,
    ) -> Result<usize, IngestionError>
    where
        E: Embedder + ?Sized,
        I: VectorIndex + ?Sized,
    {
        let mut total_embeddings = 0usize;
        let mut buffer: Vec<(VectorId, String)> = Vec::new();
        // Track whether we've signalled the `Embedding` phase yet — flip
        // it on the FIRST batch received so SSE consumers see the phase
        // change at the right moment. Before the first batch arrives,
        // the producer is still parsing; flipping any earlier would
        // misreport the work.
        let mut phase_flipped = false;

        while let Some(pairs) = embed_rx.recv().await {
            if !phase_flipped && let Some(p) = progress {
                p.set_phase(IngestionPhase::Embedding);
                phase_flipped = true;
            }
            buffer.extend(pairs);
            if buffer.len() >= EMBED_FLUSH_THRESHOLD {
                let count = batch_embed_and_insert(&buffer, embedder, index).await?;
                total_embeddings += count;
                if let Some(p) = progress {
                    p.add_embeddings_done(count);
                }
                buffer.clear();
            }
        }
        if !buffer.is_empty() {
            let count = batch_embed_and_insert(&buffer, embedder, index).await?;
            total_embeddings += count;
            if let Some(p) = progress {
                p.add_embeddings_done(count);
            }
        }
        Ok(total_embeddings)
    }

    /// Consume embedding pairs using a [`DualEmbedder`], storing both truncated
    /// vectors in HNSW and full-dimension vectors in SQLite.
    async fn run_embedding_consumer_dual<I>(
        mut embed_rx: tokio::sync::mpsc::Receiver<Vec<(VectorId, String)>>,
        dual_embedder: &dyn crate::embedding::DualEmbedder,
        index: &I,
        full_dim_storage: &crate::storage::SqliteStorage,
        progress: Option<&Arc<IngestionProgress>>,
    ) -> Result<usize, IngestionError>
    where
        I: VectorIndex + ?Sized,
    {
        let mut total_embeddings = 0usize;
        let mut buffer: Vec<(VectorId, String)> = Vec::new();
        // Mirror of the single-embedder path — flip phase on the
        // first batch so SSE consumers see `Embedding` at the
        // right moment.
        let mut phase_flipped = false;

        while let Some(pairs) = embed_rx.recv().await {
            if !phase_flipped && let Some(p) = progress {
                p.set_phase(IngestionPhase::Embedding);
                phase_flipped = true;
            }
            buffer.extend(pairs);
            if buffer.len() >= EMBED_FLUSH_THRESHOLD {
                let count = super::embedding::batch_embed_and_insert_dual(
                    &buffer,
                    dual_embedder,
                    index,
                    full_dim_storage,
                )
                .await?;
                total_embeddings += count;
                if let Some(p) = progress {
                    p.add_embeddings_done(count);
                }
                buffer.clear();
            }
        }
        if !buffer.is_empty() {
            let count = super::embedding::batch_embed_and_insert_dual(
                &buffer,
                dual_embedder,
                index,
                full_dim_storage,
            )
            .await?;
            total_embeddings += count;
            if let Some(p) = progress {
                p.add_embeddings_done(count);
            }
        }
        Ok(total_embeddings)
    }

    /// Resolve pending cross-references and store bridge links.
    ///
    /// Bridge data is rebuilt from **all** code files in `all_files`, not
    /// from the per-file-hash incremental batch. The incremental batch
    /// misses files that were fast-skipped by content hash, and extractor
    /// logic changes (new language support, new rules) won't propagate
    /// unless every file is re-extracted. The full rebuild is cheap:
    /// tree-sitter parse + short walks, typically a few hundred ms on a
    /// 200-file corpus.
    async fn finalize_ingestion<S: Storage + ?Sized>(
        &self,
        pending_refs: &[PendingRef],
        all_files: &[(PathBuf, String)],
        bridge_linker: Option<&BridgeLinker>,
        active_graph: Option<&PackageGraph>,
        storage: &S,
    ) {
        let (_resolved, still_pending) =
            resolve_pending_refs(pending_refs, storage, active_graph).await;
        persist_pending_refs(&still_pending, storage).await;

        if let Some(linker) = bridge_linker {
            // Wipe stale rows so retired extractor outputs don't linger.
            // A failure to clear is logged but not fatal — store_bridge_links
            // will still write fresh rows; worst case we have duplicates
            // that get resolved on the next full pass.
            if let Err(e) = storage.clear_bridge_data().await {
                warn!(error = %e, "failed to clear stale bridge data before rebuild");
            }
            let endpoints = rebuild_bridge_endpoints(all_files, linker).await;
            store_bridge_links(&endpoints, Some(linker), storage).await;
        }

        // Opt-in occurrence index (F-CodeExplorer v2) — default off; cheap
        // early return unless MINISTR_INDEX_OCCURRENCES is set.
        super::occurrences::rebuild_occurrences(all_files, storage).await;
    }

    // ── Shared file processing (used by rooted + multi-path) ─────────────

    /// Parse, enrich, and store a single file. Embedding is deferred.
    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self, storage, index, package_graph), fields(path = %relative_path))]
    async fn parse_and_store_file<S, I>(
        &self,
        file_path: &Path,
        relative_path: &str,
        root_path: Option<&Path>,
        storage: &S,
        index: &I,
        package_graph: Option<&PackageGraph>,
    ) -> Result<FileResult, IngestionError>
    where
        S: Storage + ?Sized,
        I: VectorIndex + ?Sized,
    {
        if is_in_ignored_dir(root_path, file_path) {
            debug!(path = %relative_path, "skipped: file is inside an always-ignored directory");
            return Ok(FileResult::Skipped);
        }

        let file_mtime_ns = file_mtime_nanos(file_path).await;

        let existing_hash = storage
            .get_file_hash(relative_path)
            .await
            .map_err(IngestionError::from)?;

        // Both skip paths below require the cached record to have been
        // produced by the CURRENT extractor AND resolver versions. When
        // either logic changes (bumping `EXTRACTOR_VERSION` or
        // `RESOLVER_VERSION`), the stored row compares < current, so we
        // fall through and re-parse — the index auto-heals without a
        // manual corpus wipe. Resolver-stale + extractor-fresh files
        // could in principle skip re-parse and re-resolve in place; for
        // bulk auto-heal of an already-indexed corpus that path lives
        // in `re_resolve_stale_files` and runs on daemon startup. Here
        // in the per-file ingest path we conservatively re-parse —
        // tree-sitter is cheap and this keeps the file-watcher
        // semantics simple.
        let extractor_fresh = existing_hash
            .as_ref()
            .is_some_and(|e| e.extractor_version >= super::EXTRACTOR_VERSION);
        let resolver_fresh = existing_hash
            .as_ref()
            .is_some_and(|e| e.resolver_version >= super::RESOLVER_VERSION);
        let cache_fresh = extractor_fresh && resolver_fresh;

        if cache_fresh
            && let Some(ref existing) = existing_hash
            && let (Some(stored_mtime), Some(current_mtime)) = (existing.mtime_ns, file_mtime_ns)
            && stored_mtime == current_mtime
        {
            return Ok(FileResult::Skipped);
        }

        let content = tokio::fs::read(file_path)
            .await
            .map_err(|e| IngestionError::Io {
                path: file_path.to_path_buf(),
                source: e,
            })?;

        let content_str = String::from_utf8(content).map_err(|_| IngestionError::Encoding {
            path: file_path.to_path_buf(),
        })?;

        let hash = compute_content_hash(&content_str);

        if cache_fresh
            && let Some(ref existing) = existing_hash
            && existing.content_hash == hash
        {
            storage
                .upsert_file_hash(&FileHashRecord {
                    path: relative_path.to_string(),
                    content_hash: hash,
                    mtime_ns: file_mtime_ns,
                    extractor_version: super::EXTRACTOR_VERSION,
                    resolver_version: super::RESOLVER_VERSION,
                })
                .await
                .map_err(IngestionError::from)?;
            return Ok(FileResult::Skipped);
        }

        let parser = self.parser_for(Path::new(relative_path));
        let mut doc = parser.parse(Path::new(relative_path), &content_str)?;

        let result = store_enriched_document(
            &mut doc,
            relative_path,
            storage,
            &self.claim_extractor,
            &self.summary_generator,
            &self.relationship_detector,
            self.min_section_tokens,
            existing_hash.is_some(),
            Some(index),
            ProcessOptions {
                hash_path: Some(relative_path),
                content_hash: Some(hash),
                mtime_ns: file_mtime_ns,
            },
        )
        .await?;

        // Collect document embedding pairs (deferred for batch embedding)
        let mut embedding_pairs: Vec<(VectorId, String)> = Vec::new();
        collect_document_embeddings(&doc, &mut embedding_pairs);

        // For code files: extract symbols and collect symbol embedding pairs
        let parser_kind = self
            .parser_override
            .or_else(|| detect_parser_kind(Path::new(relative_path)));
        let mut pending_refs = Vec::new();
        if parser_kind == Some(ParserKind::Code) {
            let sym_result =
                extract_code_symbols(relative_path, &content_str, storage, package_graph).await?;
            pending_refs = sym_result.pending_refs;
            embedding_pairs.extend(sym_result.embedding_pairs);
        }

        Ok(FileResult::Indexed {
            sections: result.section_count,
            claims: result.claim_count,
            pending_refs,
            embedding_pairs,
        })
    }

    // ── Re-resolve dependency refs ───────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, storage, dependency_graph, corpus_roots))]
    pub async fn re_resolve_dependency_refs<S: Storage + ?Sized>(
        &self,
        dependency_graph: &PackageGraph,
        dependency_dirs: &[String],
        corpus_roots: &[PathBuf],
        storage: &S,
    ) -> Result<usize, IngestionError> {
        use crate::code::refs::extract_refs;

        let mut combined_graph = self.package_graph.clone().unwrap_or_default();
        for pkg in dependency_graph.packages() {
            combined_graph.add_package(pkg.clone());
        }

        let documents = storage
            .list_documents()
            .await
            .map_err(IngestionError::from)?;

        let code_extensions = [
            "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "c", "cpp", "h",
        ];

        let local_code_docs: Vec<_> = documents
            .iter()
            .filter(|doc| {
                let is_code = code_extensions
                    .iter()
                    .any(|ext| doc.source_path.ends_with(&format!(".{ext}")));
                let is_in_dependency = dependency_dirs
                    .iter()
                    .any(|dep_dir| doc.source_path.starts_with(dep_dir));
                is_code && !is_in_dependency
            })
            .collect();

        if local_code_docs.is_empty() {
            debug!("no local code files to re-resolve");
            return Ok(0);
        }

        info!(
            files = local_code_docs.len(),
            "re-resolving references against cloned dependency"
        );

        let mut total_refs = 0;

        for doc in &local_code_docs {
            let source_path = &doc.source_path;

            let filter = SymbolFilter {
                file_path: Some(source_path.clone()),
                ..SymbolFilter::default()
            };
            let Ok(local_symbols) = storage.list_symbols(&filter).await else {
                continue;
            };
            if local_symbols.is_empty() {
                continue;
            }

            let content = {
                let mut found = None;
                for root in corpus_roots {
                    let full_path = root.join(source_path);
                    if let Ok(bytes) = tokio::fs::read(&full_path).await {
                        found = Some(bytes);
                        break;
                    }
                }
                if found.is_none()
                    && let Ok(bytes) = tokio::fs::read(source_path).await
                {
                    found = Some(bytes);
                }
                let Some(content) = found else {
                    continue;
                };
                content
            };
            let Ok(content_str) = std::str::from_utf8(&content) else {
                continue;
            };

            let Ok(mut ast_parser) = AstParser::try_new() else {
                continue;
            };
            let Ok(tree) = ast_parser.parse(content.as_slice()) else {
                continue;
            };

            let language = Path::new(source_path)
                .extension()
                .and_then(|e| e.to_str())
                .and_then(|ext| {
                    crate::code::GrammarRegistry::global().language_name_for_extension(ext)
                })
                .unwrap_or("rust");

            let raw_refs = extract_refs(&tree, content_str.as_bytes(), language);
            if raw_refs.is_empty() {
                continue;
            }

            // Delete existing refs and re-resolve with the combined graph
            let _ = storage.delete_refs_for_file(source_path).await;

            let file_anchor = local_symbols
                .iter()
                .find(|s| s.kind == "mod")
                .or_else(|| {
                    local_symbols.iter().find(|s| {
                        matches!(
                            s.kind.as_str(),
                            "struct" | "enum" | "trait" | "function" | "type"
                        )
                    })
                })
                .or(local_symbols.first())
                .map(|s| s.id.clone());

            let local_id_set: std::collections::HashSet<_> =
                local_symbols.iter().map(|s| &s.id).collect();

            let mut inserted = 0;
            for raw in &raw_refs {
                let from_id = match &raw.from_context {
                    Some(type_name) => local_symbols
                        .iter()
                        .find(|s| {
                            s.name == *type_name
                                && (s.kind == "struct" || s.kind == "enum" || s.kind == "type")
                        })
                        .map(|s| s.id.clone()),
                    None => file_anchor.clone(),
                };

                let Some(from_id) = from_id else { continue };
                if !local_id_set.contains(&from_id) {
                    continue;
                }

                let target_filter = SymbolFilter {
                    name_exact: Some(raw.target_name.clone()),
                    ..SymbolFilter::default()
                };
                let Ok(matches) = storage.list_symbols(&target_filter).await else {
                    continue;
                };
                let primary: Vec<_> = matches
                    .iter()
                    .filter(|s| {
                        matches!(
                            s.kind.as_str(),
                            "struct"
                                | "enum"
                                | "trait"
                                | "function"
                                | "type"
                                | "const"
                                | "static"
                                | "mod"
                        )
                    })
                    .collect();

                let target = match primary.len() {
                    0 => continue,
                    1 => primary[0],
                    _ => {
                        let crate_filtered: Vec<_> = if let Some(tc) = &raw.target_crate {
                            if let Some(dir_prefix) = combined_graph.dir_prefix_for_crate(tc) {
                                primary
                                    .iter()
                                    .filter(|s| s.file_path.starts_with(dir_prefix))
                                    .copied()
                                    .collect()
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        };
                        if crate_filtered.len() == 1 {
                            crate_filtered[0]
                        } else if !crate_filtered.is_empty() {
                            crate_filtered
                                .iter()
                                .find(|s| s.file_path != *source_path)
                                .copied()
                                .unwrap_or(crate_filtered[0])
                        } else {
                            primary
                                .iter()
                                .find(|s| s.file_path != *source_path)
                                .copied()
                                .unwrap_or(primary[0])
                        }
                    }
                };

                if from_id == target.id {
                    continue;
                }

                let record = crate::storage::traits::SymbolRefRecord {
                    from_symbol_id: from_id,
                    to_symbol_id: target.id.clone(),
                    ref_kind: raw.kind,
                };
                if storage
                    .insert_symbol_refs(std::slice::from_ref(&record))
                    .await
                    .is_ok()
                {
                    inserted += 1;
                }
            }

            if inserted > 0 {
                debug!(
                    path = %source_path,
                    refs = inserted,
                    "re-resolved dependency references"
                );
            }
            total_refs += inserted;
        }

        if total_refs > 0 {
            info!(
                refs = total_refs,
                "dependency reference re-resolution complete"
            );
        }

        Ok(total_refs)
    }

    // ── Resolver-version auto-heal ───────────────────────────────────────

    /// Re-resolve `symbol_refs` for every file whose stored
    /// `resolver_version` is below the current
    /// [`super::RESOLVER_VERSION`].
    ///
    /// The auto-heal counterpart to [`Self::re_resolve_dependency_refs`]:
    /// where that method re-resolves when a new dependency tree becomes
    /// available, this one re-resolves when the *resolver code itself*
    /// has been upgraded. Reads stored symbols (no re-extraction),
    /// re-parses each stale file (tree-sitter is cheap), replays
    /// [`resolve_and_store_refs`] which deletes the old `symbol_refs`
    /// rows for the file and writes new ones. Embeddings, documents,
    /// sections, claims are not touched — the resolver step is
    /// orthogonal to all of those.
    ///
    /// Returns the count of files successfully re-resolved.
    ///
    /// # Errors
    ///
    /// Returns [`IngestionError`] when storage lookups fail. Individual
    /// per-file failures (missing source, unparseable content, no
    /// symbols) are logged at debug and silently skipped — the auto-heal
    /// must be best-effort so one broken file can't block the rest.
    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, storage, corpus_roots))]
    pub async fn re_resolve_stale_files<S: Storage + ?Sized>(
        &self,
        corpus_roots: &[PathBuf],
        storage: &S,
    ) -> Result<usize, IngestionError> {
        let all_hashes = storage
            .list_file_hashes()
            .await
            .map_err(IngestionError::from)?;

        let stale: Vec<FileHashRecord> = all_hashes
            .into_iter()
            .filter(|h| h.resolver_version < super::RESOLVER_VERSION)
            .collect();

        if stale.is_empty() {
            return Ok(0);
        }

        info!(
            stale_count = stale.len(),
            current_resolver_version = super::RESOLVER_VERSION,
            "resolver auto-heal: re-resolving stale files"
        );

        let mut healed = 0usize;

        for hash in &stale {
            let source_path = &hash.path;

            // Skip non-code files. The resolver only touches symbol-bearing
            // languages; markdown / json / etc. file_hash rows just need
            // their stamp bumped so we don't retry every startup.
            let ext = Path::new(source_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let language = crate::code::GrammarRegistry::global().language_name_for_extension(ext);

            // Load stored symbols.
            let filter = SymbolFilter {
                file_path: Some(source_path.clone()),
                ..SymbolFilter::default()
            };
            let Ok(local_symbols) = storage.list_symbols(&filter).await else {
                continue;
            };

            if local_symbols.is_empty() || language.is_none() {
                // Nothing to resolve here — just stamp the row so we
                // don't reconsider it on every restart.
                let _ = storage
                    .upsert_file_hash(&FileHashRecord {
                        resolver_version: super::RESOLVER_VERSION,
                        ..hash.clone()
                    })
                    .await;
                healed += 1;
                continue;
            }

            // Read source content. Try corpus roots first, then absolute.
            let content_bytes = {
                let mut found: Option<Vec<u8>> = None;
                for root in corpus_roots {
                    let full_path = root.join(source_path);
                    if let Ok(bytes) = tokio::fs::read(&full_path).await {
                        found = Some(bytes);
                        break;
                    }
                }
                if found.is_none()
                    && let Ok(bytes) = tokio::fs::read(source_path).await
                {
                    found = Some(bytes);
                }
                let Some(b) = found else {
                    debug!(
                        path = %source_path,
                        "resolver auto-heal: source file missing, skipping"
                    );
                    continue;
                };
                b
            };

            // Parse with the same language-dispatch logic
            // `extract_code_symbols` uses: Rust path via the dedicated
            // parser, other languages via the grammar registry.
            let is_rust = language == Some("rust");
            let tree = if is_rust {
                let Ok(mut ast_parser) = AstParser::try_new() else {
                    continue;
                };
                match ast_parser.parse(content_bytes.as_slice()) {
                    Ok(t) => t,
                    Err(_) => continue,
                }
            } else {
                let Some(ts_lang) =
                    crate::code::GrammarRegistry::global().language_for_extension(ext)
                else {
                    continue;
                };
                let Ok(mut ast_parser) = AstParser::with_language(ts_lang) else {
                    continue;
                };
                match ast_parser.parse(content_bytes.as_slice()) {
                    Ok(t) => t,
                    Err(_) => continue,
                }
            };

            // Replay the resolver — deletes existing refs for this file
            // as its first step, then writes the new edges using the
            // *current* resolver semantics (line-range from_context,
            // same-crate disambiguation, expanded stdlib denylist via
            // `extract_refs`).
            let language_str = language.unwrap_or("rust");
            let _ = resolve_and_store_refs(
                &tree,
                content_bytes.as_slice(),
                source_path,
                language_str,
                &local_symbols,
                storage,
                self.package_graph.as_ref(),
            )
            .await;

            // Stamp the file's resolver_version so next startup skips it.
            let _ = storage
                .upsert_file_hash(&FileHashRecord {
                    resolver_version: super::RESOLVER_VERSION,
                    ..hash.clone()
                })
                .await;

            healed += 1;
        }

        info!(healed, "resolver auto-heal complete");
        Ok(healed)
    }
}

#[cfg(test)]
mod phase5_chunk2_persist_gate_tests {
    //! PHASE5 chunk 2 — regression coverage for the empty-index persist
    //! gate added to `run_producer_consumer`. Driving the full pipeline
    //! here would require building Storage + Embedder + FileItems and a
    //! cancellation token harness; PHASE4 chunk 4 explicitly noted that
    //! the persist hook is exercised by the broader test suite + the
    //! operator smoke. This module instead pins the two facts the gate
    //! relies on:
    //!
    //! 1. A freshly-constructed [`HnswIndex`](crate::index::HnswIndex)
    //!    reports `is_empty() == true`.
    //! 2. Calling `persist()` on that empty index is the failure mode
    //!    we're avoiding — either an `Err` (current behaviour at the
    //!    time of writing) OR a no-op that still produces a misleading
    //!    snapshot. Either way the gate is the right place to short-
    //!    circuit.
    //!
    //! If `HnswIndex` ever starts returning `Ok(())` for empty persist,
    //! the second test still passes (it asserts `is_empty()` matches
    //! the persist outcome) and the gate is unchanged — the gate is
    //! the contract, not the test.

    use crate::index::{HnswIndex, VectorIndex};
    use tempfile::tempdir;

    #[test]
    fn fresh_hnsw_is_empty() {
        // Producer-side regression: a brand-new index reports empty,
        // which is exactly the state the parser-side counter races into
        // before the embedder catches up.
        let idx = HnswIndex::new(384, 1024).expect("create hnsw");
        assert!(
            idx.is_empty(),
            "fresh HNSW must report is_empty=true (this is what the gate reads)",
        );
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn empty_persist_is_the_failure_we_avoid() {
        // PHASE4 chunk 4 hit this in production: index.persist() on an
        // empty HNSW returns an error like "nb point 0", drowning the
        // demo log in WARNs until the embedder catches up. The gate's
        // job is to ensure persist() is never called in this state.
        let dir = tempdir().expect("tempdir");
        let idx = HnswIndex::new(384, 1024).expect("create hnsw");
        let result = idx.persist(dir.path());
        // Either branch is acceptable behaviour from HnswIndex; the
        // important thing is that the gate short-circuits BEFORE we
        // reach this code path at all.
        if result.is_ok() {
            // If the underlying impl later starts accepting empty
            // persists, the gate is still desirable (no point writing a
            // useless snapshot every persist_every files).
            return;
        }
        // Confirm the failure mode we're avoiding is in fact tied to
        // emptiness — once we add a vector, the same persist succeeds.
        idx.insert("sec-1", &[0.1_f32; 384])
            .expect("insert one vector");
        idx.persist(dir.path())
            .expect("persist with at least one vector must succeed");
    }
}
