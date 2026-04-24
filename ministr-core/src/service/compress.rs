//! Content compression pipeline for [`QueryService`].
//!
//! Provides extractive, auto-tier, and abstractive compression of sections
//! and symbols for context budget management.

use tracing::instrument;

use crate::extraction::abstractive::AbstractiveCompressor;
use crate::extraction::claims::{ClaimExtractor, HeuristicClaimExtractor};
use crate::extraction::strategy::{AutoCompressor, CompressStrategy, ExtractiveStrategy};
use crate::extraction::summary::{ExtractiveSummaryGenerator, SummaryGenerator};
use crate::storage::Storage;
use crate::token::count_tokens;
use crate::types::{SectionId, SymbolId};

use super::{CompressedItem, QueryError, QueryService};

impl QueryService {
    /// Compress content items into shorter summaries for eviction.
    ///
    /// Accepts three content-ID shapes — the same ones [`QueryService`]
    /// hands back to callers via survey / extract / symbols:
    ///
    /// - Section IDs (e.g. `…/foo.rs#mod::Bar`) → compressed with `strategy`.
    /// - Claim IDs (section with a `:cN` suffix) → transparently rewritten to
    ///   the parent section ID and compressed. The returned
    ///   [`CompressedItem::original_id`] preserves the caller's claim ID so
    ///   input ↔ output correlation still works.
    /// - Symbol IDs (`sym-…`) → compressed via a symbol-stub summary.
    ///
    /// Content IDs that don't match any of the above are silently skipped,
    /// as are sections whose compressed form doesn't shrink.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    #[instrument(skip(self, strategy))]
    pub async fn compress_content_with(
        &self,
        content_ids: &[String],
        strategy: &dyn CompressStrategy,
    ) -> Result<Vec<CompressedItem>, QueryError> {
        let mut results = Vec::with_capacity(content_ids.len());

        for id in content_ids {
            let section_lookup_id = strip_claim_suffix(id);
            if let Some(mut item) = self
                .try_compress_section_with(section_lookup_id, strategy)
                .await?
            {
                // Preserve the caller's original ID (e.g. a claim ID) so
                // they can correlate this result with the ID they asked
                // about, even when we compressed the parent section.
                if section_lookup_id != id.as_str() {
                    item.original_id.clone_from(id);
                }
                results.push(item);
            } else if let Some(item) = self.try_compress_symbol(id).await? {
                results.push(item);
            }
        }

        Ok(results)
    }

    /// Compress content items using the default extractive strategy.
    ///
    /// Convenience wrapper around [`compress_content_with`] using TF-IDF
    /// extractive summarization (2 sentences).
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    #[instrument(skip(self))]
    pub async fn compress_content(
        &self,
        content_ids: &[String],
    ) -> Result<Vec<CompressedItem>, QueryError> {
        self.compress_content_with(content_ids, &ExtractiveStrategy::default())
            .await
    }

    /// Compress content with auto-tier selection based on content type.
    ///
    /// Code → symbol summary, Documentation → extractive TF-IDF, Claims → skip.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    #[instrument(skip(self))]
    pub async fn compress_content_auto(
        &self,
        content_ids: &[String],
    ) -> Result<Vec<CompressedItem>, QueryError> {
        let auto = AutoCompressor::default();
        let mut results = Vec::with_capacity(content_ids.len());

        for id in content_ids {
            // For symbols, always use symbol stub compression
            if id.starts_with("sym-") {
                if let Some(item) = self.try_compress_symbol(id).await? {
                    results.push(item);
                }
                continue;
            }

            // For sections, use auto-tier
            let sid = SectionId(id.clone());
            if let Some(section) = self.storage.get_section(&sid).await? {
                let original_tokens = count_tokens(&section.text);

                if let Some((summary, method)) = auto.compress_auto(id, &section.text, 2) {
                    let compressed_tokens = count_tokens(&summary);
                    if compressed_tokens < original_tokens {
                        results.push(CompressedItem {
                            original_id: id.clone(),
                            summary,
                            original_tokens,
                            compressed_tokens,
                            method: method.to_string(),
                        });
                    }
                }
            }
        }

        Ok(results)
    }

