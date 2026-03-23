//! Ingestion pipeline orchestrator.
//!
//! Coordinates the full document ingestion flow: file discovery, content hashing
//! for incremental re-indexing, parsing, claim extraction, summarization, and
//! storage. The pipeline is generic over the parser and storage implementations.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use tracing::{debug, info, instrument, warn};

use crate::code::refs::extract_refs;
use crate::code::{AstParser, extract_symbols};
use crate::embedding::{Embedder, SparseEmbedder};
use crate::error::IngestionError;
use crate::extraction::claims::{ClaimExtractor, HeuristicClaimExtractor};
use crate::extraction::relationships::{HeuristicRelationshipDetector, RelationshipDetector};
use crate::extraction::summary::{ExtractiveSummaryGenerator, SummaryGenerator};
use crate::index::{SparseIndex, VectorIndex};
use crate::parser::{
    DocumentParser, MarkdownParser, ParserKind, create_parser, detect_parser_kind,
};
use crate::storage::traits::{
    FileHashRecord, Storage, SymbolFilter, SymbolRecord, SymbolRefRecord,
};
use crate::token::count_tokens;
use crate::types::{Claim, DocumentTree, Section, SymbolId, VectorId};

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
    /// Number of files discovered.
    pub files_discovered: usize,
    /// Number of files that were unchanged and skipped.
    pub files_skipped: usize,
    /// Number of files that were (re-)indexed.
    pub files_indexed: usize,
    /// Number of files that were removed from the index (deleted from disk).
    pub files_removed: usize,
    /// Number of files that failed to ingest.
    pub files_failed: usize,
    /// Total sections extracted across all indexed files.
    pub total_sections: usize,
    /// Total claims extracted across all indexed files.
    pub total_claims: usize,
    /// Total embeddings inserted into the vector index.
    pub total_embeddings: usize,
}

/// Shared progress tracker for background ingestion.
///
/// Uses atomics so it can be read from tool handlers while the ingestion
/// task updates it concurrently. Status values: 0=pending, 1=running, 2=complete.
///
/// # Examples
///
/// ```
/// use iris_core::ingestion::IngestionProgress;
/// use std::sync::Arc;
///
/// let progress = Arc::new(IngestionProgress::new());
/// progress.start(42);
/// assert_eq!(progress.files_total(), 42);
/// assert_eq!(progress.files_done(), 0);
/// assert!(progress.is_running());
///
/// progress.increment_done();
/// assert_eq!(progress.files_done(), 1);
///
/// progress.complete();
/// assert!(!progress.is_running());
/// ```
pub struct IngestionProgress {
    status: std::sync::atomic::AtomicU8,
    files_total: AtomicUsize,
    files_done: AtomicUsize,
}

impl IngestionProgress {
    /// Create a new progress tracker in the pending state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            status: std::sync::atomic::AtomicU8::new(0),
            files_total: AtomicUsize::new(0),
            files_done: AtomicUsize::new(0),
        }
    }

    /// Transition to the running state with a known file count.
    pub fn start(&self, total_files: usize) {
        self.files_total.store(total_files, Ordering::Relaxed);
        self.files_done.store(0, Ordering::Relaxed);
        self.status.store(1, Ordering::Relaxed);
    }

    /// Mark one more file as processed (indexed, skipped, or failed).
    pub fn increment_done(&self) {
        self.files_done.fetch_add(1, Ordering::Relaxed);
    }

    /// Transition to the complete state.
    pub fn complete(&self) {
        self.status.store(2, Ordering::Relaxed);
    }

    /// Returns `true` while ingestion is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.status.load(Ordering::Relaxed) == 1
    }

    /// Total files discovered for ingestion.
    #[must_use]
    pub fn files_total(&self) -> usize {
        self.files_total.load(Ordering::Relaxed)
    }

    /// Files processed so far.
    #[must_use]
    pub fn files_done(&self) -> usize {
        self.files_done.load(Ordering::Relaxed)
    }

    /// Raw status value (0=pending, 1=running, 2=complete).
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

/// Maximum number of sentences in a section-level summary.
const SUMMARY_MAX_SENTENCES: usize = 3;

/// Maximum number of sentences in a document-level summary.
const DOC_SUMMARY_MAX_SENTENCES: usize = 5;

/// Configuration for paragraph-boundary splitting on headingless documents.
/// Sections with text exceeding this word count will be split at paragraph boundaries.
const PARAGRAPH_SPLIT_THRESHOLD: usize = 500;

/// Ingestion pipeline orchestrator.
///
/// Coordinates file discovery, incremental hashing, parsing, extraction, and
/// storage for a corpus of documents.
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
    /// Optional parser override — when set, all files use this parser.
    /// When `None`, the parser is auto-detected per file from the extension.
    parser_override: Option<ParserKind>,
    /// Minimum token count for a section to remain standalone.
    /// Sections below this threshold are merged with adjacent siblings.
    min_section_tokens: usize,
    claim_extractor: HeuristicClaimExtractor,
    summary_generator: ExtractiveSummaryGenerator,
    relationship_detector: HeuristicRelationshipDetector,
    /// Optional shared progress tracker for background ingestion.
    progress: Option<Arc<IngestionProgress>>,
}

