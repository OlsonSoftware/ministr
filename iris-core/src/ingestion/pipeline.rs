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
use tracing::{debug, info, instrument, warn};

use crate::code::AstParser;
use crate::code::bridge::linker::BridgeLinker;
use crate::code::bridge::{BridgeEndpoint, BridgeKind, create_linker_for_kinds, detector};
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
    accumulate_language_stats, all_files_unchanged_by_mtime, compute_relative_path,
    compute_root_id, compute_sha256, file_mtime_nanos, find_root_for_file, namespace_path,
    strip_root_prefix, update_root_stats,
};
use super::symbols::{
    PendingRef, extract_code_symbols, persist_pending_refs, repair_missing_refs,
    resolve_pending_refs, store_bridge_links,
};

// ── Stats types ──────────────────────────────────────────────────────────────

/// Result of ingesting a corpus directory.
///
/// # Examples
///
/// ```
/// use iris_core::ingestion::IngestionStats;
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
        }
    }
}

/// Result of ingesting raw content via [`IngestionPipeline::ingest_content`].
///
/// # Examples
///
/// ```
/// use iris_core::ingestion::ContentIngestionStats;
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
/// use iris_core::ingestion::{IngestionProgress, IngestionPhase};
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

fn default_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(4)
        .min(16)
}

/// Result of processing a single file.
pub(super) enum FileResult {
    Skipped,
    Indexed {
        sections: usize,
        claims: usize,
        pending_refs: Vec<PendingRef>,
        bridge_endpoints: Vec<BridgeEndpoint>,
        embedding_pairs: Vec<(VectorId, String)>,
    },
}

/// A file to be ingested, with its resolved relative path and optional root ID.
struct FileItem {
    path: PathBuf,
    relative: String,
    root_id: Option<String>,
}

// ── IngestionPipeline ────────────────────────────────────────────────────────