    /// Try to compress a section using the given strategy.
    /// Returns `None` if not found or if compression achieves no reduction.
    pub(super) async fn try_compress_section_with(
        &self,
        id: &str,
        strategy: &dyn CompressStrategy,
    ) -> Result<Option<CompressedItem>, QueryError> {
        let sid = SectionId(id.to_string());
        let Some(section) = self.storage.get_section(&sid).await? else {
            return Ok(None);
        };

        let summary = section
            .summary
            .or_else(|| strategy.compress(&section.text, 2))
            .unwrap_or_default();

        if summary.is_empty() {
            return Ok(None);
        }

        let original_tokens = count_tokens(&section.text);
        let compressed_tokens = count_tokens(&summary);

        if compressed_tokens >= original_tokens {
            return Ok(None);
        }

        Ok(Some(CompressedItem {
            original_id: id.to_string(),
            summary,
            original_tokens,
            compressed_tokens,
            method: strategy.method_name().to_string(),
        }))
    }

    /// Try to compress a symbol by its ID. Generates a compact summary
    /// from the symbol's signature and doc comment.
    pub(super) async fn try_compress_symbol(
        &self,
        id: &str,
    ) -> Result<Option<CompressedItem>, QueryError> {
        if !id.starts_with("sym-") {
            return Ok(None);
        }

        let sid = SymbolId(id.to_string());
        let Some(symbol) = self.storage.get_symbol(&sid).await? else {
            return Ok(None);
        };

        // Build a compact representation from signature + doc summary
        let mut summary = symbol.signature.clone();
        if let Some(ref doc) = symbol.doc_comment {
            // Take just the first sentence of the doc comment
            let first_sentence = doc
                .split_once(". ")
                .map_or(doc.as_str(), |(first, _)| first);
            summary = format!("/// {first_sentence}\n{summary}");
        }

        // Estimate original size from the full source context
        let original_text = self
            .read_source_context(&symbol.file_path, symbol.line_start, symbol.line_end)
            .await;
        let original_tokens = count_tokens(&original_text);
        let compressed_tokens = count_tokens(&summary);

        // Always return a result for symbols — the caller asked for compression
        Ok(Some(CompressedItem {
            original_id: id.to_string(),
            summary,
            original_tokens,
            compressed_tokens,
            method: "symbol_stub".to_string(),
        }))
    }

    /// Generate claims from a symbol's doc comment using the heuristic extractor.
    ///
    /// Returns claim records derived from the doc comment text. If the symbol
    /// has no doc comment, returns an empty vec.
    pub(super) async fn extract_symbol_claims(
        &self,
        symbol_id: &str,
    ) -> Result<Vec<crate::storage::ClaimRecord>, QueryError> {
        let sid = SymbolId(symbol_id.to_string());
        let symbol =
            self.storage
                .get_symbol(&sid)
                .await?
                .ok_or_else(|| QueryError::SymbolNotFound {
                    id: symbol_id.to_string(),
                })?;

        let Some(ref doc) = symbol.doc_comment else {
            return Ok(Vec::new());
        };

        let extractor = HeuristicClaimExtractor::new();
        let section_id = SectionId(symbol_id.to_string());
        let claims = extractor.extract(doc, &section_id);

        Ok(claims
            .into_iter()
            .enumerate()
            .map(|(i, c)| crate::storage::ClaimRecord {
                id: c.id,
                section_id: SectionId(symbol_id.to_string()),
                text: c.text,
                #[allow(clippy::cast_possible_wrap)]
                position: i as i64,
            })
            .collect())
    }

