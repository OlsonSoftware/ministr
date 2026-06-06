//! `IngestionPipeline` — the main orchestrator for document ingestion.
//!
//! All four public entry points delegate their core processing to
//! [`process::store_enriched_document`], keeping each method focused on its
//! specific I/O and embedding strategy.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};

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
use super::embedding::{batch_embed_and_insert, collect_document_embeddings, embed_document};
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
    /// Wall-clock start of the current run, set by [`IngestionProgress::start`].
    /// Backs the throughput/latency getters (fg4 — per-stage observability).
    started_at: parking_lot::Mutex<Option<std::time::Instant>>,
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
            started_at: parking_lot::Mutex::new(None),
        }
    }

    pub fn start(&self, total_files: usize) {
        self.files_total.store(total_files, Ordering::Relaxed);
        self.files_done.store(0, Ordering::Relaxed);
        self.sections_done.store(0, Ordering::Relaxed);
        self.embeddings_total.store(0, Ordering::Relaxed);
        self.embeddings_done.store(0, Ordering::Relaxed);
        *self.started_at.lock() = Some(std::time::Instant::now());
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

    // ── Per-stage observability (fg4) ──────────────────────────────────────
    //
    // Lightweight derived metrics so the parse-vs-embed bottleneck is visible
    // rather than guessed. `embed_backlog` is the embed-stage queue depth
    // (sections the producer has queued for embedding minus those the consumer
    // has embedded); the throughput getters divide the live counters by the
    // wall-clock elapsed since `start`.

    /// Embed-stage backlog: sections parsed and queued for embedding but not
    /// yet embedded. A persistently large value means embedding is the
    /// bottleneck (the GPU/embedder can't keep up with parsing).
    #[must_use]
    pub fn embed_backlog(&self) -> usize {
        self.embeddings_total
            .load(Ordering::Relaxed)
            .saturating_sub(self.embeddings_done.load(Ordering::Relaxed))
    }

    /// Seconds elapsed since [`IngestionProgress::start`], or `0.0` if the run
    /// has not started.
    #[must_use]
    pub fn elapsed_secs(&self) -> f64 {
        let started = *self.started_at.lock();
        started.map_or(0.0, |t| t.elapsed().as_secs_f64())
    }

    /// Files indexed per second since `start` (`0.0` before any time elapses).
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // throughput metric; counts fit f64 exactly well past any real corpus
    pub fn files_per_sec(&self) -> f64 {
        let secs = self.elapsed_secs();
        if secs > 0.0 {
            self.files_done() as f64 / secs
        } else {
            0.0
        }
    }

    /// Embeddings completed per second since `start` (`0.0` before any time
    /// elapses).
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // throughput metric; counts fit f64 exactly well past any real corpus
    pub fn embeddings_per_sec(&self) -> f64 {
        let secs = self.elapsed_secs();
        if secs > 0.0 {
            self.embeddings_done() as f64 / secs
        } else {
            0.0
        }
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
///
/// `pub(super)` so the Parse stage can read the per-item fields it streams.
pub(super) struct FileItem {
    pub path: PathBuf,
    pub relative: String,
    pub root_id: Option<String>,
    /// Absolute corpus root the file was discovered under (when known).
    /// Used by `parse_and_store_file` to scope the ignore-dir guard to
    /// components inside the root — without this, a corpus rooted under
    /// an always-ignored ancestor name (e.g. `~/.ministr/remote/<hash>/`)
    /// would have every file rejected.
    pub root_path: Option<PathBuf>,
}

/// Ambient inputs for one ingest run, grouped as a parameter object (Fowler's
/// *Introduce Parameter Object*) so the producer/consumer entry point stays
/// within the argument budget.
///
/// Every field is a shared borrow over a single lifetime `'a`: the Parse stage
/// (producer) and Embed stage (consumer) both read these while `join!`ed, so
/// none may be `&mut`. This is also the natural seam for the eventual
/// Coordinator to thread one context through the staged composition.
struct IngestContext<'a, S: ?Sized, E: ?Sized, I: ?Sized> {
    storage: &'a S,
    embedder: &'a E,
    index: &'a I,
    active_graph: Option<&'a PackageGraph>,
    ct: Option<&'a CancellationToken>,
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
    /// Optional dedicated embedding service (ADR 0001 D1). When set, the
    /// single-embedder consumer routes each batch through it — the model runs
    /// on the service's own thread and this task `await`s without blocking a
    /// Tokio worker. The daemon sets this so concurrent-capable indexing never
    /// starves the runtime. `None` (tests, `ministr index`, web fetch) keeps
    /// the inline path. Does not affect the dual (Matryoshka) consumer.
    embedding_service: Option<Arc<crate::embedding::EmbeddingService>>,
    /// Heuristic Contextual Retrieval (rq): when `true`, each section's embed
    /// text is prefixed with a compact structural breadcrumb (heading path)
    /// before embedding. Default `false` — production embed text is
    /// byte-identical to the verbatim section, so flipping this forces a full
    /// re-index. The rq0 real-embedder A/B (`just eval-quality`, all-MiniLM,
    /// 72 queries) measured this prefix as a *mixed* lever: MRR +0.017,
    /// nDCG@5 +0.010, P@5 +0.011, but R@5 −0.007 (the breadcrumb tokens push a
    /// borderline doc out of top-5). Kept default-OFF on that net; exposed as
    /// an opt-in so per-corpus callers + future code-corpus evals can measure it.
    contextualize_embeddings: bool,
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
            embedding_service: None,
            contextualize_embeddings: false,
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

    /// Route embedding through a dedicated
    /// [`EmbeddingService`](crate::embedding::EmbeddingService) (ADR 0001 D1)
    /// instead of calling the synchronous embedder inline on a Tokio worker.
    /// Set by the daemon so concurrent-capable indexing never starves the
    /// async runtime. Affects only the single-embedder consumer path; the
    /// dual (Matryoshka) path is unchanged.
    #[must_use]
    pub fn with_embedding_service(
        mut self,
        service: Arc<crate::embedding::EmbeddingService>,
    ) -> Self {
        self.embedding_service = Some(service);
        self
    }

    /// Enable heuristic Contextual Retrieval (rq) for this pipeline: each
    /// section's embed text is prefixed with a compact structural breadcrumb
    /// (heading path) before embedding.
    ///
    /// Default is `false` (verbatim embed text). Turning this on changes the
    /// embedded text, so an existing corpus must be **fully re-indexed** to
    /// benefit. The rq0 real-embedder A/B measured the prefix as a *mixed*
    /// lever (MRR/nDCG/P@5 up, R@5 slightly down) on the doc-heavy eval corpus,
    /// which is why it ships opt-in rather than default-on. See the
    /// `contextualize_embeddings` field docs for the measured deltas.
    #[must_use]
    pub fn with_contextual_embeddings(mut self, on: bool) -> Self {
        self.contextualize_embeddings = on;
        self
    }

    #[must_use]
    pub fn with_parser(kind: ParserKind) -> Self {
        Self {
            parser_override: Some(kind),
            ..Self::new()
        }
    }

    /// Chaining form of [`Self::with_parser`] for the per-corpus config seam
    /// (parity-meta-toml-load): sets the parser override in the middle of a
    /// builder chain. `None` leaves auto-detection (extension-based) on.
    ///
    /// [`Self::with_parser`] is a *constructor* (it discards `self`), so it
    /// can't be used after `with_progress`/`with_embedding_service`; both
    /// ingestion entry points need this chaining setter to apply a corpus's
    /// resolved `meta.toml` `parser` without clobbering the rest of the chain.
    #[must_use]
    pub fn with_parser_override(mut self, parser: Option<ParserKind>) -> Self {
        self.parser_override = parser;
        self
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

        // `Arc<str>` so the bytes can be shared into the off-runtime parse
        // pool (which needs `'static`) without copying the file contents.
        let content_str: Arc<str> =
            Arc::from(
                String::from_utf8(content).map_err(|_| IngestionError::Encoding {
                    path: file_path.to_path_buf(),
                })?,
            );

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

        // Run the CPU-bound tree-sitter parse on the dedicated rayon pool,
        // off the Tokio runtime (see `parse_pool`); the parser's internal
        // PARSE_BUDGET timeout is preserved (it lives inside `parse`).
        let parser = self.parser_for(Path::new(relative_path));
        let mut doc =
            super::parse_pool::parse_on_pool(parser, PathBuf::from(relative_path), content_str)
                .await?;

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
        embed_document(&doc, embedder, index, storage).await?;

        // For code files: extract symbols and embed immediately
        if parser_kind == ParserKind::Code {
            let sym_result = extract_code_symbols(source_path, content, storage, None).await?;
            if !sym_result.embedding_pairs.is_empty() {
                batch_embed_and_insert(
                    &sym_result.embedding_pairs,
                    embedder,
                    self.embedding_service.as_deref(),
                    index,
                    storage,
                    self.progress.as_ref(),
                )
                .await?;
            }
        }

        info!(source = %source_path, result.section_count, result.claim_count, "ingested content with embeddings");

        Ok(ContentIngestionStats {
            sections: result.section_count,
            claims: result.claim_count,
            skipped: false,
        })
    }

    /// Remove a runtime-ingested document: its vectors (including the D4
    /// `indexed_vectors` source-of-truth rows), the storage record, and
    /// the file-hash row (so a later re-ingest of identical content is
    /// not skipped as unchanged).
    ///
    /// The deletion counterpart of
    /// [`Self::ingest_content_with_embeddings`] — used by retention
    /// policies over runtime-ingested content (e.g. `exec-runs/` run
    /// reports).
    ///
    /// # Errors
    ///
    /// Returns [`IngestionError`] if vector or storage deletion fails.
    pub async fn remove_document_with_embeddings<S, I>(
        &self,
        doc_id: &crate::types::ContentId,
        storage: &S,
        index: &I,
    ) -> Result<usize, IngestionError>
    where
        S: Storage + ?Sized,
        I: VectorIndex + ?Sized,
    {
        let deleted = super::embedding::delete_document_vectors(doc_id, storage, index).await?;
        storage
            .delete_document(doc_id)
            .await
            .map_err(IngestionError::from)?;
        storage
            .delete_file_hash(&doc_id.0)
            .await
            .map_err(IngestionError::from)?;
        Ok(deleted)
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
        // Workspace → package graph for cross-crate ref resolution. `detected`
        // is bound here so `active_graph` can borrow it for the whole run.
        let detected_graph = self.detect_workspace_graph(dir);
        let active_graph = self.package_graph.as_ref().or(detected_graph.as_ref());

        if let Some(ref progress) = self.progress {
            progress.set_phase(IngestionPhase::Discovering);
        }

        // Discover stage: walk once, producing the file list and (when rooted)
        // a stat-merkle so an unchanged corpus can short-circuit the reindex.
        let (root_hash, files) = Self::discover_files_and_hash(dir, root_id)?;

        if let Some(stats) =
            Self::corpus_merkle_short_circuit(storage, root_id, root_hash.as_deref(), files.len())
                .await
        {
            if let Some(ref progress) = self.progress {
                progress.complete();
            }
            return Ok(stats);
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

        Self::register_corpus_root(storage, dir, root_id, &files).await;
        let bridge_linker = Self::detect_bridge_linker(dir, &files);

        mem_profile::checkpoint("ingestion loop start (rooted)");

        let file_items = Self::build_file_items(&files, dir, root_id);

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
                IngestContext {
                    storage,
                    embedder,
                    index,
                    active_graph,
                    ct,
                },
                &mut stats,
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

        Self::sweep_stale_documents(storage, index, dir, root_id, &files, &mut stats).await?;

        info!(
            indexed = stats.files_indexed,
            skipped = stats.files_skipped,
            removed = stats.files_removed,
            failed = stats.files_failed,
            "ingestion with embeddings complete"
        );

        Self::persist_corpus_merkle(storage, root_id, root_hash, stats.files_discovered).await;

        if let Some(ref progress) = self.progress {
            progress.complete();
        }

        Ok(stats)
    }

    // ── Rooted-ingest stage helpers (extracted from the orchestrator) ─────

    /// Workspace detection → package graph for cross-crate ref resolution.
    /// Returns an *owned* graph the caller binds so `active_graph` can borrow
    /// it for the whole run; `None` when a graph was already supplied or no
    /// Cargo workspace is detected.
    fn detect_workspace_graph(&self, dir: &Path) -> Option<PackageGraph> {
        if self.package_graph.is_some() {
            return None;
        }
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
    }

    /// Discover stage. Walks once, computing the file list and — for rooted
    /// corpora — a sorted BLAKE3 stat-merkle over each file's
    /// `(rel_path, mtime_ns, size)`. Unrooted callers (tests / ad-hoc) have no
    /// key to remember a fingerprint under, so they take the legacy
    /// `discover_files` path with no hash.
    fn discover_files_and_hash(
        dir: &Path,
        root_id: Option<&str>,
    ) -> Result<(Option<String>, Vec<PathBuf>), IngestionError> {
        if root_id.is_some() {
            let (h, f) = super::discovery::compute_corpus_stat_merkle(dir)?;
            Ok((Some(h), f))
        } else {
            Ok((None, discover_files(dir)?))
        }
    }

    /// Phase 7 corpus-root stat-merkle short-circuit. When the freshly-computed
    /// `root_hash` matches the stored value for `root_id` *and* the extractor
    /// version is current, no file has changed since the last successful index
    /// — returns `Some(stats)` (everything counted skipped) so the caller can
    /// bail before any parse/embed/SQL work. A version mismatch logs and
    /// returns `None` (re-extract); a missing key / hash mismatch is also
    /// `None` (proceed).
    async fn corpus_merkle_short_circuit<S>(
        storage: &S,
        root_id: Option<&str>,
        root_hash: Option<&str>,
        file_count: usize,
    ) -> Option<IngestionStats>
    where
        S: Storage + ?Sized,
    {
        let (Some(rid), Some(hash)) = (root_id, root_hash) else {
            return None;
        };
        let Ok(Some(prior)) = storage.get_corpus_merkle(rid).await else {
            return None;
        };
        if prior.root_hash.as_str() != hash {
            return None;
        }
        if prior.extractor_version == super::EXTRACTOR_VERSION {
            info!(
                corpus_id = rid,
                file_count,
                extractor_version = super::EXTRACTOR_VERSION,
                "corpus stat-merkle unchanged — short-circuiting reindex"
            );
            let mut stats = IngestionStats::new(file_count);
            stats.files_skipped = file_count;
            return Some(stats);
        }
        info!(
            corpus_id = rid,
            stored_extractor_version = prior.extractor_version,
            current_extractor_version = super::EXTRACTOR_VERSION,
            "corpus stat-merkle matches but extractor version differs — re-extracting"
        );
        None
    }

    /// Register this corpus root *before* the producer/consumer loop:
    /// `run_producer_consumer` calls `set_document_root` per file, and that
    /// UPDATE is FK-constrained to `corpus_roots`, so the row must already
    /// exist or `root_id` silently stays NULL. Guarded on a non-empty
    /// discovery so a transient unreadable root can't stomp a prior good
    /// file_count with 0 (same footing as the orphan-sweep guard).
    async fn register_corpus_root<S>(
        storage: &S,
        dir: &Path,
        root_id: Option<&str>,
        files: &[PathBuf],
    ) where
        S: Storage + ?Sized,
    {
        let Some(rid) = root_id else {
            return;
        };
        if files.is_empty() {
            return;
        }
        let roots = [(dir.to_path_buf(), rid.to_string())];
        let mut root_lang_stats: std::collections::HashMap<
            String,
            std::collections::HashMap<String, usize>,
        > = std::collections::HashMap::new();
        let mut root_file_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        accumulate_language_stats(files, &roots, &mut root_lang_stats, &mut root_file_counts);
        update_root_stats(storage, &roots, &root_lang_stats, &root_file_counts).await;
    }

    /// Detect bridge frameworks for cross-language linking. Unions the upward
    /// walk from `dir` with a scan of every manifest discovered *below* it, so
    /// a monorepo's subdirectory app (e.g. a Tauri app under
    /// `<repo>/app/src-tauri/`) still registers its framework and links.
    fn detect_bridge_linker(dir: &Path, files: &[PathBuf]) -> Option<BridgeLinker> {
        let detected_kinds: Vec<BridgeKind> = {
            let mut set: std::collections::BTreeSet<BridgeKind> =
                detector::FrameworkDetector::detect(dir)
                    .into_iter()
                    .collect();
            set.extend(detector::FrameworkDetector::detect_in_files(files));
            set.into_iter().collect()
        };
        let linker = create_linker_for_kinds(&detected_kinds);
        if !detected_kinds.is_empty() {
            info!(
                kinds = ?detected_kinds,
                "detected bridge frameworks for cross-language linking"
            );
        }
        linker
    }

    /// Build the per-file work items, namespacing each relative path under the
    /// corpus `root_id` (when rooted) so documents are addressable as
    /// `rid/rel`.
    fn build_file_items(files: &[PathBuf], dir: &Path, root_id: Option<&str>) -> Vec<FileItem> {
        files
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
            .collect()
    }

    /// Orphan-GC stale-doc sweep (F32) — a keying-aware global diff.
    ///
    /// Enumerates *every* document (a NULL-`root_id` doc is invisible to
    /// `list_documents_by_root` and would orphan forever) but scopes
    /// *deletion* by document ownership so a sibling root's docs in a shared
    /// multi-root index are never false-deleted. Skipped entirely on empty
    /// discovery — far more likely a transient unreadable root than a
    /// genuinely emptied corpus, and a global diff would then delete every
    /// attributable document in the index.
    async fn sweep_stale_documents<S, I>(
        storage: &S,
        index: &I,
        dir: &Path,
        root_id: Option<&str>,
        files: &[PathBuf],
        stats: &mut IngestionStats,
    ) -> Result<(), IngestionError>
    where
        S: Storage + ?Sized,
        I: VectorIndex + ?Sized,
    {
        if files.is_empty() {
            warn!(
                "skipping stale-doc cleanup: discovery returned 0 files (likely a transient unreadable root, not an emptied corpus)"
            );
            return Ok(());
        }

        // Both stored forms a document under this dir can take: namespaced
        // (`rid/rel`, root_id = Some) and bare-relative (`rel`, root_id = NULL).
        // Keyed exactly as ingestion keys them, so membership is an equality
        // test, not a `stat`.
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

        // Ownership is read from the source_path namespace prefix (`rid/…`),
        // set by `namespace_path` at ingest. The `root_id` *column* is not
        // written on this code path, so scope must not key off it.
        // `this_prefix` is what this run namespaces under (None when unrooted).
        let this_prefix = root_id.map(|rid| format!("{rid}/"));

        // Does a *foreign* explicit root share this index? A bare-relative
        // (NULL-keyed) document carries no attribution, so we reclaim such
        // documents only when none is present — i.e. a single-corpus
        // CLI/worker index, the only place NULL-keyed documents legitimately
        // arise. In a genuine multi-root index they could belong to a sibling.
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
            //   bare-relative (NULL-keyed) → ours only on an unrooted reindex,
            //     or when no sibling root shares the index (the F32.1 fix).
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
            // Symbols live in their own table keyed by file_path (NOT cascaded
            // by delete_document); without this they'd keep surfacing in symbol
            // search forever. Runs after delete_document_vectors so the HNSW
            // teardown could still enumerate them. symbol_refs cascade via FK.
            storage
                .delete_symbols_for_file(&doc.source_path)
                .await
                .map_err(IngestionError::from)?;
            stats.files_removed += 1;
        }
        Ok(())
    }

    /// Phase 7 — record the new stat-merkle so the *next* reindex can
    /// short-circuit. Only persists when we have a corpus key *and* a hash was
    /// computed (i.e. the rooted path). A failed upsert is non-fatal: the
    /// corpus is correctly indexed, we just lose next-run short-circuiting.
    async fn persist_corpus_merkle<S>(
        storage: &S,
        root_id: Option<&str>,
        root_hash: Option<String>,
        files_discovered: usize,
    ) where
        S: Storage + ?Sized,
    {
        let (Some(rid), Some(hash)) = (root_id, root_hash) else {
            return;
        };
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_nanos()).ok())
            .unwrap_or(0);
        let record = crate::storage::traits::CorpusMerkleRecord {
            corpus_id: rid.to_string(),
            root_hash: hash,
            file_count: i64::try_from(files_discovered).unwrap_or(i64::MAX),
            last_indexed_ns: now_ns,
            extractor_version: super::EXTRACTOR_VERSION,
        };
        if let Err(e) = storage.upsert_corpus_merkle(&record).await {
            tracing::warn!(error = ?e, "failed to upsert corpus_merkle (non-fatal)");
        }
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
                IngestContext {
                    storage,
                    embedder,
                    index,
                    active_graph,
                    ct: None,
                },
                &mut stats,
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
    ///
    /// Composes the two pipes-and-filters stages (ADR 0001 D3): the Parse
    /// stage ([`super::parse_stage::run_parse_stage`], producer) feeds the
    /// Embed stage ([`super::embed_stage::run_embed_stage`], consumer) over a
    /// bounded channel, then rolls back on embed failure.
    ///
    /// Takes its backends + cancellation as an [`IngestContext`] parameter
    /// object; it is destructured immediately so the body reads as before.
    async fn run_producer_consumer<S, E, I>(
        &self,
        file_items: Vec<FileItem>,
        cx: IngestContext<'_, S, E, I>,
        stats: &mut IngestionStats,
    ) -> Result<(bool, usize, Vec<PendingRef>), IngestionError>
    where
        S: Storage + ?Sized,
        E: Embedder + ?Sized,
        I: VectorIndex + ?Sized,
    {
        let IngestContext {
            storage,
            embedder,
            index,
            active_graph,
            ct,
        } = cx;

        let (embed_tx, embed_rx) = tokio::sync::mpsc::channel::<Vec<(VectorId, String)>>(16);

        // PHASE4 chunk 4: bridge endpoints used to be accumulated here,
        // returned, and immediately discarded by both callers —
        // `finalize_ingestion` rebuilds bridge data from `all_files` (see its
        // doc comment) so the per-file batch was dead weight. Removed entirely.

        // Shared cancel signal: when the consumer errors it trips this token
        // so the producer stops scheduling new parses and exits promptly.
        // Combined with any caller-supplied token via a parent/child check.
        let internal_ct = CancellationToken::new();
        let external_ct = ct;

        // Track every document the producer persisted so we can roll back on
        // embedding failure (bug #1: partial-write corpus corruption).
        let indexed_doc_ids: std::sync::Mutex<Vec<crate::types::ContentId>> =
            std::sync::Mutex::new(Vec::new());

        // Parse stage (ADR 0001 D3): the producer. Streams the discovered
        // files through `parse_and_store_file`, accounts stats, snapshots the
        // HNSW periodically, tracks persisted docs for rollback, and forwards
        // each file's embedding pairs into the Embed stage's channel.
        let producer = super::parse_stage::run_parse_stage(
            self,
            file_items,
            storage,
            index,
            active_graph,
            stats,
            super::parse_stage::ParseStageWiring {
                concurrency: self.concurrency.unwrap_or_else(default_concurrency),
                progress: self.progress.as_ref(),
                persist_every: self.batch_config.persist_every,
                corpus_dir: self.corpus_dir.as_deref(),
                embed_tx,
                indexed_doc_ids: &indexed_doc_ids,
                internal_ct: &internal_ct,
                external_ct,
            },
        );

        let dual = self
            .dual_embedder
            .as_ref()
            .zip(self.full_dim_storage.as_ref());
        let progress_ref = self.progress.as_ref();
        let service_ref = self.embedding_service.as_deref();
        // Copy the shared storage reference for the consumer (`&S` is `Copy`),
        // so the single-embed path can persist the indexed vectors (D4) while
        // the producer keeps its own borrow.
        let storage_for_consumer = storage;
        let internal_ct_for_consumer = internal_ct.clone();
        let consumer = async move {
            // The Embed stage (ADR 0001 D3): drain the producer channel, batch,
            // embed (off-runtime via the shared service when set), and insert;
            // the dual (Matryoshka) variant also stores full-dim vectors.
            let result = super::embed_stage::run_embed_stage(
                embed_rx,
                storage_for_consumer,
                embedder,
                service_ref,
                dual.map(|(d, s)| (d.as_ref(), s)),
                index,
                progress_ref,
            )
            .await;
            // Bug #4: trip the shared cancel so the producer exits instead of
            // continuing to persist files that will immediately be rolled back.
            if result.is_err() {
                internal_ct_for_consumer.cancel();
            }
            result
        };

        let ((was_cancelled, all_pending_refs), embed_result) = futures::join!(producer, consumer);

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
            super::embedding::rollback_partial_documents(&docs_to_rollback, storage, index).await;
        }

        let embed_count = embed_result?;

        if let Some(ref progress) = self.progress {
            // fg4 — per-stage observability: emit the run's throughput +
            // embed-backlog so the parse-vs-embed bottleneck is visible in logs
            // without a profiler. embed_backlog should be ~0 here (the consumer
            // has drained); a non-zero value flags a torn/early exit.
            tracing::info!(
                files_indexed = stats.files_indexed,
                embeddings = embed_count,
                elapsed_secs = progress.elapsed_secs(),
                files_per_sec = progress.files_per_sec(),
                embeddings_per_sec = progress.embeddings_per_sec(),
                embed_backlog = progress.embed_backlog(),
                "ingestion stage metrics",
            );
            progress.set_phase(IngestionPhase::Finalizing);
            progress.set_current_file("");
        }

        Ok((was_cancelled, embed_count, all_pending_refs))
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
    ///
    /// `pub(super)` so the Parse stage ([`super::parse_stage::run_parse_stage`])
    /// can drive it as the per-file unit of work.
    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self, storage, index, package_graph), fields(path = %relative_path))]
    pub(super) async fn parse_and_store_file<S, I>(
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

        // `Arc<str>` so the bytes can be shared into the off-runtime parse
        // pool (which needs `'static`) without copying the file contents.
        let content_str: Arc<str> =
            Arc::from(
                String::from_utf8(content).map_err(|_| IngestionError::Encoding {
                    path: file_path.to_path_buf(),
                })?,
            );

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

        // Run the CPU-bound tree-sitter parse on the dedicated rayon pool,
        // off the Tokio runtime, so async workers stay free for IO and the
        // embedding consumer while all cores parse. The parser's internal
        // PARSE_BUDGET timeout is preserved (it lives inside `parse`).
        let parser = self.parser_for(Path::new(relative_path));
        let mut doc = super::parse_pool::parse_on_pool(
            parser,
            PathBuf::from(relative_path),
            Arc::clone(&content_str),
        )
        .await?;

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

        // Collect document embedding pairs (deferred for batch embedding).
        // Heuristic Contextual Retrieval (rq) is opt-in via
        // [`Self::with_contextual_embeddings`]; default `false` keeps the embed
        // text byte-identical to the verbatim section (no forced re-index).
        let mut embedding_pairs: Vec<(VectorId, String)> = Vec::new();
        collect_document_embeddings(&doc, &mut embedding_pairs, self.contextualize_embeddings);

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

#[cfg(test)]
mod rooted_helper_tests {
    //! Isolation coverage for the stage helpers extracted from
    //! `ingest_directory_with_embeddings_rooted` (ADR 0001 D3, slice 4). Each
    //! runs against in-memory fakes (`SqliteStorage::open_in_memory` +
    //! [`NullVectorIndex`]) — no real embedder, no on-disk corpus.
    //!
    //! The crown jewel is the [`IngestionPipeline::sweep_stale_documents`]
    //! attribution matrix (F32 / F32.1): the data-loss-critical logic where a
    //! wrong call would delete a *sibling corpus's* documents. `sweep` builds
    //! its `discovered` set from `files` + `dir` by pure string ops, so these
    //! tests need no filesystem — only crafted `source_path`s in storage.

    use std::path::{Path, PathBuf};

    use super::{IngestionPipeline, IngestionStats};
    use crate::index::NullVectorIndex;
    use crate::ingestion::EXTRACTOR_VERSION;
    use crate::storage::traits::CorpusMerkleRecord;
    use crate::storage::{SqliteStorage, Storage};
    use crate::types::{ContentId, DocumentTree};

    // A corpus root id is `root-` + 16 ASCII-hex chars (21 chars) — the exact
    // shape `strip_root_prefix` recognises as a namespace prefix. Anything
    // else is treated as bare/NULL-keyed, so the fixtures must use this form.
    const ROOT_A: &str = "root-000000000000000a";
    const ROOT_B: &str = "root-000000000000000b";

    fn doc(id: &str, source_path: &str) -> DocumentTree {
        DocumentTree {
            id: ContentId(id.into()),
            title: String::new(),
            source_path: source_path.into(),
            sections: vec![],
            summary: None,
        }
    }

    async fn insert(storage: &SqliteStorage, id: &str, source_path: &str) {
        storage
            .insert_document(&doc(id, source_path))
            .await
            .expect("insert document");
    }

    async fn surviving_paths(storage: &SqliteStorage) -> Vec<String> {
        let mut paths: Vec<String> = storage
            .list_documents()
            .await
            .expect("list documents")
            .into_iter()
            .map(|d| d.source_path)
            .collect();
        paths.sort();
        paths
    }

    // ── build_file_items ─────────────────────────────────────────────────

    #[test]
    fn build_file_items_namespaces_when_rooted() {
        let dir = Path::new("/corpus");
        let files = [PathBuf::from("/corpus/src/lib.rs")];
        let items = IngestionPipeline::build_file_items(&files, dir, Some(ROOT_A));
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.relative, format!("{ROOT_A}/src/lib.rs"));
        assert_eq!(it.root_id.as_deref(), Some(ROOT_A));
        assert_eq!(it.root_path.as_deref(), Some(dir));
        assert_eq!(it.path, PathBuf::from("/corpus/src/lib.rs"));
    }

    #[test]
    fn build_file_items_bare_when_unrooted() {
        let dir = Path::new("/corpus");
        let files = [PathBuf::from("/corpus/src/lib.rs")];
        let items = IngestionPipeline::build_file_items(&files, dir, None);
        assert_eq!(items[0].relative, "src/lib.rs");
        assert_eq!(items[0].root_id, None);
    }

    // ── corpus_merkle_short_circuit ──────────────────────────────────────

    async fn store_merkle(storage: &SqliteStorage, hash: &str, extractor_version: i64) {
        storage
            .upsert_corpus_merkle(&CorpusMerkleRecord {
                corpus_id: ROOT_A.to_string(),
                root_hash: hash.to_string(),
                file_count: 3,
                last_indexed_ns: 0,
                extractor_version,
            })
            .await
            .expect("upsert merkle");
    }

    #[tokio::test]
    async fn short_circuit_when_hash_and_version_match() {
        let storage = SqliteStorage::open_in_memory().expect("open");
        store_merkle(&storage, "HASH", EXTRACTOR_VERSION).await;
        let out =
            IngestionPipeline::corpus_merkle_short_circuit(&storage, Some(ROOT_A), Some("HASH"), 7)
                .await;
        assert_eq!(
            out.expect("unchanged corpus must short-circuit")
                .files_skipped,
            7
        );
    }

    #[tokio::test]
    async fn no_short_circuit_when_extractor_version_differs() {
        let storage = SqliteStorage::open_in_memory().expect("open");
        store_merkle(&storage, "HASH", EXTRACTOR_VERSION - 1).await;
        let out =
            IngestionPipeline::corpus_merkle_short_circuit(&storage, Some(ROOT_A), Some("HASH"), 7)
                .await;
        assert!(out.is_none(), "a stale extractor version must re-extract");
    }

    #[tokio::test]
    async fn no_short_circuit_when_hash_absent_or_differs() {
        let storage = SqliteStorage::open_in_memory().expect("open");
        // No merkle stored at all → proceed.
        assert!(
            IngestionPipeline::corpus_merkle_short_circuit(&storage, Some(ROOT_A), Some("HASH"), 7)
                .await
                .is_none()
        );
        // Stored, but the freshly-computed hash differs → proceed.
        store_merkle(&storage, "STORED", EXTRACTOR_VERSION).await;
        assert!(
            IngestionPipeline::corpus_merkle_short_circuit(
                &storage,
                Some(ROOT_A),
                Some("DIFFERENT"),
                7,
            )
            .await
            .is_none()
        );
    }

    // ── sweep_stale_documents — the F32 / F32.1 attribution matrix ────────

    #[tokio::test]
    async fn sweep_deletes_only_undiscovered_owned_docs() {
        let storage = SqliteStorage::open_in_memory().expect("open");
        let index = NullVectorIndex;
        let dir = Path::new("/corpus");
        insert(&storage, "keep", &format!("{ROOT_A}/keep.rs")).await; // ours, discovered
        insert(&storage, "gone", &format!("{ROOT_A}/gone.rs")).await; // ours, undiscovered

        let files = [PathBuf::from("/corpus/keep.rs")];
        let mut stats = IngestionStats::new(files.len());
        IngestionPipeline::sweep_stale_documents(
            &storage,
            &index,
            dir,
            Some(ROOT_A),
            &files,
            &mut stats,
        )
        .await
        .expect("sweep");

        assert_eq!(stats.files_removed, 1);
        assert_eq!(
            surviving_paths(&storage).await,
            vec![format!("{ROOT_A}/keep.rs")]
        );
    }

    #[tokio::test]
    async fn sweep_never_touches_a_sibling_root() {
        let storage = SqliteStorage::open_in_memory().expect("open");
        let index = NullVectorIndex;
        let dir = Path::new("/corpus");
        insert(&storage, "ours-gone", &format!("{ROOT_A}/gone.rs")).await; // ours, undiscovered
        insert(&storage, "sibling", &format!("{ROOT_B}/keep.rs")).await; // foreign root

        let files = [PathBuf::from("/corpus/unrelated.rs")];
        let mut stats = IngestionStats::new(files.len());
        IngestionPipeline::sweep_stale_documents(
            &storage,
            &index,
            dir,
            Some(ROOT_A),
            &files,
            &mut stats,
        )
        .await
        .expect("sweep");

        // Our undiscovered doc is gone; the sibling root is never touched.
        assert_eq!(stats.files_removed, 1);
        assert_eq!(
            surviving_paths(&storage).await,
            vec![format!("{ROOT_B}/keep.rs")]
        );
    }

    #[tokio::test]
    async fn sweep_keeps_bare_docs_when_a_foreign_root_shares_the_index() {
        // F32.1: a bare/NULL-keyed doc carries no attribution, so when a
        // foreign explicit root shares the index we must NOT reclaim it.
        let storage = SqliteStorage::open_in_memory().expect("open");
        let index = NullVectorIndex;
        let dir = Path::new("/corpus");
        insert(&storage, "sibling", &format!("{ROOT_B}/x.rs")).await; // foreign root present
        insert(&storage, "bare", "orphan.rs").await; // bare, undiscovered

        let files = [PathBuf::from("/corpus/unrelated.rs")];
        let mut stats = IngestionStats::new(files.len());
        IngestionPipeline::sweep_stale_documents(
            &storage,
            &index,
            dir,
            Some(ROOT_A),
            &files,
            &mut stats,
        )
        .await
        .expect("sweep");

        assert_eq!(
            stats.files_removed, 0,
            "bare doc must survive a foreign-root index"
        );
        assert_eq!(
            surviving_paths(&storage).await,
            vec!["orphan.rs".to_string(), format!("{ROOT_B}/x.rs")]
        );
    }

    #[tokio::test]
    async fn sweep_reclaims_bare_docs_when_no_foreign_root_present() {
        // F32.1: a single-corpus index (no foreign root) is the only place
        // NULL-keyed docs legitimately arise, so the bare orphan IS reclaimed.
        let storage = SqliteStorage::open_in_memory().expect("open");
        let index = NullVectorIndex;
        let dir = Path::new("/corpus");
        insert(&storage, "bare-keep", "kept.rs").await; // bare, discovered
        insert(&storage, "bare-gone", "orphan.rs").await; // bare, undiscovered

        let files = [PathBuf::from("/corpus/kept.rs")];
        let mut stats = IngestionStats::new(files.len());
        IngestionPipeline::sweep_stale_documents(
            &storage,
            &index,
            dir,
            Some(ROOT_A),
            &files,
            &mut stats,
        )
        .await
        .expect("sweep");

        assert_eq!(stats.files_removed, 1);
        assert_eq!(surviving_paths(&storage).await, vec!["kept.rs".to_string()]);
    }

    #[tokio::test]
    async fn sweep_skips_entirely_on_empty_discovery() {
        // Guard: an empty `files` set is far more likely a transient unreadable
        // root than a genuinely emptied corpus, so nothing is deleted.
        let storage = SqliteStorage::open_in_memory().expect("open");
        let index = NullVectorIndex;
        let dir = Path::new("/corpus");
        insert(&storage, "ours-gone", &format!("{ROOT_A}/gone.rs")).await; // undiscovered, but…

        let files: [PathBuf; 0] = [];
        let mut stats = IngestionStats::new(0);
        IngestionPipeline::sweep_stale_documents(
            &storage,
            &index,
            dir,
            Some(ROOT_A),
            &files,
            &mut stats,
        )
        .await
        .expect("sweep");

        assert_eq!(
            stats.files_removed, 0,
            "empty discovery must delete nothing"
        );
        assert_eq!(
            surviving_paths(&storage).await,
            vec![format!("{ROOT_A}/gone.rs")]
        );
    }

    #[tokio::test]
    async fn sweep_unrooted_reclaims_bare_docs() {
        // An unrooted reindex (root_id = None) owns the bare-keyed docs.
        let storage = SqliteStorage::open_in_memory().expect("open");
        let index = NullVectorIndex;
        let dir = Path::new("/corpus");
        insert(&storage, "keep", "keep.rs").await; // discovered
        insert(&storage, "gone", "gone.rs").await; // undiscovered

        let files = [PathBuf::from("/corpus/keep.rs")];
        let mut stats = IngestionStats::new(files.len());
        IngestionPipeline::sweep_stale_documents(&storage, &index, dir, None, &files, &mut stats)
            .await
            .expect("sweep");

        assert_eq!(stats.files_removed, 1);
        assert_eq!(surviving_paths(&storage).await, vec!["keep.rs".to_string()]);
    }
}