impl IngestionPipeline {
    /// Create a new ingestion pipeline with default components.
    ///
    /// Uses auto-detection to select the parser based on file extension.
    /// Section merging uses the default threshold of 50 tokens.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parser_override: None,
            min_section_tokens: 50,
            claim_extractor: HeuristicClaimExtractor::new(),
            summary_generator: ExtractiveSummaryGenerator::new(),
            relationship_detector: HeuristicRelationshipDetector::new(),
            progress: None,
        }
    }

    /// Create a new ingestion pipeline with a specific parser override.
    ///
    /// When set, all files are parsed with this parser regardless of extension.
    #[must_use]
    pub fn with_parser(kind: ParserKind) -> Self {
        Self {
            parser_override: Some(kind),
            min_section_tokens: 50,
            claim_extractor: HeuristicClaimExtractor::new(),
            summary_generator: ExtractiveSummaryGenerator::new(),
            relationship_detector: HeuristicRelationshipDetector::new(),
            progress: None,
        }
    }

    /// Attach a shared progress tracker for background ingestion monitoring.
    #[must_use]
    pub fn with_progress(mut self, progress: Arc<IngestionProgress>) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Set the minimum section token threshold for section coalescing.
    ///
    /// Sections below this threshold are merged with adjacent siblings
    /// of the same depth. Set to `0` to disable merging.
    #[must_use]
    pub fn with_min_section_tokens(mut self, min_tokens: usize) -> Self {
        self.min_section_tokens = min_tokens;
        self
    }

    /// Select the parser for a given file path.
    fn parser_for(&self, path: &Path) -> Box<dyn DocumentParser> {
        if let Some(kind) = self.parser_override {
            return create_parser(kind);
        }
        if let Some(kind) = detect_parser_kind(path) {
            return create_parser(kind);
        }
        // Fallback: use markdown parser (backward compatibility)
        Box::new(MarkdownParser::new())
    }

    /// Ingest all supported files from a directory into storage.
    ///
    /// Performs incremental re-indexing: files with unchanged content hashes
    /// are skipped. Files that no longer exist on disk are removed from the index.
    ///
    /// # Errors
    ///
    /// Returns [`IngestionError`] if directory traversal or storage operations fail.
    /// Individual file parse errors are logged and counted but do not abort the pipeline.
    #[instrument(skip(self, storage), fields(dir = %dir.display()))]
    pub async fn ingest_directory<S: Storage>(
        &self,
        dir: &Path,
        storage: &S,
    ) -> Result<IngestionStats, IngestionError> {
        let files = discover_files(dir)?;
        let mut stats = IngestionStats {
            files_discovered: files.len(),
            files_skipped: 0,
            files_indexed: 0,
            files_removed: 0,
            files_failed: 0,
            total_sections: 0,
            total_claims: 0,
            total_embeddings: 0,
        };

        if files.is_empty() {
            warn!("discovered 0 files for ingestion");
        } else {
            info!(count = files.len(), "discovered files for ingestion");
        }

        // Index new and changed files
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
                Ok(FileResult::Indexed { sections, claims }) => {
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

    /// Ingest a single file, returning whether it was skipped or indexed.
    #[instrument(skip(self, storage), fields(path = %relative_path))]
    async fn ingest_file<S: Storage>(
        &self,
        file_path: &Path,
        relative_path: &str,
        storage: &S,
    ) -> Result<FileResult, IngestionError> {
        // Read file content
        let content = tokio::fs::read(file_path)
            .await
            .map_err(|e| IngestionError::Io {
                path: file_path.to_path_buf(),
                source: e,
            })?;

        let content_str = String::from_utf8(content).map_err(|_| IngestionError::Encoding {
            path: file_path.to_path_buf(),
        })?;

        // Compute content hash
        let hash = compute_sha256(&content_str);

        // Check if file is unchanged
        let existing_hash = storage
            .get_file_hash(relative_path)
            .await
            .map_err(IngestionError::from)?;

        if let Some(ref existing) = existing_hash {
            if existing.content_hash == hash {
                return Ok(FileResult::Skipped);
            }
        }

        // Parse the document (auto-detect parser from extension or use override)
        let parser = self.parser_for(Path::new(relative_path));
        let mut doc = parser.parse(Path::new(relative_path), &content_str)?;

        // Handle paragraph-boundary splitting for large headingless sections
        doc.sections = doc
            .sections
            .into_iter()
            .flat_map(|s| split_large_headingless_section(s, relative_path))
            .collect();

        // Coalesce small adjacent sibling sections
        doc.sections = coalesce_small_sections(doc.sections, self.min_section_tokens);

        // Enrich sections with claims and summaries
        let (section_count, claim_count) = enrich_sections(
            &mut doc.sections,
            &self.claim_extractor,
            &self.summary_generator,
        );

        // Generate document-level summary from section texts
        let all_text = collect_all_text(&doc.sections);
        if !all_text.is_empty() {
            doc.summary = Some(
                self.summary_generator
                    .summarize(&all_text, DOC_SUMMARY_MAX_SENTENCES),
            );
        }

        // Delete existing document if re-indexing
        if existing_hash.is_some() {
            storage
                .delete_document(&doc.id)
                .await
                .map_err(IngestionError::from)?;
        }

        // Store the enriched document
        storage
            .insert_document(&doc)
            .await
            .map_err(IngestionError::from)?;

        // Detect and store claim relationships
        let all_claims = collect_all_claims(&doc.sections);
        if all_claims.len() >= 2 {
            let relationships = self.relationship_detector.detect(&all_claims);
            if !relationships.is_empty() {
                debug!(
                    path = %relative_path,
                    count = relationships.len(),
                    "detected claim relationships"
                );
                storage
                    .insert_claim_relationships(&relationships)
                    .await
                    .map_err(IngestionError::from)?;
            }
        }

        // Update file hash
        storage
            .upsert_file_hash(&FileHashRecord {
                path: relative_path.to_string(),
                content_hash: hash,
            })
            .await
            .map_err(IngestionError::from)?;

        Ok(FileResult::Indexed {
            sections: section_count,
            claims: claim_count,
        })
    }

    /// Ingest raw content directly (without file I/O).
    ///
    /// Useful for web-fetched content that already exists as a string. The
    /// `source_path` is a virtual path used as the document ID and for section
    /// ID generation (e.g. `"web://example.com/docs/guide"`).
    ///
    /// # Errors
    ///
    /// Returns [`IngestionError`] if parsing or storage fails.
    #[instrument(skip(self, content, storage), fields(source = %source_path))]
    pub async fn ingest_content<S: Storage>(
        &self,
        source_path: &str,
        content: &str,
        parser_kind: ParserKind,
        storage: &S,
    ) -> Result<ContentIngestionStats, IngestionError> {
        let hash = compute_sha256(content);

        // Check if content is unchanged
        let existing_hash = storage
            .get_file_hash(source_path)
            .await
            .map_err(IngestionError::from)?;

        if let Some(ref existing) = existing_hash {
            if existing.content_hash == hash {
                return Ok(ContentIngestionStats {
                    sections: 0,
                    claims: 0,
                    skipped: true,
                });
            }
        }

        // Parse the content
        let parser = create_parser(parser_kind);
        let mut doc = parser.parse(Path::new(source_path), content)?;

        // Handle paragraph-boundary splitting for large headingless sections
        doc.sections = doc
            .sections
            .into_iter()
            .flat_map(|s| split_large_headingless_section(s, source_path))
            .collect();

        // Coalesce small adjacent sibling sections
        doc.sections = coalesce_small_sections(doc.sections, self.min_section_tokens);

        // Enrich sections with claims and summaries
        let (section_count, claim_count) = enrich_sections(
            &mut doc.sections,
            &self.claim_extractor,
            &self.summary_generator,
        );

        // Generate document-level summary
        let all_text = collect_all_text(&doc.sections);
        if !all_text.is_empty() {
            doc.summary = Some(
                self.summary_generator
                    .summarize(&all_text, DOC_SUMMARY_MAX_SENTENCES),
            );
        }

        // Delete existing document if re-indexing
        if existing_hash.is_some() {
            storage
                .delete_document(&doc.id)
                .await
                .map_err(IngestionError::from)?;
        }

        // Store the enriched document
        storage
            .insert_document(&doc)
            .await
            .map_err(IngestionError::from)?;

        // Detect and store claim relationships
        let all_claims = collect_all_claims(&doc.sections);
        if all_claims.len() >= 2 {
            let relationships = self.relationship_detector.detect(&all_claims);
            if !relationships.is_empty() {
                debug!(
                    source = %source_path,
                    count = relationships.len(),
                    "detected claim relationships"
                );
                storage
                    .insert_claim_relationships(&relationships)
                    .await
                    .map_err(IngestionError::from)?;
            }
        }

        // Update file hash
        storage
            .upsert_file_hash(&FileHashRecord {
                path: source_path.to_string(),
                content_hash: hash,
            })
            .await
            .map_err(IngestionError::from)?;

        info!(source = %source_path, section_count, claim_count, "ingested content");

        Ok(ContentIngestionStats {
            sections: section_count,
            claims: claim_count,
            skipped: false,
        })
    }

    /// Ingest raw content with multi-resolution embedding.
    ///
    /// Like [`ingest_content`](Self::ingest_content) but also embeds summaries,
    /// sections, and claims into the vector index.
    ///
    /// # Errors
    ///
    /// Returns [`IngestionError`] if parsing, storage, or embedding fails.
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

        // Check if content is unchanged
        let existing_hash = storage
            .get_file_hash(source_path)
            .await
            .map_err(IngestionError::from)?;

        if let Some(ref existing) = existing_hash {
            if existing.content_hash == hash {
                return Ok(ContentIngestionStats {
                    sections: 0,
                    claims: 0,
                    skipped: true,
                });
            }
        }

        // Parse the content
        let parser = create_parser(parser_kind);
        let mut doc = parser.parse(Path::new(source_path), content)?;

        // Handle paragraph-boundary splitting
        doc.sections = doc
            .sections
            .into_iter()
            .flat_map(|s| split_large_headingless_section(s, source_path))
            .collect();

        // Coalesce small adjacent sibling sections
        doc.sections = coalesce_small_sections(doc.sections, self.min_section_tokens);

        // Enrich sections with claims and summaries
        let (section_count, claim_count) = enrich_sections(
            &mut doc.sections,
            &self.claim_extractor,
            &self.summary_generator,
        );

        // Generate document-level summary
        let all_text = collect_all_text(&doc.sections);
        if !all_text.is_empty() {
            doc.summary = Some(
                self.summary_generator
                    .summarize(&all_text, DOC_SUMMARY_MAX_SENTENCES),
            );
        }

        // Delete old document + embeddings if re-indexing
        if existing_hash.is_some() {
            delete_document_vectors(&doc.id, storage, index).await?;
            storage
                .delete_document(&doc.id)
                .await
                .map_err(IngestionError::from)?;
        }

        // Store the enriched document
        storage
            .insert_document(&doc)
            .await
            .map_err(IngestionError::from)?;

        // Embed all resolution levels
        embed_document(&doc, embedder, index)?;

        // For code files: extract symbols, store in SQLite, and embed into vector index
        if parser_kind == ParserKind::Code {
            embed_code_symbols(source_path, content, storage, embedder, index).await?;
        }

        // Detect and store claim relationships
        let all_claims = collect_all_claims(&doc.sections);
        if all_claims.len() >= 2 {
            let relationships = self.relationship_detector.detect(&all_claims);
            if !relationships.is_empty() {
                debug!(
                    source = %source_path,
                    count = relationships.len(),
                    "detected claim relationships"
                );
                storage
                    .insert_claim_relationships(&relationships)
                    .await
                    .map_err(IngestionError::from)?;
            }
        }

        // Update file hash
        storage
            .upsert_file_hash(&FileHashRecord {
                path: source_path.to_string(),
                content_hash: hash,
            })
            .await
            .map_err(IngestionError::from)?;

        info!(source = %source_path, section_count, claim_count, "ingested content with embeddings");

        Ok(ContentIngestionStats {
            sections: section_count,
            claims: claim_count,
            skipped: false,
        })
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
    /// Number of sections extracted.
    pub sections: usize,
    /// Number of claims extracted.
    pub claims: usize,
    /// Whether the content was unchanged and skipped.
    pub skipped: bool,
}

impl IngestionPipeline {
    /// Ingest a directory with multi-resolution embedding.
    ///
    /// This method extends [`ingest_directory`](Self::ingest_directory) by also
    /// embedding summaries, sections, and claims into the vector index. When
    /// re-indexing a changed file, old embeddings are deleted before inserting new ones.
    ///
    /// # Errors
    ///
    /// Returns [`IngestionError`] if directory traversal, storage, or embedding fails.
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
        let files = discover_files(dir)?;
        let mut stats = IngestionStats {
            files_discovered: files.len(),
            files_skipped: 0,
            files_indexed: 0,
            files_removed: 0,
            files_failed: 0,
            total_sections: 0,
            total_claims: 0,
            total_embeddings: 0,
        };

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

        for file_path in &files {
            let relative = file_path
                .strip_prefix(dir)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            match self
                .ingest_file_with_embeddings(file_path, &relative, storage, embedder, index)
                .await
            {
                Ok(FileResult::Skipped) => {
                    debug!(path = %relative, "unchanged, skipping");
                    stats.files_skipped += 1;
                }
                Ok(FileResult::Indexed { sections, claims }) => {
                    debug!(path = %relative, sections, claims, "indexed with embeddings");
                    stats.files_indexed += 1;
                    stats.total_sections += sections;
                    stats.total_claims += claims;
                }
                Err(e) => {
                    warn!(path = %relative, error = %e, "failed to ingest file");
                    stats.files_failed += 1;
                }
            }

            if let Some(ref progress) = self.progress {
                progress.increment_done();
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
                // Delete embeddings before removing document from storage
                delete_document_vectors(&doc.id, storage, index).await?;
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

    /// Ingest files from multiple paths (directories, individual files, or globs) with embeddings.
    ///
    /// Uses [`discover_paths`] to resolve the input paths into a deduplicated file list,
    /// then ingests each file. Relative paths for storage are computed by stripping
    /// the common base directory for directory entries, or using the filename for
    /// individual files.
    ///
    /// # Errors
    ///
    /// Returns [`IngestionError`] if path resolution, storage, or embedding fails.
    #[instrument(skip(self, storage, embedder, index), fields(path_count = paths.len()))]
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
        let files = discover_paths(paths)?;
        let mut stats = IngestionStats {
            files_discovered: files.len(),
            files_skipped: 0,
            files_indexed: 0,
            files_removed: 0,
            files_failed: 0,
            total_sections: 0,
            total_claims: 0,
            total_embeddings: 0,
        };

        if files.is_empty() {
            warn!("discovered 0 files from multiple paths (with embeddings)");
        } else {
            info!(
                count = files.len(),
                "discovered files from multiple paths (with embeddings)"
            );
        }

        if let Some(ref progress) = self.progress {
            progress.start(files.len());
        }

        for file_path in &files {
            let relative = compute_relative_path(file_path, paths);

            match self
                .ingest_file_with_embeddings(file_path, &relative, storage, embedder, index)
                .await
            {
                Ok(FileResult::Skipped) => {
                    debug!(path = %relative, "unchanged, skipping");
                    stats.files_skipped += 1;
                }
                Ok(FileResult::Indexed { sections, claims }) => {
                    debug!(path = %relative, sections, claims, "indexed with embeddings");
                    stats.files_indexed += 1;
                    stats.total_sections += sections;
                    stats.total_claims += claims;
                }
                Err(e) => {
                    warn!(path = %relative, error = %e, "failed to ingest file");
                    stats.files_failed += 1;
                }
            }

            if let Some(ref progress) = self.progress {
                progress.increment_done();
            }
        }

        // Remove documents for files that no longer exist in any of the resolved paths
        let existing_docs = storage
            .list_documents()
            .await
            .map_err(IngestionError::from)?;
        for doc in &existing_docs {
            // Check if the source path still maps to an existing file
            let still_exists = files.iter().any(|f| {
                let rel = compute_relative_path(f, paths);
                rel == doc.source_path
            });
            if !still_exists {
                debug!(path = %doc.source_path, "file removed, deleting from index");
                delete_document_vectors(&doc.id, storage, index).await?;
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
            "multi-path ingestion with embeddings complete"
        );

        if let Some(ref progress) = self.progress {
            progress.complete();
        }

        Ok(stats)
    }

    /// Ingest a single file with multi-resolution embedding.
    #[instrument(skip(self, storage, embedder, index), fields(path = %relative_path))]
    async fn ingest_file_with_embeddings<S, E, I>(
        &self,
        file_path: &Path,
        relative_path: &str,
        storage: &S,
        embedder: &E,
        index: &I,
    ) -> Result<FileResult, IngestionError>
    where
        S: Storage + ?Sized,
        E: Embedder + ?Sized,
        I: VectorIndex + ?Sized,
    {
        // Read file content
        let content = tokio::fs::read(file_path)
            .await
            .map_err(|e| IngestionError::Io {
                path: file_path.to_path_buf(),
                source: e,
            })?;

        let content_str = String::from_utf8(content).map_err(|_| IngestionError::Encoding {
            path: file_path.to_path_buf(),
        })?;

        // Compute content hash
        let hash = compute_sha256(&content_str);

        // Check if file is unchanged
        let existing_hash = storage
            .get_file_hash(relative_path)
            .await
            .map_err(IngestionError::from)?;

        if let Some(ref existing) = existing_hash {
            if existing.content_hash == hash {
                return Ok(FileResult::Skipped);
            }
        }

        // Parse the document (auto-detect parser from extension or use override)
        let parser = self.parser_for(Path::new(relative_path));
        let mut doc = parser.parse(Path::new(relative_path), &content_str)?;

        // Handle paragraph-boundary splitting
        doc.sections = doc
            .sections
            .into_iter()
            .flat_map(|s| split_large_headingless_section(s, relative_path))
            .collect();

        // Coalesce small adjacent sibling sections
        doc.sections = coalesce_small_sections(doc.sections, self.min_section_tokens);

        // Enrich sections with claims and summaries
        let (section_count, claim_count) = enrich_sections(
            &mut doc.sections,
            &self.claim_extractor,
            &self.summary_generator,
        );

        // Generate document-level summary
        let all_text = collect_all_text(&doc.sections);
        if !all_text.is_empty() {
            doc.summary = Some(
                self.summary_generator
                    .summarize(&all_text, DOC_SUMMARY_MAX_SENTENCES),
            );
        }

        // Delete old document + embeddings if re-indexing
        if existing_hash.is_some() {
            delete_document_vectors(&doc.id, storage, index).await?;
            storage
                .delete_document(&doc.id)
                .await
                .map_err(IngestionError::from)?;
        }

        // Store the enriched document
        storage
            .insert_document(&doc)
            .await
            .map_err(IngestionError::from)?;

        // Embed all resolution levels
        embed_document(&doc, embedder, index)?;

        // For code files: extract symbols, store in SQLite, and embed into vector index
        let parser_kind = self
            .parser_override
            .or_else(|| detect_parser_kind(Path::new(relative_path)));
        if parser_kind == Some(ParserKind::Code) {
            embed_code_symbols(relative_path, &content_str, storage, embedder, index).await?;
        }

        // Detect and store claim relationships
        let all_claims = collect_all_claims(&doc.sections);
        if all_claims.len() >= 2 {
            let relationships = self.relationship_detector.detect(&all_claims);
            if !relationships.is_empty() {
                debug!(
                    path = %relative_path,
                    count = relationships.len(),
                    "detected claim relationships"
                );
                storage
                    .insert_claim_relationships(&relationships)
                    .await
                    .map_err(IngestionError::from)?;
            }
        }

        // Update file hash
        storage
            .upsert_file_hash(&FileHashRecord {
                path: relative_path.to_string(),
                content_hash: hash,
            })
            .await
            .map_err(IngestionError::from)?;

        Ok(FileResult::Indexed {
            sections: section_count,
            claims: claim_count,
        })
    }
}

/// Embed a document tree at all three resolution levels.
///
/// Inserts vectors for:
/// - Document-level summary (if present)
/// - Each section's summary (if present) and full text
/// - Each claim
fn embed_document<E: Embedder + ?Sized, I: VectorIndex + ?Sized>(
    doc: &DocumentTree,
    embedder: &E,
    index: &I,
) -> Result<usize, IngestionError> {
    let mut texts: Vec<String> = Vec::new();
    let mut ids: Vec<VectorId> = Vec::new();

    // Document-level summary
    if let Some(ref summary) = doc.summary {
        if !summary.trim().is_empty() {
            ids.push(VectorId::doc_summary(doc.id.as_ref()));
            texts.push(summary.clone());
        }
    }

    // Collect section and claim texts
    collect_embeddable_items(&doc.sections, &mut ids, &mut texts);

    if texts.is_empty() {
        return Ok(0);
    }

    // Batch embed all texts
    let text_refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let vectors = embedder
        .embed(&text_refs)
        .map_err(|e| IngestionError::Embedding {
            reason: e.to_string(),
        })?;

    // Insert each vector into the index
    for (vid, vector) in ids.iter().zip(vectors.iter()) {
        index
            .insert(vid.as_str(), vector)
            .map_err(|e| IngestionError::Embedding {
                reason: format!("failed to insert vector {vid}: {e}"),
            })?;
    }

    let count = ids.len();
    debug!(embeddings = count, doc_id = %doc.id, "embedded document");
    Ok(count)
}

/// Embed a document tree into the sparse index using a SPLADE-style model.
///
/// Mirrors [`embed_document`] but produces sparse vectors instead of dense.
/// Called by ingestion orchestrators when sparse search components are configured.
///
/// # Errors
///
/// Returns [`IngestionError::Embedding`] if sparse embedding or insertion fails.
pub fn embed_document_sparse<SE: SparseEmbedder + ?Sized, SI: SparseIndex + ?Sized>(
    doc: &DocumentTree,
    sparse_embedder: &SE,
    sparse_index: &SI,
) -> Result<usize, IngestionError> {
    let mut texts: Vec<String> = Vec::new();
    let mut ids: Vec<VectorId> = Vec::new();

    if let Some(ref summary) = doc.summary {
        if !summary.trim().is_empty() {
            ids.push(VectorId::doc_summary(doc.id.as_ref()));
            texts.push(summary.clone());
        }
    }

    collect_embeddable_items(&doc.sections, &mut ids, &mut texts);

    if texts.is_empty() {
        return Ok(0);
    }

    let text_refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let sparse_vecs =
        sparse_embedder
            .embed_sparse(&text_refs)
            .map_err(|e| IngestionError::Embedding {
                reason: format!("sparse embedding failed: {e}"),
            })?;

    for (vid, sv) in ids.iter().zip(sparse_vecs.iter()) {
        sparse_index
            .insert_sparse(vid.as_str(), &sv.indices, &sv.values)
            .map_err(|e| IngestionError::Embedding {
                reason: format!("failed to insert sparse vector {vid}: {e}"),
            })?;
    }

    let count = ids.len();
    debug!(sparse_embeddings = count, doc_id = %doc.id, "sparse-embedded document");
    Ok(count)
}

/// Recursively collect embeddable items (section summaries, section texts, claims).
fn collect_embeddable_items(
    sections: &[Section],
    ids: &mut Vec<VectorId>,
    texts: &mut Vec<String>,
) {
    for section in sections {
        // Section summary
        if let Some(ref summary) = section.summary {
            if !summary.trim().is_empty() {
                ids.push(VectorId::sec_summary(section.id.as_ref()));
                texts.push(summary.clone());
            }
        }

        // Section full text
        if !section.text.trim().is_empty() {
            ids.push(VectorId::section(section.id.as_ref()));
            texts.push(section.text.clone());
        }

        // Claims
        for claim in &section.claims {
            if !claim.text.trim().is_empty() {
                ids.push(VectorId::claim(claim.id.as_ref()));
                texts.push(claim.text.clone());
            }
        }

        // Recurse into children
        collect_embeddable_items(&section.children, ids, texts);
    }
}

/// Recursively collect all claims from a section tree.
fn collect_all_claims(sections: &[Section]) -> Vec<Claim> {
    let mut claims = Vec::new();
    for section in sections {
        claims.extend(section.claims.iter().cloned());
        claims.extend(collect_all_claims(&section.children));
    }
    claims
}

/// Extract code symbols from a source file, store them in `SQLite`, and embed
/// into the HNSW vector index as `symbol-stub` and `symbol-full` vectors.
///
/// Symbol stubs embed `"signature\ndoc_comment"` for high-precision search.
/// Symbol full embeds the complete symbol source for broader matching.
#[allow(clippy::too_many_lines)]
async fn embed_code_symbols<S, E, I>(
    relative_path: &str,
    content: &str,
    storage: &S,
    embedder: &E,
    index: &I,
) -> Result<usize, IngestionError>
where
    S: Storage + ?Sized,
    E: Embedder + ?Sized,
    I: VectorIndex + ?Sized,
{
    let source = content.as_bytes();

    let mut ast_parser = AstParser::new();
    let Ok(tree) = ast_parser.parse(source) else {
        return Ok(0); // Unparseable code — skip symbol embedding
    };

    let file_stem = Path::new(relative_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let module_path: Vec<&str> = if file_stem == "lib" || file_stem == "main" || file_stem == "mod"
    {
        vec![]
    } else {
        vec![file_stem]
    };

    let symbols = extract_symbols(&tree, source, relative_path, &module_path);

    if symbols.is_empty() {
        return Ok(0);
    }

    // Delete old symbols for this file (re-index safety)
    let _ = storage.delete_symbols_for_file(relative_path).await;

    // Convert to SymbolRecords and store in SQLite
    let symbol_records: Vec<SymbolRecord> = symbols
        .iter()
        .map(|sym| {
            let module_str = sym.module_path.join("::");
            // Disambiguate impl blocks to avoid ID collision with the type they implement
            let qualified_name = if sym.kind == crate::code::ItemKind::Impl {
                format!("impl-{}", sym.name)
            } else {
                sym.name.clone()
            };
            let symbol_id = if module_str.is_empty() {
                format!("sym-{relative_path}::{qualified_name}")
            } else {
                format!("sym-{relative_path}::{module_str}::{qualified_name}")
            };

            // Compute line numbers from byte range
            #[allow(clippy::cast_possible_truncation)]
            let line_start = content[..sym.byte_range.start].matches('\n').count() as u32 + 1;
            #[allow(clippy::cast_possible_truncation)]
            let line_end = content[..sym.byte_range.end].matches('\n').count() as u32 + 1;

            // Compute cyclomatic complexity for function symbols
            let cyclomatic_complexity = if sym.kind == crate::code::ItemKind::Function {
                tree.root_node()
                    .descendant_for_byte_range(sym.byte_range.start, sym.byte_range.end)
                    .map(|node| crate::code::cyclomatic_complexity(&node, source))
            } else {
                None
            };

            SymbolRecord {
                id: SymbolId(symbol_id),
                file_path: relative_path.to_string(),
                name: sym.name.clone(),
                kind: sym.kind.as_str().to_string(),
                visibility: sym.visibility.as_str().to_string(),
                signature: sym.signature.clone(),
                doc_comment: sym.doc_comment.clone(),
                module_path: module_str,
                line_start,
                line_end,
                cyclomatic_complexity,
            }
        })
        .collect();

    storage
        .insert_symbols(&symbol_records)
        .await
        .map_err(IngestionError::from)?;

    // Extract and resolve cross-references (use imports, impl relationships)
    // Derive language from file extension for multi-language ref extraction.
    let language = Path::new(relative_path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|ext| crate::code::GrammarRegistry::global().language_name_for_extension(ext))
        .unwrap_or("rust");
    if let Err(e) = resolve_and_store_refs(
        &tree,
        source,
        relative_path,
        language,
        &symbol_records,
        storage,
    )
    .await
    {
        warn!(path = %relative_path, error = %e, "failed to extract symbol refs");
    }

    // Build embeddable texts for symbol-stub and symbol-full
    let mut ids: Vec<VectorId> = Vec::new();
    let mut texts: Vec<String> = Vec::new();

    for (sym, record) in symbols.iter().zip(symbol_records.iter()) {
        // Symbol stub: signature + doc comment
        let stub_text = match &sym.doc_comment {
            Some(doc) => format!("{}\n{doc}", sym.signature),
            None => sym.signature.clone(),
        };
        if !stub_text.trim().is_empty() {
            ids.push(VectorId::symbol_stub(record.id.as_ref()));
            texts.push(stub_text);
        }

        // Symbol full: complete source code
        let full_text = &content[sym.byte_range.clone()];
        if !full_text.trim().is_empty() {
            ids.push(VectorId::symbol_full(record.id.as_ref()));
            texts.push(full_text.to_string());
        }
    }

    if texts.is_empty() {
        return Ok(0);
    }

    // Batch embed
    let text_refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let vectors = embedder
        .embed(&text_refs)
        .map_err(|e| IngestionError::Embedding {
            reason: e.to_string(),
        })?;

    for (vid, vector) in ids.iter().zip(vectors.iter()) {
        index
            .insert(vid.as_str(), vector)
            .map_err(|e| IngestionError::Embedding {
                reason: format!("failed to insert symbol vector {vid}: {e}"),
            })?;
    }

    let count = ids.len();
    debug!(
        symbols = count,
        path = %relative_path,
        "embedded code symbols"
    );
    Ok(count)
}

/// Resolve raw cross-references against the stored symbol table and persist them.
///
/// Extracts `use` imports and `impl Trait for Type` relationships from the AST,
/// resolves target names against known symbols, and stores the resulting
/// `SymbolRefRecord` values. Refs that can't be resolved (external crates,
/// missing symbols) are silently skipped.
async fn resolve_and_store_refs<S: Storage + ?Sized>(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
    local_symbols: &[SymbolRecord],
    storage: &S,
) -> Result<usize, IngestionError> {
    let raw_refs = extract_refs(tree, source, language);
    if raw_refs.is_empty() {
        return Ok(0);
    }

    // Build a set of local symbol IDs for fast existence checks.
    let local_id_set: std::collections::HashSet<&SymbolId> =
        local_symbols.iter().map(|s| &s.id).collect();

    // Delete existing refs for this file before inserting new ones
    let _ = storage.delete_refs_for_file(file_path).await;

    // For import refs, we need a valid "from" symbol that exists in the DB.
    // Prefer a module-level symbol as the anchor (most meaningful for imports),
    // then fall back to the first top-level type/function definition.
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

    let mut resolved = Vec::new();

    for raw in &raw_refs {
        // Determine the "from" symbol ID
        let from_id = match &raw.from_context {
            // For impl refs: the from_context is the implementing type name
            Some(type_name) => local_symbols
                .iter()
                .find(|s| {
                    s.name == *type_name
                        && (s.kind == "struct" || s.kind == "enum" || s.kind == "type")
                })
                .map(|s| s.id.clone()),
            // For imports: use the file's anchor symbol
            None => file_anchor.clone(),
        };

        let Some(from_id) = from_id else {
            continue;
        };

        // Resolve the target symbol by name — must be a primary definition,
        // not an impl block or nested item (those have composite names that
        // won't match simple use-path imports).
        let target_filter = SymbolFilter {
            name: Some(raw.target_name.clone()),
            ..SymbolFilter::default()
        };

        let Ok(matches) = storage.list_symbols(&target_filter).await else {
            continue;
        };

        // Filter to primary definitions (structs, enums, traits, functions, types)
        // to avoid matching enum variants or impl methods whose symbol IDs
        // contain :: separators that violate FK constraints.
        let primary: Vec<_> = matches
            .iter()
            .filter(|s| {
                matches!(
                    s.kind.as_str(),
                    "struct" | "enum" | "trait" | "function" | "type" | "const" | "static" | "mod"
                )
            })
            .collect();

        let target = match primary.len() {
            0 => continue, // No valid target found
            1 => primary[0],
            _ => {
                // Prefer a target in a different file (cross-file ref is more useful)
                primary
                    .iter()
                    .find(|s| s.file_path != file_path)
                    .copied()
                    .unwrap_or(primary[0])
            }
        };

        // Skip self-references
        if from_id == target.id {
            continue;
        }

        // Verify the from_id exists in our local symbols (it should, since
        // it was derived from local_symbols, but guard against edge cases).
        if !local_id_set.contains(&from_id) {
            continue;
        }

        resolved.push(SymbolRefRecord {
            from_symbol_id: from_id,
            to_symbol_id: target.id.clone(),
            ref_kind: raw.kind,
        });
    }

    if resolved.is_empty() {
        return Ok(0);
    }

    // Insert refs one at a time, skipping any that violate FK constraints
    // (target symbol may have been deleted or renamed between resolution and insert)
    let mut inserted = 0;
    for r in &resolved {
        if storage
            .insert_symbol_refs(std::slice::from_ref(r))
            .await
            .is_ok()
        {
            inserted += 1;
        }
    }

    if inserted > 0 {
        debug!(
            refs = inserted,
            path = %file_path,
            "resolved symbol cross-references"
        );
    }

    Ok(inserted)
}

/// Delete all vectors associated with a document from the index.
///
/// Queries storage for the document's sections and claims, derives their
/// vector IDs, and deletes them from the index.
async fn delete_document_vectors<S: Storage + ?Sized, I: VectorIndex + ?Sized>(
    doc_id: &crate::types::ContentId,
    storage: &S,
    index: &I,
) -> Result<usize, IngestionError> {
    let mut deleted = 0;

    // Delete document summary vector
    let vid = VectorId::doc_summary(doc_id.as_ref());
    if index
        .delete(vid.as_str())
        .map_err(|e| IngestionError::Embedding {
            reason: e.to_string(),
        })?
    {
        deleted += 1;
    }

    // Get all sections for this document
    let sections = storage
        .list_sections(doc_id)
        .await
        .map_err(IngestionError::from)?;

    for section in &sections {
        // Delete section summary vector
        let vid = VectorId::sec_summary(section.id.as_ref());
        if index
            .delete(vid.as_str())
            .map_err(|e| IngestionError::Embedding {
                reason: e.to_string(),
            })?
        {
            deleted += 1;
        }

        // Delete section text vector
        let vid = VectorId::section(section.id.as_ref());
        if index
            .delete(vid.as_str())
            .map_err(|e| IngestionError::Embedding {
                reason: e.to_string(),
            })?
        {
            deleted += 1;
        }

        // Delete claim vectors
        let claims = storage
            .list_claims(&section.id)
            .await
            .map_err(IngestionError::from)?;
        for claim in &claims {
            let vid = VectorId::claim(claim.id.as_ref());
            if index
                .delete(vid.as_str())
                .map_err(|e| IngestionError::Embedding {
                    reason: e.to_string(),
                })?
            {
                deleted += 1;
            }
        }
    }

    // Delete symbol vectors for the document's source path
    let doc_record = storage
        .get_document(doc_id)
        .await
        .map_err(IngestionError::from)?;
    if let Some(doc) = doc_record {
        let symbols = storage
            .list_symbols(&crate::storage::SymbolFilter {
                file_path: Some(doc.source_path.clone()),
                ..Default::default()
            })
            .await
            .map_err(IngestionError::from)?;
        for sym in &symbols {
            let stub_vid = VectorId::symbol_stub(sym.id.as_ref());
            if index.delete(stub_vid.as_str()).unwrap_or(false) {
                deleted += 1;
            }
            let full_vid = VectorId::symbol_full(sym.id.as_ref());
            if index.delete(full_vid.as_str()).unwrap_or(false) {
                deleted += 1;
            }
        }
        let _ = storage.delete_symbols_for_file(&doc.source_path).await;
    }

    debug!(deleted, doc_id = %doc_id, "deleted document vectors");
    Ok(deleted)
}

impl Default for IngestionPipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of processing a single file.
enum FileResult {
    /// File was unchanged and skipped.
    Skipped,
    /// File was indexed with the given counts.
    Indexed { sections: usize, claims: usize },
}

/// Directories that are always skipped during file discovery, regardless of
/// `.gitignore` settings. These are build artifacts, dependency caches, and
/// IDE/editor directories that never contain useful content to index.
const ALWAYS_IGNORE_DIRS: &[&str] = &[
    // Rust / Cargo
    "target",
    // JavaScript / Node
    "node_modules",
    ".next",
    ".nuxt",
    ".output",
    // Python
    "__pycache__",
    ".venv",
    "venv",
    "env",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    // Java / Gradle
    ".gradle",
    // General build output
    "dist",
    "build",
    "out",
    // VCS
    ".git",
    // IDE / Editor
    ".idea",
    ".vs",
    ".vscode",
    // Caches
    ".cache",
    // Vendor / deps
    "vendor",
];

/// File patterns that are always skipped during file discovery.
/// Minified bundles, source maps, and lockfiles add noise without value.
const ALWAYS_IGNORE_PATTERNS: &[&str] = &[
    "*.min.js",
    "*.min.css",
    "*.map",
    "*.lock",
    "package-lock.json",
    "Cargo.lock",
    "yarn.lock",
    "pnpm-lock.yaml",
    "*.chunk.js",
    "*.bundle.js",
];

/// Discover all supported files in a directory recursively.
///
/// Respects `.gitignore` rules and skips well-known junk directories
/// (`node_modules`, `target`, `.git`, `dist`, `build`, etc.) and file
/// patterns (`*.min.js`, `*.lock`, etc.).
///
/// # Errors
///
/// Returns [`IngestionError::Io`] if the directory cannot be read.
pub fn discover_files(dir: &Path) -> Result<Vec<PathBuf>, IngestionError> {
    use ignore::WalkBuilder;
    use ignore::overrides::OverrideBuilder;

    let mut overrides = OverrideBuilder::new(dir);
    for pattern in ALWAYS_IGNORE_PATTERNS {
        // Prefix with `!` to negate (exclude) the pattern
        let _ = overrides.add(&format!("!{pattern}"));
    }
    let overrides = overrides.build().map_err(|e| IngestionError::Io {
        path: dir.to_path_buf(),
        source: std::io::Error::other(format!("invalid ignore pattern: {e}")),
    })?;

    let mut walker = WalkBuilder::new(dir);
    walker
        .hidden(false) // don't skip dotfiles by default (we handle .git via ALWAYS_IGNORE_DIRS)
        .parents(true) // read .gitignore from parent directories (critical for subdirectory corpus roots)
        .git_ignore(true) // respect .gitignore
        .git_global(true) // respect global gitignore
        .git_exclude(true) // respect .git/info/exclude
        .overrides(overrides)
        .filter_entry(|entry| {
            // Skip well-known junk directories
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                if let Some(name) = entry.file_name().to_str() {
                    if ALWAYS_IGNORE_DIRS.contains(&name) {
                        return false;
                    }
                }
            }
            true
        });

    let mut files = Vec::new();
    for result in walker.build() {
        let entry = result.map_err(|e| IngestionError::Io {
            path: dir.to_path_buf(),
            source: std::io::Error::other(format!("walk error: {e}")),
        })?;
        let path = entry.into_path();
        if path.is_file() && is_supported_file(&path) {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

/// Discover supported files from a mix of directories, individual files, and glob patterns.
///
/// Each entry in `paths` is classified as:
/// - **Directory** — recursively walked via [`discover_files`].
/// - **Glob pattern** (contains `*`, `?`, or `[`) — expanded via the `glob` crate,
///   then each match is classified as a directory or individual file.
/// - **Individual file** — included directly if it has a supported extension.
///
/// Results are deduplicated by canonical path and returned sorted.
///
/// # Errors
///
/// Returns [`IngestionError`] if directory traversal fails or a glob pattern is invalid.
///
/// # Examples
///
/// ```no_run
/// use iris_core::ingestion::discover_paths;
/// use std::path::PathBuf;
///
/// let paths = vec![
///     PathBuf::from("docs/"),
///     PathBuf::from("DESIGN.md"),
///     PathBuf::from("*.md"),
/// ];
/// let files = discover_paths(&paths).unwrap();
/// ```
pub fn discover_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, IngestionError> {
    use std::collections::HashSet;

    let mut all_files = Vec::new();
    let mut seen = HashSet::new();

    for path in paths {
        let path_str = path.to_string_lossy();

        if path_str.contains('*') || path_str.contains('?') || path_str.contains('[') {
            // Glob pattern — expand and classify each match
            let entries = glob::glob(&path_str).map_err(|e| IngestionError::Io {
                path: path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()),
            })?;

            for entry in entries {
                let entry_path = entry.map_err(|e| IngestionError::Io {
                    path: path.clone(),
                    source: std::io::Error::other(e.to_string()),
                })?;
                collect_path_entry(&entry_path, &mut all_files, &mut seen)?;
            }
        } else {
            collect_path_entry(path, &mut all_files, &mut seen)?;
        }
    }

    all_files.sort();
    Ok(all_files)
}

/// Classify a single path as a directory or file and add its discovered files.
fn collect_path_entry(
    path: &Path,
    files: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
) -> Result<(), IngestionError> {
    if path.is_dir() {
        let dir_files = discover_files(path)?;
        for f in dir_files {
            let canonical = f.canonicalize().unwrap_or_else(|_| f.clone());
            if seen.insert(canonical) {
                files.push(f);
            }
        }
    } else if path.is_file() && is_supported_file(path) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if seen.insert(canonical) {
            files.push(path.to_path_buf());
        }
    }
    Ok(())
}

/// Compute a relative path for a file that is unique across all corpus sources.
///
/// Uses the full path relative to the current working directory. This ensures
/// files from different corpus roots (e.g. `iris-core/src/lib.rs` vs
/// `iris-mcp/src/lib.rs`) never collide. Strips only the leading `./` if present.
fn compute_relative_path(file: &Path, _sources: &[PathBuf]) -> String {
    let s = file.to_string_lossy();
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

/// Check if a file has a supported extension.
fn is_supported_file(path: &Path) -> bool {
    detect_parser_kind(path).is_some()
}

/// Compute the SHA-256 hex digest of a string.
#[must_use]
pub fn compute_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Recursively enrich sections with claims and summaries.
///
/// Returns `(total_sections, total_claims)` counts.
fn enrich_sections(
    sections: &mut [Section],
    claim_extractor: &dyn ClaimExtractor,
    summary_generator: &dyn SummaryGenerator,
) -> (usize, usize) {
    let mut section_count = 0;
    let mut claim_count = 0;

    for section in sections.iter_mut() {
        section_count += 1;

        // Extract claims from non-empty section text
        if !section.text.trim().is_empty() {
            let claims = claim_extractor.extract(&section.text, &section.id);
            claim_count += claims.len();
            section.claims = claims;

            // Generate section summary
            let summary = summary_generator.summarize(&section.text, SUMMARY_MAX_SENTENCES);
            if !summary.is_empty() {
                section.summary = Some(summary);
            }
        }

        // Recurse into children
        let (child_sections, child_claims) =
            enrich_sections(&mut section.children, claim_extractor, summary_generator);
        section_count += child_sections;
        claim_count += child_claims;
    }

    (section_count, claim_count)
}

/// Collect all text from sections recursively for document-level summarization.
fn collect_all_text(sections: &[Section]) -> String {
    let mut parts = Vec::new();
    collect_text_recursive(sections, &mut parts);
    parts.join(" ")
}

fn collect_text_recursive(sections: &[Section], parts: &mut Vec<String>) {
    for section in sections {
        if !section.text.trim().is_empty() {
            parts.push(section.text.clone());
        }
        collect_text_recursive(&section.children, parts);
    }
}

/// Split a large headingless section (depth 0) at paragraph boundaries.
///
/// Documents without headings produce a single root section. If that section
/// is very large, we split it at double-newline paragraph boundaries so each
/// chunk gets its own claims and summary.
fn split_large_headingless_section(section: Section, source_path: &str) -> Vec<Section> {
    // Only split depth-0 (implicit root) sections that exceed the threshold
    if section.depth != 0 || section.text.split_whitespace().count() <= PARAGRAPH_SPLIT_THRESHOLD {
        return vec![section];
    }

    let paragraphs: Vec<&str> = section
        .text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();

    if paragraphs.len() <= 1 {
        return vec![section];
    }

    paragraphs
        .into_iter()
        .enumerate()
        .map(|(i, para)| {
            let id_str = format!("{source_path}#paragraph-{i}");
            Section {
                id: crate::types::SectionId(id_str),
                heading_path: Vec::new(),
                depth: 0,
                text: para.to_string(),
                structural_nodes: Vec::new(),
                children: Vec::new(),
                claims: Vec::new(),
                summary: None,
            }
        })
        .collect()
}

/// Coalesce adjacent sibling sections below a minimum token threshold.
///
/// Walks the section tree and merges runs of consecutive siblings at the same
/// depth whose text is below `min_tokens`. Merged sections use the first
/// sibling's section ID and concatenate text with heading markers so child
/// headings remain searchable.
///
/// Set `min_tokens` to `0` to disable merging.
///
/// # Examples
///
/// ```
/// use iris_core::ingestion::coalesce_small_sections;
/// use iris_core::types::{Section, SectionId};
///
/// let sections = vec![
///     Section {
///         id: SectionId("s1".into()),
///         heading_path: vec!["Small A".into()],
///         depth: 2,
///         text: "Tiny.".into(),
///         structural_nodes: vec![],
///         children: vec![],
///         claims: vec![],
///         summary: None,
///     },
///     Section {
///         id: SectionId("s2".into()),
///         heading_path: vec!["Small B".into()],
///         depth: 2,
///         text: "Also tiny.".into(),
///         structural_nodes: vec![],
///         children: vec![],
///         claims: vec![],
///         summary: None,
///     },
/// ];
///
/// let merged = coalesce_small_sections(sections, 50);
/// assert_eq!(merged.len(), 1);
/// assert!(merged[0].text.contains("Small B"));
/// ```
#[must_use]
pub fn coalesce_small_sections(sections: Vec<Section>, min_tokens: usize) -> Vec<Section> {
    if min_tokens == 0 {
        return sections;
    }

    let mut result: Vec<Section> = Vec::new();

    for section in sections {
        let token_count = count_tokens(&section.text);

        if token_count >= min_tokens {
            // Large enough — keep standalone, but recurse into children
            let mut section = section;
            section.children =
                coalesce_small_sections(std::mem::take(&mut section.children), min_tokens);
            result.push(section);
        } else if let Some(prev) = result.last_mut() {
            // Check if previous section is a small sibling at the same depth
            if prev.depth == section.depth && count_tokens(&prev.text) < min_tokens {
                merge_into(prev, section);
            } else {
                // Previous is large or different depth — start potential new run
                let mut section = section;
                section.children =
                    coalesce_small_sections(std::mem::take(&mut section.children), min_tokens);
                result.push(section);
            }
        } else {
            // First section in the list — just push it
            let mut section = section;
            section.children =
                coalesce_small_sections(std::mem::take(&mut section.children), min_tokens);
            result.push(section);
        }
    }

    result
}

/// Merge a small section into an existing accumulator section.
///
/// Appends the source section's text with a heading marker, and merges
/// structural nodes and children.
fn merge_into(target: &mut Section, source: Section) {
    use std::fmt::Write;

    // Add heading marker for the merged section so its heading remains searchable
    let heading = source.heading_path.last().cloned().unwrap_or_default();

    if heading.is_empty() {
        target.text.push_str("\n\n");
    } else {
        let _ = write!(target.text, "\n\n### {heading}\n\n");
    }
    target.text.push_str(&source.text);

    // Merge structural nodes
    target.structural_nodes.extend(source.structural_nodes);

    // Merge children (recurse coalescing is handled by the caller for the target)
    target.children.extend(source.children);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteStorage;
    use crate::storage::traits::Storage;
    use crate::types::SectionId;

    // --- Hash computation ---

    #[test]
    fn sha256_deterministic() {
        let hash1 = compute_sha256("hello world");
        let hash2 = compute_sha256("hello world");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn sha256_different_content() {
        let hash1 = compute_sha256("hello");
        let hash2 = compute_sha256("world");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn sha256_empty_string() {
        let hash = compute_sha256("");
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex chars
    }

    // --- File discovery ---

    #[test]
    fn is_supported_file_accepts_all_formats() {
        assert!(is_supported_file(Path::new("docs/readme.md")));
        assert!(is_supported_file(Path::new("notes.markdown")));
        assert!(is_supported_file(Path::new("test.mkd")));
        assert!(is_supported_file(Path::new("test.mdx")));
        assert!(is_supported_file(Path::new("page.html")));
        assert!(is_supported_file(Path::new("page.htm")));
        assert!(is_supported_file(Path::new("page.xhtml")));
        assert!(is_supported_file(Path::new("manual.pdf")));
        assert!(is_supported_file(Path::new("code.rs")));
        assert!(is_supported_file(Path::new("app.ts")));
        assert!(is_supported_file(Path::new("main.py")));
    }

    #[test]
    fn is_supported_file_rejects_others() {
        assert!(!is_supported_file(Path::new("data.csv")));
        assert!(!is_supported_file(Path::new("readme.txt")));
        assert!(!is_supported_file(Path::new("image.png")));
    }

    #[test]
    fn discover_files_from_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("doc1.md"), "# Hello").unwrap();
        std::fs::write(tmp.path().join("doc2.md"), "# World").unwrap();
        std::fs::write(tmp.path().join("ignore.txt"), "not markdown").unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/nested.md"), "# Nested").unwrap();

        let files = discover_files(tmp.path()).unwrap();
        assert_eq!(files.len(), 3);
    }

    // --- Paragraph-boundary splitting ---

    #[test]
    fn small_headingless_section_not_split() {
        let section = Section {
            id: SectionId("test.md#root".into()),
            heading_path: Vec::new(),
            depth: 0,
            text: "Short paragraph.".into(),
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        };

        let result = split_large_headingless_section(section, "test.md");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn large_headingless_section_split_at_paragraphs() {
        // Create text with many words split across paragraphs
        let para1 = "Word ".repeat(300);
        let para2 = "More ".repeat(300);
        let text = format!("{para1}\n\n{para2}");

        let section = Section {
            id: SectionId("test.md#root".into()),
            heading_path: Vec::new(),
            depth: 0,
            text,
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        };

        let result = split_large_headingless_section(section, "test.md");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id.0, "test.md#paragraph-0");
        assert_eq!(result[1].id.0, "test.md#paragraph-1");
        assert_eq!(result[0].depth, 0);
    }

    #[test]
    fn headed_section_not_split() {
        let para1 = "Word ".repeat(300);
        let para2 = "More ".repeat(300);
        let text = format!("{para1}\n\n{para2}");

        let section = Section {
            id: SectionId("test.md#heading".into()),
            heading_path: vec!["Heading".into()],
            depth: 1,
            text,
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        };

        let result = split_large_headingless_section(section, "test.md");
        assert_eq!(result.len(), 1); // depth != 0, not split
    }

    // --- Section coalescing ---

    fn make_section(id: &str, heading: &str, depth: u32, text: &str) -> Section {
        Section {
            id: SectionId(id.into()),
            heading_path: if heading.is_empty() {
                Vec::new()
            } else {
                vec![heading.into()]
            },
            depth,
            text: text.into(),
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        }
    }

    #[test]
    fn coalesce_three_small_siblings_into_one() {
        // 3 sibling sections each well below 50 tokens
        let sections = vec![
            make_section("s1", "Alpha", 2, "Short text A."),
            make_section("s2", "Beta", 2, "Short text B."),
            make_section("s3", "Gamma", 2, "Short text C."),
        ];

        let result = coalesce_small_sections(sections, 50);
        assert_eq!(result.len(), 1, "3 small siblings should merge into 1");
        assert_eq!(
            result[0].id.0, "s1",
            "merged section uses first sibling's ID"
        );
        assert!(result[0].text.contains("Short text A."));
        assert!(result[0].text.contains("### Beta"));
        assert!(result[0].text.contains("Short text B."));
        assert!(result[0].text.contains("### Gamma"));
        assert!(result[0].text.contains("Short text C."));
    }

    #[test]
    fn coalesce_large_section_stays_untouched() {
        // Generate a section with ~200 tokens worth of text
        let big_text = "The quick brown fox jumps over the lazy dog. ".repeat(30);
        let sections = vec![make_section("s1", "Big", 2, &big_text)];

        let result = coalesce_small_sections(sections, 50);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.0, "s1");
    }

    #[test]
    fn coalesce_mixed_depths_merge_at_each_level() {
        let sections = vec![
            make_section("d1-a", "D1 A", 1, "Small."),
            make_section("d1-b", "D1 B", 1, "Also small."),
            make_section("d2-a", "D2 A", 2, "Tiny."),
            make_section("d2-b", "D2 B", 2, "Also tiny."),
        ];

        let result = coalesce_small_sections(sections, 50);
        // d1-a and d1-b merge; then d2-a and d2-b are separate (different depth from merged d1)
        // d2-a starts as standalone, d2-b merges into d2-a
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].depth, 1);
        assert_eq!(result[1].depth, 2);
    }

    #[test]
    fn coalesce_disabled_with_zero_threshold() {
        let sections = vec![
            make_section("s1", "A", 1, "Tiny."),
            make_section("s2", "B", 1, "Also tiny."),
        ];

        let result = coalesce_small_sections(sections, 0);
        assert_eq!(result.len(), 2, "zero threshold disables merging");
    }

    #[test]
    fn coalesce_preserves_document_order() {
        let sections = vec![
            make_section("s1", "First", 1, "First section."),
            make_section("s2", "Second", 1, "Second section."),
            make_section("s3", "Third", 1, "Third section."),
        ];

        let result = coalesce_small_sections(sections, 50);
        assert_eq!(result.len(), 1);
        // Text should appear in document order
        let first_pos = result[0].text.find("First section.").unwrap();
        let second_pos = result[0].text.find("Second section.").unwrap();
        let third_pos = result[0].text.find("Third section.").unwrap();
        assert!(first_pos < second_pos);
        assert!(second_pos < third_pos);
    }

    #[test]
    fn coalesce_small_between_large_stays_separate() {
        let big_text = "The quick brown fox jumps over the lazy dog. ".repeat(30);
        let sections = vec![
            make_section("big1", "Big 1", 1, &big_text),
            make_section("small", "Small", 1, "Tiny."),
            make_section("big2", "Big 2", 1, &big_text),
        ];

        let result = coalesce_small_sections(sections, 50);
        // big1 stays, small can't merge with big1 (big1 is large), small stays alone,
        // big2 is large so stays alone
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn coalesce_recurses_into_children() {
        let parent = Section {
            id: SectionId("parent".into()),
            heading_path: vec!["Parent".into()],
            depth: 1,
            text: "The quick brown fox jumps over the lazy dog. ".repeat(30),
            structural_nodes: Vec::new(),
            children: vec![
                make_section("child1", "Child A", 2, "Small child A."),
                make_section("child2", "Child B", 2, "Small child B."),
            ],
            claims: Vec::new(),
            summary: None,
        };

        let result = coalesce_small_sections(vec![parent], 50);
        assert_eq!(result.len(), 1);
        // Children should have been coalesced
        assert_eq!(
            result[0].children.len(),
            1,
            "two small children should merge into one"
        );
    }

    // --- Section enrichment ---

    #[test]
    fn enrich_sections_adds_claims_and_summaries() {
        let mut sections = vec![Section {
            id: SectionId("test#s1".into()),
            heading_path: vec!["Test".into()],
            depth: 1,
            text: "The API uses JWT tokens with RS256 signing. Rate limits are 100 requests per minute.".into(),
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        }];

        let extractor = HeuristicClaimExtractor::new();
        let summarizer = ExtractiveSummaryGenerator::new();
        let (sec_count, claim_count) = enrich_sections(&mut sections, &extractor, &summarizer);

        assert_eq!(sec_count, 1);
        assert!(claim_count > 0);
        assert!(!sections[0].claims.is_empty());
        assert!(sections[0].summary.is_some());
    }

    #[test]
    fn enrich_empty_text_section_no_claims() {
        let mut sections = vec![Section {
            id: SectionId("test#empty".into()),
            heading_path: vec!["Empty".into()],
            depth: 1,
            text: "   ".into(), // whitespace only
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        }];

        let extractor = HeuristicClaimExtractor::new();
        let summarizer = ExtractiveSummaryGenerator::new();
        let (_, claim_count) = enrich_sections(&mut sections, &extractor, &summarizer);

        assert_eq!(claim_count, 0);
        assert!(sections[0].claims.is_empty());
        assert!(sections[0].summary.is_none());
    }

    #[test]
    fn enrich_nested_sections() {
        let mut sections = vec![Section {
            id: SectionId("test#parent".into()),
            heading_path: vec!["Parent".into()],
            depth: 1,
            text: "The parent section provides an overview of the system architecture.".into(),
            structural_nodes: Vec::new(),
            children: vec![Section {
                id: SectionId("test#child".into()),
                heading_path: vec!["Parent".into(), "Child".into()],
                depth: 2,
                text: "The child section implements authentication with OAuth2 and JWT tokens."
                    .into(),
                structural_nodes: Vec::new(),
                children: Vec::new(),
                claims: Vec::new(),
                summary: None,
            }],
            claims: Vec::new(),
            summary: None,
        }];

        let extractor = HeuristicClaimExtractor::new();
        let summarizer = ExtractiveSummaryGenerator::new();
        let (sec_count, _) = enrich_sections(&mut sections, &extractor, &summarizer);

        assert_eq!(sec_count, 2); // parent + child
    }

    // --- Full pipeline integration tests ---

    #[tokio::test]
    async fn ingest_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("test.md"),
            "# API Reference\n\n\
             The auth service uses JWT tokens with RS256 signing.\n\n\
             ## Rate Limits\n\n\
             Rate limits are 100 requests per minute per API key.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_discovered, 1);
        assert_eq!(stats.files_indexed, 1);
        assert_eq!(stats.files_skipped, 0);
        assert!(stats.total_sections > 0);

        // Verify stored in database
        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "API Reference");
        assert!(docs[0].summary.is_some());
    }

    #[tokio::test]
    async fn incremental_reindex_skips_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("doc.md"),
            "# Hello\n\nThe world is round.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();

        // First ingestion
        let stats1 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats1.files_indexed, 1);

        // Second ingestion — same content
        let stats2 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats2.files_skipped, 1);
        assert_eq!(stats2.files_indexed, 0);
    }

    #[tokio::test]
    async fn incremental_reindex_updates_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("doc.md");
        std::fs::write(&file_path, "# V1\n\nOriginal content.\n").unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();

        // First ingestion
        let stats1 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats1.files_indexed, 1);

        // Modify file
        std::fs::write(
            &file_path,
            "# V2\n\nUpdated content with new information.\n",
        )
        .unwrap();

        // Second ingestion — changed content
        let stats2 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats2.files_indexed, 1);
        assert_eq!(stats2.files_skipped, 0);

        // Verify updated in database
        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "V2");
    }

    #[tokio::test]
    async fn incremental_reindex_removes_deleted_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("keep.md"), "# Keep\n\nThis file stays.\n").unwrap();
        std::fs::write(
            tmp.path().join("remove.md"),
            "# Remove\n\nThis file will be deleted.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();

        // First ingestion
        let stats1 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats1.files_indexed, 2);

        // Delete one file
        std::fs::remove_file(tmp.path().join("remove.md")).unwrap();

        // Second ingestion
        let stats2 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats2.files_removed, 1);

        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "Keep");
    }

    #[tokio::test]
    async fn ingest_document_without_headings() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("plain.md"),
            "Just a plain paragraph.\n\nAnother paragraph here.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_indexed, 1);
        assert!(stats.total_sections >= 1);

        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
    }

    #[tokio::test]
    async fn ingest_empty_document() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("empty.md"), "").unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_indexed, 1);
        assert_eq!(stats.total_sections, 0);
    }

    #[tokio::test]
    async fn ingest_document_with_nested_lists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("lists.md"),
            "# Configuration\n\n\
             The system supports the following options:\n\n\
             - Option A: enables feature X\n\
             - Option B: configures timeout to 30 seconds\n\
             - Option C: sets the maximum retry count to 5\n\n\
             1. First step: initialize the database\n\
             2. Second step: run migrations\n\
             3. Third step: start the server on port 8080\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_indexed, 1);
        assert!(stats.total_sections >= 1);
    }

    #[tokio::test]
    async fn ingest_empty_directory() {
        let tmp = tempfile::tempdir().unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_discovered, 0);
        assert_eq!(stats.files_indexed, 0);
    }

    #[tokio::test]
    async fn ingest_multiple_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("api.md"),
            "# API\n\nThe API uses REST over HTTPS.\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("guide.md"),
            "# Guide\n\nThe guide covers installation and configuration.\n",
        )
        .unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(
            tmp.path().join("sub/advanced.md"),
            "# Advanced\n\nAdvanced topics include clustering and replication.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_discovered, 3);
        assert_eq!(stats.files_indexed, 3);

        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 3);
    }

    // --- Embedding ingestion tests ---

    /// Deterministic mock embedder for testing (no model download needed).
    struct MockEmbedder {
        dim: usize,
    }

    impl crate::embedding::Embedder for MockEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, crate::error::IndexError> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += f32::from(b) / 255.0;
                    }
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if norm > 0.0 {
                        for x in &mut v {
                            *x /= norm;
                        }
                    }
                    v
                })
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    fn make_mock_embedder_and_index() -> (MockEmbedder, crate::index::HnswIndex) {
        let dim = 8;
        let embedder = MockEmbedder { dim };
        let index = crate::index::HnswIndex::new(dim, 10_000).unwrap();
        (embedder, index)
    }

    #[tokio::test]
    async fn ingest_with_embeddings_creates_vectors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("test.md"),
            "# API Reference\n\n\
             The auth service uses JWT tokens with RS256 signing.\n\n\
             ## Rate Limits\n\n\
             Rate limits are 100 requests per minute per API key.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        let stats = pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        assert_eq!(stats.files_indexed, 1);
        assert!(stats.total_sections > 0);

        // Vector index should have embeddings
        assert!(!index.is_empty());

        // Should have doc summary + section summaries + section texts + claims
        let vec_count = index.len();
        assert!(
            vec_count >= 3,
            "expected at least 3 vectors, got {vec_count}"
        );
    }

    #[tokio::test]
    async fn embedding_ingestion_skips_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("doc.md"),
            "# Hello\n\nThe world is round.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        // First ingestion
        pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        let count_after_first = index.len();

        // Second ingestion — same content
        let stats2 = pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        assert_eq!(stats2.files_skipped, 1);
        assert_eq!(stats2.files_indexed, 0);
        // Vector count should not change
        assert_eq!(index.len(), count_after_first);
    }

    #[tokio::test]
    async fn embedding_ingestion_updates_changed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("doc.md");
        std::fs::write(
            &file_path,
            "# V1\n\nOriginal content about authentication.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        // First ingestion
        pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        let count_v1 = index.len();

        // Modify file with more sections
        std::fs::write(
            &file_path,
            "# V2\n\nUpdated content.\n\n## New Section\n\nNew information about rate limits.\n",
        )
        .unwrap();

        // Second ingestion — should delete old vectors and insert new ones
        pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        // Should have vectors (old ones deleted, new ones inserted)
        assert!(!index.is_empty());
        // V2 has more sections, so likely more vectors
        assert!(index.len() >= count_v1);
    }

    #[tokio::test]
    async fn embedding_ingestion_removes_deleted_file_vectors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("keep.md"),
            "# Keep\n\nThis file stays in the index.\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("remove.md"),
            "# Remove\n\nThis file will be deleted from the index.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        // First ingestion
        pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        let count_before = index.len();
        assert!(count_before > 0);

        // Delete one file
        std::fs::remove_file(tmp.path().join("remove.md")).unwrap();

        // Second ingestion — should remove vectors for deleted file
        let stats2 = pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        assert_eq!(stats2.files_removed, 1);
        // Should have fewer vectors now
        assert!(index.len() < count_before);
    }

    #[tokio::test]
    async fn embed_document_creates_multi_resolution_vectors() {
        let doc = crate::types::DocumentTree {
            id: crate::types::ContentId("doc1".into()),
            title: "Test".into(),
            source_path: "test.md".into(),
            sections: vec![crate::types::Section {
                id: SectionId("test.md#s1".into()),
                heading_path: vec!["Section One".into()],
                depth: 1,
                text: "The authentication system uses JWT tokens.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![crate::types::Claim {
                    id: crate::types::ClaimId("c1".into()),
                    text: "JWT tokens use RS256 signing.".into(),
                    section_id: SectionId("test.md#s1".into()),
                }],
                summary: Some("Auth system overview.".into()),
            }],
            summary: Some("Document about authentication.".into()),
        };

        let (embedder, index) = make_mock_embedder_and_index();
        let count = embed_document(&doc, &embedder, &index).unwrap();

        // Should have: doc summary + sec summary + section text + claim = 4
        assert_eq!(count, 4);
        assert_eq!(index.len(), 4);

        // Verify specific vector IDs exist via KNN search returning all vectors.
        // Use a large k to ensure all vectors are returned regardless of HNSW
        // graph connectivity with very small indexes.
        let query = embedder
            .embed(&["auth"])
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let results = index.search_knn(&query, 10).unwrap();
        let result_ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();

        // With only 4 vectors, KNN(10) should return all of them.
        // If HNSW graph connectivity causes fewer results, at minimum
        // verify the count and check that expected ID prefixes are present.
        assert!(
            results.len() >= 3,
            "expected at least 3 of 4 vectors from KNN search, got {}",
            results.len()
        );
        assert!(result_ids.contains(&"doc-summary::doc1"));
        assert!(result_ids.contains(&"sec-summary::test.md#s1"));
        assert!(result_ids.contains(&"section::test.md#s1"));
        // The claim vector may not always appear in KNN results with tiny HNSW
        // graphs due to graph connectivity. Verify it was inserted via count.
        assert_eq!(index.len(), 4, "all 4 vectors should be in the index");
    }

    // --- Integration test: coalescing reduces section count ---

    #[tokio::test]
    async fn coalescing_reduces_section_count_in_ingestion() {
        let tmp = tempfile::tempdir().unwrap();
        // Document with many small sections that should be coalesced
        std::fs::write(
            tmp.path().join("fragmented.md"),
            "# Guide\n\n\
             ## A\n\nTiny.\n\n\
             ## B\n\nAlso tiny.\n\n\
             ## C\n\nStill tiny.\n\n\
             ## Big Section\n\n\
             This section has much more content that should keep it standalone. \
             It contains detailed information about the system architecture, \
             including multiple paragraphs of explanation covering authentication, \
             authorization, rate limiting, caching strategies, database design, \
             and deployment considerations for the production environment.\n",
        )
        .unwrap();

        let storage_merged = SqliteStorage::open_in_memory().unwrap();
        let pipeline_merged = IngestionPipeline::new(); // default: 50 tokens

        let stats_merged = pipeline_merged
            .ingest_directory(tmp.path(), &storage_merged)
            .await
            .unwrap();

        let storage_unmerged = SqliteStorage::open_in_memory().unwrap();
        let pipeline_unmerged = IngestionPipeline::new().with_min_section_tokens(0);

        let stats_unmerged = pipeline_unmerged
            .ingest_directory(tmp.path(), &storage_unmerged)
            .await
            .unwrap();

        assert!(
            stats_merged.total_sections < stats_unmerged.total_sections,
            "merged section count ({}) should be less than unmerged ({})",
            stats_merged.total_sections,
            stats_unmerged.total_sections,
        );
    }

    // --- Multi-path discovery ---

    #[test]
    fn discover_paths_with_mixed_dirs_and_files() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a directory with files
        let docs_dir = tmp.path().join("docs");
        std::fs::create_dir(&docs_dir).unwrap();
        std::fs::write(docs_dir.join("guide.md"), "# Guide").unwrap();
        std::fs::write(docs_dir.join("api.md"), "# API").unwrap();
        std::fs::write(docs_dir.join("ignore.txt"), "not supported").unwrap();

        // Create an individual file outside the directory
        std::fs::write(tmp.path().join("DESIGN.md"), "# Design").unwrap();

        let paths = vec![docs_dir.clone(), tmp.path().join("DESIGN.md")];

        let files = discover_paths(&paths).unwrap();

        assert_eq!(
            files.len(),
            3,
            "should discover 2 from docs/ + 1 individual file, got: {files:?}"
        );

        // Verify no .txt files included
        for f in &files {
            assert!(is_supported_file(f), "unsupported file included: {f:?}");
        }
    }

    #[test]
    fn discover_paths_deduplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let docs_dir = tmp.path().join("docs");
        std::fs::create_dir(&docs_dir).unwrap();
        std::fs::write(docs_dir.join("guide.md"), "# Guide").unwrap();

        // Pass the same directory twice
        let paths = vec![docs_dir.clone(), docs_dir];

        let files = discover_paths(&paths).unwrap();
        assert_eq!(files.len(), 1, "duplicates should be removed");
    }

    #[test]
    fn discover_paths_with_glob_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("readme.md"), "# Readme").unwrap();
        std::fs::write(tmp.path().join("design.md"), "# Design").unwrap();
        std::fs::write(tmp.path().join("code.rs"), "fn main() {}").unwrap();

        let glob_pattern = tmp.path().join("*.md");
        let paths = vec![glob_pattern];

        let files = discover_paths(&paths).unwrap();
        assert_eq!(
            files.len(),
            2,
            "glob should match 2 .md files, got: {files:?}"
        );
    }

    #[test]
    fn discover_paths_empty_input() {
        let files = discover_paths(&[]).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn compute_relative_path_preserves_full_path() {
        let sources = vec![PathBuf::from("./docs"), PathBuf::from("./src")];

        // Files keep their full path relative to CWD (minus leading ./)
        let rel = compute_relative_path(Path::new("./docs/guide.md"), &sources);
        assert_eq!(rel, "docs/guide.md");

        let rel = compute_relative_path(Path::new("./src/lib.rs"), &sources);
        assert_eq!(rel, "src/lib.rs");
    }

    #[test]
    fn compute_relative_path_no_collision_across_crates() {
        let sources = vec![
            PathBuf::from("./iris-core/src"),
            PathBuf::from("./iris-mcp/src"),
        ];

        let rel1 = compute_relative_path(Path::new("./iris-core/src/lib.rs"), &sources);
        let rel2 = compute_relative_path(Path::new("./iris-mcp/src/lib.rs"), &sources);
        assert_ne!(rel1, rel2, "paths from different crates must not collide");
        assert_eq!(rel1, "iris-core/src/lib.rs");
        assert_eq!(rel2, "iris-mcp/src/lib.rs");
    }

    // --- C6.2: E2E unified code + doc search ---

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn e2e_unified_code_and_doc_search() {
        use crate::search::{MultiResolutionSearch, SearchConfig};
        use crate::types::Resolution;

        let tmp = tempfile::tempdir().unwrap();

        // Write a markdown doc about ingestion
        std::fs::write(
            tmp.path().join("ingestion.md"),
            "# Ingestion Pipeline\n\n\
             The ingestion pipeline processes files from a directory.\n\
             It hashes each file and skips unchanged content.\n\n\
             ## Parsing\n\n\
             Files are parsed into document trees with sections and claims.\n",
        )
        .unwrap();

        // Write a Rust source file with symbols about ingestion
        std::fs::write(
            tmp.path().join("pipeline.rs"),
            r"//! Ingestion pipeline orchestrator.

/// Processes files and indexes their content.
pub struct IngestionPipeline {
    /// Minimum section token threshold.
    pub min_tokens: usize,
}

impl IngestionPipeline {
    /// Create a new pipeline with defaults.
    pub fn new() -> Self {
        Self { min_tokens: 50 }
    }

    /// Ingest all files from a directory.
    pub fn ingest(&self, dir: &str) -> usize {
        42
    }
}

/// Hash the content of a file for change detection.
pub fn compute_hash(content: &str) -> String {
    content.len().to_string()
}
",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        let stats = pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        // Both files should be indexed
        assert_eq!(stats.files_failed, 0, "no files should fail: {stats:?}");
        assert_eq!(stats.files_indexed, 2);

        // Vector index should contain both doc sections AND symbol vectors
        let total_vectors = index.len();
        // Doc vectors: doc-summary + sec-summary(s) + section(s) + claims
        // Symbol vectors: symbol-stub + symbol-full for each symbol
        assert!(
            total_vectors >= 6,
            "expected at least 6 vectors (doc + symbol), got {total_vectors}"
        );

        // Search for "ingestion pipeline" — should return both doc and code results
        let searcher = MultiResolutionSearch::new(&embedder, &index);
        let config = SearchConfig {
            raw_k: 30,
            top_k: 10,
            sparse_weight: 0.0,
            rerank_top_k: None,
        };
        let results = searcher.search("ingestion pipeline", config).unwrap();

        assert!(
            !results.is_empty(),
            "search should return results for 'ingestion pipeline'"
        );

        // Verify we get both doc-level and symbol-level results
        let has_doc_result = results.iter().any(|r| {
            matches!(
                r.resolution,
                Resolution::Summary | Resolution::Section | Resolution::Claim
            )
        });
        let has_symbol_result = results.iter().any(|r| {
            matches!(
                r.resolution,
                Resolution::SymbolStub | Resolution::SymbolFull
            )
        });

        assert!(
            has_doc_result,
            "search results should include document sections"
        );
        assert!(
            has_symbol_result,
            "search results should include code symbols"
        );

        // Verify symbols were stored in SQLite
        let symbols = storage
            .list_symbols(&crate::storage::SymbolFilter {
                file_path: Some("pipeline.rs".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            symbols.len() >= 2,
            "expected at least 2 symbols (struct + impl/fn), got {}: {:?}",
            symbols.len(),
            symbols
                .iter()
                .map(|s| format!("{} {}", s.kind, s.name))
                .collect::<Vec<_>>()
        );

        // Verify symbol stubs have correct format in vector IDs
        let pipeline_struct = symbols
            .iter()
            .find(|s| s.name == "IngestionPipeline" && s.kind != "impl")
            .unwrap_or_else(|| {
                panic!(
                    "should have IngestionPipeline struct, found: {:?}",
                    symbols
                        .iter()
                        .map(|s| format!("{}:{}", s.kind, s.name))
                        .collect::<Vec<_>>()
                )
            });
        let stub_vid = VectorId::symbol_stub(pipeline_struct.id.as_ref());
        assert_eq!(stub_vid.resolution(), Resolution::SymbolStub);
    }

    // --- IngestionProgress ---

    #[test]
    fn progress_lifecycle() {
        let progress = IngestionProgress::new();
        assert_eq!(progress.status(), 0);
        assert!(!progress.is_running());

        progress.start(10);
        assert!(progress.is_running());
        assert_eq!(progress.files_total(), 10);
        assert_eq!(progress.files_done(), 0);

        for _ in 0..5 {
            progress.increment_done();
        }
        assert_eq!(progress.files_done(), 5);
        assert!(progress.is_running());

        progress.complete();
        assert!(!progress.is_running());
        assert_eq!(progress.status(), 2);
    }

    #[test]
    fn progress_default() {
        let progress = IngestionProgress::default();
        assert_eq!(progress.status(), 0);
        assert_eq!(progress.files_total(), 0);
        assert_eq!(progress.files_done(), 0);
    }

    // --- resolve_and_store_refs hardening ---

    #[tokio::test]
    async fn resolve_refs_prefers_mod_anchor_for_imports() {
        let storage = SqliteStorage::open_in_memory().unwrap();

        // Insert symbols: a mod symbol and a struct symbol in the same file
        let mod_sym = SymbolRecord {
            id: SymbolId::from("sym-test.rs::test_mod".to_string()),
            file_path: "test.rs".to_string(),
            name: "test_mod".to_string(),
            kind: "mod".to_string(),
            visibility: "pub".to_string(),
            module_path: String::new(),
            line_start: 1,
            line_end: 1,
            signature: String::new(),
            doc_comment: None,
            cyclomatic_complexity: None,
        };
        let struct_sym = SymbolRecord {
            id: SymbolId::from("sym-test.rs::MyStruct".to_string()),
            file_path: "test.rs".to_string(),
            name: "MyStruct".to_string(),
            kind: "struct".to_string(),
            visibility: "pub".to_string(),
            module_path: String::new(),
            line_start: 5,
            line_end: 10,
            signature: String::new(),
            doc_comment: None,
            cyclomatic_complexity: None,
        };
        // A target symbol in another file
        let target_sym = SymbolRecord {
            id: SymbolId::from("sym-other.rs::OtherType".to_string()),
            file_path: "other.rs".to_string(),
            name: "OtherType".to_string(),
            kind: "struct".to_string(),
            visibility: "pub".to_string(),
            module_path: String::new(),
            line_start: 1,
            line_end: 5,
            signature: String::new(),
            doc_comment: None,
            cyclomatic_complexity: None,
        };

        storage
            .insert_symbols(&[mod_sym.clone(), struct_sym.clone(), target_sym])
            .await
            .unwrap();

        // Parse a Rust file with a `use` import
        let source = b"use crate::OtherType;\n\npub struct MyStruct {}\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();

        let local_symbols = vec![mod_sym.clone(), struct_sym];
        let inserted =
            resolve_and_store_refs(&tree, source, "test.rs", "rust", &local_symbols, &storage)
                .await
                .unwrap();

        assert_eq!(inserted, 1, "should resolve one import ref");

        // Verify the ref uses the mod symbol as the anchor
        let refs = storage.query_refs(&mod_sym.id, None).await.unwrap();
        assert!(
            !refs.is_empty(),
            "the mod symbol should be the from_symbol in the ref"
        );
    }
}