    /// Compress content using LLM-assisted abstractive compression.
    ///
    /// For each content ID, attempts abstractive compression via the given
    /// [`AbstractiveCompressor`] (typically backed by MCP sampling). Falls
    /// back to extractive compression if the abstractive attempt fails.
    ///
    /// Abstractive compression typically achieves 90%+ token reduction
    /// compared to 60–80% for extractive methods.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    /// Abstractive compression errors are handled internally by falling
    /// back to extractive — they do not propagate.
    #[instrument(skip(self, compressor))]
    pub async fn compress_content_abstractive<C: AbstractiveCompressor>(
        &self,
        content_ids: &[String],
        compressor: &C,
    ) -> Result<Vec<CompressedItem>, QueryError> {
        let extractive = ExtractiveSummaryGenerator::new();
        let mut results = Vec::with_capacity(content_ids.len());

        for id in content_ids {
            // Accept both section and claim IDs: claim IDs point at a
            // concrete sub-range of text, but compression operates on the
            // parent section. Round-trip the caller's ID unchanged.
            let section_lookup_id = strip_claim_suffix(id);
            let sid = SectionId(section_lookup_id.to_string());
            let Some(section) = self.storage.get_section(&sid).await? else {
                // Fall back to symbol compression for non-section content IDs
                if let Some(item) = self.try_compress_symbol(id).await? {
                    results.push(item);
                }
                continue;
            };

            let original_tokens = count_tokens(&section.text);
            let context_hint = section.heading_path.join(" > ");

            // Try abstractive compression first
            let (summary, method) = match compressor.compress(&section.text, &context_hint).await {
                Ok(abs_summary) if !abs_summary.trim().is_empty() => (abs_summary, "abstractive"),
                _ => {
                    // Fall back to extractive
                    let ext_summary = section
                        .summary
                        .unwrap_or_else(|| extractive.summarize(&section.text, 2));
                    (ext_summary, "extractive")
                }
            };

            let compressed_tokens = count_tokens(&summary);

            // Skip if no compression achieved
            if compressed_tokens >= original_tokens {
                continue;
            }

            results.push(CompressedItem {
                original_id: id.clone(),
                summary,
                original_tokens,
                compressed_tokens,
                method: method.to_string(),
            });
        }

        Ok(results)
    }
}

/// Strip a trailing `:c<digits>` claim suffix from a content ID, returning
/// the parent section ID. Pass-through for IDs that don't have the suffix
/// (so this is safe to call unconditionally on any caller-supplied ID).
///
/// Claim IDs are emitted by `ministr_survey` and `ministr_extract`; the
/// compression pipeline operates on sections, so we need to resolve claims
/// to their parent before the section-store lookup.
fn strip_claim_suffix(id: &str) -> &str {
    let Some(colon_idx) = id.rfind(":c") else {
        return id;
    };
    let suffix = &id[colon_idx + 2..];
    if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
        &id[..colon_idx]
    } else {
        id
    }
}

#[cfg(test)]
mod claim_suffix_tests {
    use super::strip_claim_suffix;

    #[test]
    fn strips_numeric_claim_suffix() {
        assert_eq!(
            strip_claim_suffix("D:\\foo\\bar.rs#mod::Foo:c0"),
            "D:\\foo\\bar.rs#mod::Foo"
        );
        assert_eq!(
            strip_claim_suffix("D:\\foo\\bar.rs#mod::Foo:c142"),
            "D:\\foo\\bar.rs#mod::Foo"
        );
    }

    #[test]
    fn passes_through_plain_section_ids() {
        assert_eq!(
            strip_claim_suffix("D:\\foo\\bar.rs#mod::Foo"),
            "D:\\foo\\bar.rs#mod::Foo"
        );
    }

    #[test]
    fn passes_through_symbol_ids() {
        // Symbol IDs can contain "::" but never end in ":c<digits>"
        assert_eq!(
            strip_claim_suffix("sym-D:\\foo\\bar.rs::mod::Foo::new"),
            "sym-D:\\foo\\bar.rs::mod::Foo::new"
        );
    }

    #[test]
    fn ignores_non_numeric_colon_c_suffix() {
        // ":c" followed by a non-digit string is NOT a claim suffix
        assert_eq!(strip_claim_suffix("something:cabcd"), "something:cabcd");
        assert_eq!(strip_claim_suffix("foo:c"), "foo:c");
    }
}