/// Ingestion pipeline orchestrator.
///
/// # Examples
///
/// ```no_run
/// use iris_core::ingestion::IngestionPipeline;
/// use iris_core::storage::SqliteStorage;
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
        }
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

        if let Some(ref existing) = existing_hash
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

        let hash = compute_sha256(&content_str);

        if let Some(ref existing) = existing_hash
            && existing.content_hash == hash
        {
            storage
                .upsert_file_hash(&FileHashRecord {
                    path: relative_path.to_string(),
                    content_hash: hash,
                    mtime_ns: file_mtime_ns,
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
            bridge_endpoints: Vec::new(),
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
        let hash = compute_sha256(content);

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
        let hash = compute_sha256(content);

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
            let sym_result =
                extract_code_symbols(source_path, content, storage, None, None).await?;
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

        let files = discover_files(dir)?;
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

        let detected_kinds = detector::FrameworkDetector::detect(dir);
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
                }
            })
            .collect();

        let (was_cancelled, embed_count, pending_refs, bridge_endpoints) = self
            .run_producer_consumer(
                file_items,
                storage,
                embedder,
                index,
                active_graph,
                bridge_linker.as_ref(),
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
            &bridge_endpoints,
            bridge_linker.as_ref(),
            active_graph,
            storage,
        )
        .await;

        // Cleanup stale docs
        let existing_docs = if let Some(rid) = root_id {
            storage
                .list_documents_by_root(rid)
                .await
                .map_err(IngestionError::from)?
        } else {
            storage
                .list_documents()
                .await
                .map_err(IngestionError::from)?
        };
        for doc in &existing_docs {
            let source_path = strip_root_prefix(&doc.source_path).unwrap_or(&doc.source_path);
            let full_path = dir.join(source_path);
            if !full_path.exists() {
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

        // Manifest-level mtime fast skip
        if !files.is_empty()
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

        // Detect bridge frameworks
        let mut all_bridge_kinds = std::collections::BTreeSet::new();
        for path in paths {
            if path.is_dir() {
                let kinds = detector::FrameworkDetector::detect(path);
                all_bridge_kinds.extend(kinds);
            }
        }
        let bridge_kinds: Vec<BridgeKind> = all_bridge_kinds.into_iter().collect();
        let bridge_linker = create_linker_for_kinds(&bridge_kinds);
        if !bridge_kinds.is_empty() {
            info!(
                kinds = ?bridge_kinds,
                "detected bridge frameworks for cross-language linking"
            );
        }

        mem_profile::checkpoint("ingestion loop start (paths)");

        accumulate_language_stats(&files, &roots, &mut root_lang_stats, &mut root_file_counts);

        let file_items: Vec<FileItem> = files
            .iter()
            .map(|file_path| {
                let relative = compute_relative_path(file_path, paths);
                let root_id = find_root_for_file(file_path, &roots).map(String::from);
                FileItem {
                    path: file_path.clone(),
                    relative,
                    root_id,
                }
            })
            .collect();

        let (_was_cancelled, embed_count, pending_refs, bridge_endpoints) = self
            .run_producer_consumer(
                file_items,
                storage,
                embedder,
                index,
                active_graph,
                bridge_linker.as_ref(),
                &mut stats,
                None,
            )
            .await?;
        stats.total_embeddings = embed_count;

        self.finalize_ingestion(
            &pending_refs,
            &bridge_endpoints,
            bridge_linker.as_ref(),
            active_graph,
            storage,
        )
        .await;

        // Cleanup stale docs per root
        for (_root_path, rid) in &roots {
            let root_docs = storage
                .list_documents_by_root(rid)
                .await
                .map_err(IngestionError::from)?;
            for doc in &root_docs {
                let still_exists = files.iter().any(|f| {
                    let rel = compute_relative_path(f, paths);
                    rel == doc.source_path
                });
                if !still_exists {
                    debug!(path = %doc.source_path, root = %rid, "file removed, deleting from index");
                    super::embedding::delete_document_vectors(&doc.id, storage, index).await?;
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
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn run_producer_consumer<S, E, I>(
        &self,
        file_items: Vec<FileItem>,
        storage: &S,
        embedder: &E,
        index: &I,
        active_graph: Option<&PackageGraph>,
        bridge_linker: Option<&BridgeLinker>,
        stats: &mut IngestionStats,
        ct: Option<&CancellationToken>,
    ) -> Result<(bool, usize, Vec<PendingRef>, Vec<BridgeEndpoint>), IngestionError>
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
        let mut all_bridge_endpoints: Vec<BridgeEndpoint> = Vec::new();

        let producer = async {
            let mut cancelled = false;
            let mut parse_stream = std::pin::pin!(
                stream::iter(file_items)
                    .take_while(|_| {
                        let stop = ct.is_some_and(CancellationToken::is_cancelled);
                        async move { !stop }
                    })
                    .map(|item| async {
                        let result = self
                            .parse_and_store_file(
                                &item.path,
                                &item.relative,
                                storage,
                                index,
                                active_graph,
                                bridge_linker,
                            )
                            .await;
                        (item, result)
                    })
                    .buffer_unordered(concurrency)
            );

            while let Some((item, result)) = parse_stream.next().await {
                if let Some(ref progress) = self.progress {
                    progress.set_current_file(&item.relative);
                }
                match result {
                    Ok(FileResult::Skipped) => {
                        debug!(path = %item.relative, "unchanged, skipping");
                        stats.files_skipped += 1;
                    }
                    Ok(FileResult::Indexed {
                        sections,
                        claims,
                        pending_refs,
                        bridge_endpoints,
                        embedding_pairs,
                    }) => {
                        debug!(path = %item.relative, sections, claims, "parsed and stored");
                        stats.files_indexed += 1;
                        stats.total_sections += sections;
                        stats.total_claims += claims;
                        all_pending_refs.extend(pending_refs);
                        all_bridge_endpoints.extend(bridge_endpoints);

                        if let Some(ref progress) = self.progress {
                            progress.add_sections_done(sections);
                        }

                        if let Some(ref rid) = item.root_id {
                            let doc_id = crate::types::ContentId(item.relative.clone());
                            if let Err(e) = storage.set_document_root(&doc_id, rid).await {
                                debug!(path = %item.relative, error = %e, "failed to set document root");
                            }
                        }

                        if !embedding_pairs.is_empty() {
                            if let Some(ref progress) = self.progress {
                                progress.add_embeddings_total(embedding_pairs.len());
                            }
                            if embed_tx.send(embedding_pairs).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        warn!(path = %item.relative, error = %e, "failed to ingest file");
                        stats.files_failed += 1;
                    }
                }

                if let Some(ref progress) = self.progress {
                    progress.increment_done();
                }
            }
            drop(embed_tx);

            if ct.is_some_and(CancellationToken::is_cancelled) {
                cancelled = true;
            }
            cancelled
        };

        let dual = self
            .dual_embedder
            .as_ref()
            .zip(self.full_dim_storage.as_ref());
        let progress_ref = self.progress.as_ref();
        let consumer = async {
            if let Some((dual_emb, full_storage)) = dual {
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
            }
        };

        let (was_cancelled, embed_result) = futures::join!(producer, consumer);
        let embed_count = embed_result?;

        if let Some(ref progress) = self.progress {
            progress.set_phase(IngestionPhase::Finalizing);
            progress.set_current_file("");
        }

        Ok((
            was_cancelled,
            embed_count,
            all_pending_refs,
            all_bridge_endpoints,
        ))
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

        while let Some(pairs) = embed_rx.recv().await {
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

        while let Some(pairs) = embed_rx.recv().await {
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
    async fn finalize_ingestion<S: Storage + ?Sized>(
        &self,
        pending_refs: &[PendingRef],
        bridge_endpoints: &[BridgeEndpoint],
        bridge_linker: Option<&BridgeLinker>,
        active_graph: Option<&PackageGraph>,
        storage: &S,
    ) {
        let (_resolved, still_pending) =
            resolve_pending_refs(pending_refs, storage, active_graph).await;
        persist_pending_refs(&still_pending, storage).await;
        store_bridge_links(bridge_endpoints, bridge_linker, storage).await;
    }

    // ── Shared file processing (used by rooted + multi-path) ─────────────

    /// Parse, enrich, and store a single file. Embedding is deferred.
    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self, storage, index, package_graph, bridge_linker), fields(path = %relative_path))]
    async fn parse_and_store_file<S, I>(
        &self,
        file_path: &Path,
        relative_path: &str,
        storage: &S,
        index: &I,
        package_graph: Option<&PackageGraph>,
        bridge_linker: Option<&BridgeLinker>,
    ) -> Result<FileResult, IngestionError>
    where
        S: Storage + ?Sized,
        I: VectorIndex + ?Sized,
    {
        if is_in_ignored_dir(file_path) {
            debug!(path = %relative_path, "skipped: file is inside an always-ignored directory");
            return Ok(FileResult::Skipped);
        }

        let file_mtime_ns = file_mtime_nanos(file_path).await;

        let existing_hash = storage
            .get_file_hash(relative_path)
            .await
            .map_err(IngestionError::from)?;

        if let Some(ref existing) = existing_hash
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

        let hash = compute_sha256(&content_str);

        if let Some(ref existing) = existing_hash
            && existing.content_hash == hash
        {
            storage
                .upsert_file_hash(&FileHashRecord {
                    path: relative_path.to_string(),
                    content_hash: hash,
                    mtime_ns: file_mtime_ns,
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
        let mut bridge_endpoints = Vec::new();
        if parser_kind == Some(ParserKind::Code) {
            let sym_result = extract_code_symbols(
                relative_path,
                &content_str,
                storage,
                package_graph,
                bridge_linker,
            )
            .await?;
            pending_refs = sym_result.pending_refs;
            bridge_endpoints = sym_result.bridge_endpoints;
            embedding_pairs.extend(sym_result.embedding_pairs);
        }

        Ok(FileResult::Indexed {
            sections: result.section_count,
            claims: result.claim_count,
            pending_refs,
            bridge_endpoints,
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

            let mut ast_parser = AstParser::new();
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
}
