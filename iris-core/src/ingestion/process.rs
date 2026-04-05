//! Shared per-document processing core.
//!
//! The four ingestion entry points all run the same sequence:
//! split → coalesce → deduplicate → enrich → summarize → delete old → insert → relationships → hash.
//!
//! This module extracts that shared pipeline into [`store_enriched_document`] so each
//! entry point is a thin wrapper that handles I/O differences (file vs content,
//! immediate vs deferred embedding) and delegates the core work here.

use tracing::debug;

use crate::error::IngestionError;
use crate::extraction::claims::ClaimExtractor;
use crate::extraction::relationships::RelationshipDetector;
use crate::extraction::summary::SummaryGenerator;
use crate::index::VectorIndex;
use crate::storage::traits::{FileHashRecord, Storage};
use crate::types::DocumentTree;

use super::embedding::delete_document_vectors;
use super::sections::{
    coalesce_small_sections, collect_all_claims, collect_all_text, enrich_sections,
    split_large_headingless_section,
};

/// Maximum number of sentences in a document-level summary.
const DOC_SUMMARY_MAX_SENTENCES: usize = 5;

/// Result of the shared document processing pipeline.
pub(super) struct ProcessedDocument {
    pub section_count: usize,
    pub claim_count: usize,
}

/// Options controlling what the shared pipeline does beyond the core sequence.
pub(super) struct ProcessOptions<'a> {
    /// Path key for the file hash record. `None` skips hash storage.
    pub hash_path: Option<&'a str>,
    /// Content hash to store. Required if `hash_path` is set.
    pub content_hash: Option<String>,
    /// File mtime in nanos. Stored alongside the hash when available.
    pub mtime_ns: Option<i64>,
}

/// The shared document processing pipeline.
///
/// Takes a parsed `DocumentTree` and runs the full enrichment + storage sequence:
/// 1. Split large headingless sections at paragraph boundaries
/// 2. Coalesce small adjacent sections
/// 3. Deduplicate section/claim IDs (fixes UNIQUE constraint cascades)
/// 4. Enrich sections with claims and summaries
/// 5. Generate document-level summary
/// 6. Delete old document (+ vectors if index provided)
/// 7. Insert enriched document
/// 8. Detect and store claim relationships
/// 9. Update file hash record
#[allow(clippy::too_many_arguments)] // domain orchestration — needs all inputs
pub(super) async fn store_enriched_document<S, I>(
    doc: &mut DocumentTree,
    source_path: &str,
    storage: &S,
    claim_extractor: &dyn ClaimExtractor,
    summary_generator: &dyn SummaryGenerator,
    relationship_detector: &dyn RelationshipDetector,
    min_section_tokens: usize,
    existing_hash: bool,
    delete_vectors_index: Option<&I>,
    opts: ProcessOptions<'_>,
) -> Result<ProcessedDocument, IngestionError>
where
    S: Storage + ?Sized,
    I: VectorIndex + ?Sized,
{
    // 1. Split large headingless sections
    doc.sections = doc
        .sections
        .drain(..)
        .flat_map(|s| split_large_headingless_section(s, source_path))
        .collect();

    // 2. Coalesce small adjacent sections
    doc.sections = coalesce_small_sections(
        std::mem::take(&mut doc.sections),
        min_section_tokens,
    );

    // 3. Deduplicate section/claim IDs to prevent UNIQUE constraint violations
    doc.deduplicate_ids();

    // 4. Enrich sections with claims and summaries
    let (section_count, claim_count) =
        enrich_sections(&mut doc.sections, claim_extractor, summary_generator);

    // 5. Generate document-level summary
    let all_text = collect_all_text(&doc.sections);
    if !all_text.is_empty() {
        doc.summary = Some(summary_generator.summarize(&all_text, DOC_SUMMARY_MAX_SENTENCES));
    }

    // 6. Delete old document (+ vectors if re-indexing)
    if existing_hash {
        if let Some(index) = delete_vectors_index {
            delete_document_vectors(&doc.id, storage, index).await?;
        }
        storage
            .delete_document(&doc.id)
            .await
            .map_err(IngestionError::from)?;
    }

    // 7. Insert enriched document
    storage
        .insert_document(doc)
        .await
        .map_err(IngestionError::from)?;

    // 8. Detect and store claim relationships
    let all_claims = collect_all_claims(&doc.sections);
    if all_claims.len() >= 2 {
        let relationships = relationship_detector.detect(&all_claims);
        if !relationships.is_empty() {
            debug!(
                path = %source_path,
                count = relationships.len(),
                "detected claim relationships"
            );
            storage
                .insert_claim_relationships(&relationships)
                .await
                .map_err(IngestionError::from)?;
        }
    }

    // 9. Update file hash record
    if let (Some(hash_path), Some(content_hash)) = (opts.hash_path, opts.content_hash) {
        storage
            .upsert_file_hash(&FileHashRecord {
                path: hash_path.to_string(),
                content_hash,
                mtime_ns: opts.mtime_ns,
            })
            .await
            .map_err(IngestionError::from)?;
    }

    Ok(ProcessedDocument {
        section_count,
        claim_count,
    })
}
