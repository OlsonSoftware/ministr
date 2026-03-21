//! Ingestion pipeline orchestrator.
//!
//! Coordinates the full document ingestion flow: file discovery, content hashing
//! for incremental re-indexing, parsing, claim extraction, summarization, and
//! storage. The pipeline is generic over the parser and storage implementations.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tracing::{debug, info, instrument, warn};

use crate::error::IngestionError;
use crate::extraction::claims::{ClaimExtractor, HeuristicClaimExtractor};
use crate::extraction::summary::{ExtractiveSummaryGenerator, SummaryGenerator};
use crate::parser::{DocumentParser, MarkdownParser};
use crate::storage::traits::{FileHashRecord, Storage};
use crate::types::Section;

/// Result of ingesting a corpus directory.
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
    parser: MarkdownParser,
    claim_extractor: HeuristicClaimExtractor,
    summary_generator: ExtractiveSummaryGenerator,
}

impl IngestionPipeline {
    /// Create a new ingestion pipeline with default components.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parser: MarkdownParser::new(),
            claim_extractor: HeuristicClaimExtractor::new(),
            summary_generator: ExtractiveSummaryGenerator::new(),
        }
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
        };

        info!(count = files.len(), "discovered files for ingestion");

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

        // Parse the document
        let mut doc = self.parser.parse(Path::new(relative_path), &content_str)?;

        // Handle paragraph-boundary splitting for large headingless sections
        doc.sections = doc
            .sections
            .into_iter()
            .flat_map(|s| split_large_headingless_section(s, relative_path))
            .collect();

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

/// Discover all supported files (`.md`) in a directory recursively.
fn discover_files(dir: &Path) -> Result<Vec<PathBuf>, IngestionError> {
    let mut files = Vec::new();
    collect_files_recursive(dir, &mut files)?;
    files.sort();
    Ok(files)
}

/// Recursively collect supported files from a directory.
fn collect_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), IngestionError> {
    let entries = std::fs::read_dir(dir).map_err(|e| IngestionError::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| IngestionError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();

        if path.is_dir() {
            collect_files_recursive(&path, files)?;
        } else if is_supported_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

/// Check if a file has a supported extension.
fn is_supported_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "markdown" | "mkd" | "mdx")
    )
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
    fn is_supported_file_md() {
        assert!(is_supported_file(Path::new("docs/readme.md")));
        assert!(is_supported_file(Path::new("notes.markdown")));
        assert!(is_supported_file(Path::new("test.mkd")));
        assert!(is_supported_file(Path::new("test.mdx")));
    }

    #[test]
    fn is_supported_file_rejects_others() {
        assert!(!is_supported_file(Path::new("code.rs")));
        assert!(!is_supported_file(Path::new("data.json")));
        assert!(!is_supported_file(Path::new("readme.txt")));
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
}
