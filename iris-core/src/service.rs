//! Query service layer for iris-core.
//!
//! [`QueryService`] composes the storage, embedding, and vector index
//! subsystems into a high-level API for searching, reading, and extracting
//! content from the corpus. This is the primary interface consumed by
//! transport adapters (e.g. the MCP server in `iris-mcp`).

use std::sync::Arc;

use serde::Serialize;
use tracing::instrument;

use crate::embedding::Embedder;
use crate::error::{IndexError, StorageError};
use crate::extraction::summary::{ExtractiveSummaryGenerator, SummaryGenerator};
use crate::index::VectorIndex;
use crate::search::{MultiResolutionSearch, SearchConfig};
use crate::storage::{SqliteStorage, Storage, SymbolFilter, SymbolRecord};
use crate::token::count_tokens;
use crate::types::{
    ClaimId, ContentId, RefKind, RelationType, Resolution, SectionId, SymbolId, TocEntry, VectorId,
};

/// A ranked result from a corpus survey search.
#[derive(Debug, Clone, Serialize)]
pub struct SurveyResult {
    /// The content ID from the vector index.
    pub content_id: String,
    /// Resolution level of this result.
    pub resolution: String,
    /// Relevance score (higher is better, 0.0–1.0).
    pub score: f32,
    /// Content text — summary for summary-level, section text for section-level,
    /// claim text for claim-level.
    pub text: String,
    /// Heading path for section-level results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<Vec<String>>,
}

/// Detailed section content returned by `read_section`.
#[derive(Debug, Clone, Serialize)]
pub struct SectionDetail {
    /// Section identifier.
    pub section_id: String,
    /// Heading hierarchy path.
    pub heading_path: Vec<String>,
    /// Full section text.
    pub text: String,
    /// Section summary, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Number of claims available for extraction.
    pub claims_available: usize,
}

/// A claim result from extraction.
#[derive(Debug, Clone, Serialize)]
pub struct ClaimResult {
    /// Claim identifier.
    pub claim_id: String,
    /// Claim text.
    pub text: String,
    /// Relevance score when filtered by query (0.0–1.0). `None` if no query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance: Option<f32>,
}

/// A compressed summary of a content item, used for eviction.
///
/// When an agent wants to free budget, it can compress sections into shorter
/// summaries that preserve the gist while reducing token count.
#[derive(Debug, Clone, Serialize)]
pub struct CompressedItem {
    /// The original content ID that was compressed.
    pub original_id: String,
    /// The compressed summary text.
    pub summary: String,
    /// Token count of the original content.
    pub original_tokens: usize,
    /// Token count of the compressed summary.
    pub compressed_tokens: usize,
}

/// A related claim returned by `related_claims`.
#[derive(Debug, Clone, Serialize)]
pub struct RelatedClaimResult {
    /// The related claim's ID.
    pub claim_id: String,
    /// The related claim's text.
    pub text: String,
    /// The type of relationship.
    pub relation_type: String,
    /// The section containing the related claim.
    pub source_section: String,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
}

/// A symbol definition with source context and module hierarchy.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolDefinition {
    /// The symbol record from storage.
    pub id: String,
    /// Symbol name.
    pub name: String,
    /// Symbol kind (e.g. "function", "struct").
    pub kind: String,
    /// Visibility (e.g. "pub", "pub(crate)").
    pub visibility: String,
    /// Declaration signature (without body).
    pub signature: String,
    /// Doc comment text, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_comment: Option<String>,
    /// Source file path relative to corpus root.
    pub file_path: String,
    /// Start line (1-based).
    pub line_start: u32,
    /// End line (1-based, inclusive).
    pub line_end: u32,
    /// Module hierarchy path (e.g. `["config", "IrisConfig"]`).
    pub heading_path: Vec<String>,
    /// Source code of the symbol with 3 lines of surrounding context.
    pub source_context: String,
}

/// A symbol reference result from cross-reference queries.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolRefResult {
    /// The symbol that holds the reference.
    pub from_symbol_id: String,
    /// Name of the referencing symbol.
    pub from_name: String,
    /// File containing the referencing symbol.
    pub from_file: String,
    /// Line of the referencing symbol.
    pub from_line: u32,
    /// The symbol being referenced.
    pub to_symbol_id: String,
    /// Name of the referenced symbol.
    pub to_name: String,
    /// File containing the referenced symbol.
    pub to_file: String,
    /// Line of the referenced symbol.
    pub to_line: u32,
    /// The kind of reference.
    pub ref_kind: String,
}

/// Errors from the query service layer.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    /// A storage operation failed.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// An index or embedding operation failed.
    #[error("index error: {0}")]
    Index(#[from] IndexError),

    /// The requested section was not found.
    #[error("section not found: {id}")]
    SectionNotFound { id: String },

    /// The requested claim was not found.
    #[error("claim not found: {id}")]
    ClaimNotFound { id: String },

    /// The requested symbol was not found.
    #[error("symbol not found: {id}")]
    SymbolNotFound { id: String },
}

/// High-level query service that composes storage, embedding, and vector index.
///
/// This is the main service interface consumed by transport layers. It provides
/// three operations corresponding to the iris MCP tools:
/// - [`survey`](Self::survey) — multi-resolution search
/// - [`read_section`](Self::read_section) — full section retrieval
/// - [`extract_claims`](Self::extract_claims) — claim-level extraction
pub struct QueryService {
    storage: SqliteStorage,
    embedder: Arc<dyn Embedder>,
    index: Arc<dyn VectorIndex>,
}

impl QueryService {
    /// Create a new query service with the given dependencies.
    #[must_use]
    pub fn new(
        storage: SqliteStorage,
        embedder: Arc<dyn Embedder>,
        index: Arc<dyn VectorIndex>,
    ) -> Self {
        Self {
            storage,
            embedder,
            index,
        }
    }

    /// Access the embedder for external use (e.g. topical prefetch).
    #[must_use]
    pub fn embedder(&self) -> &dyn Embedder {
        self.embedder.as_ref()
    }

    /// Access the vector index for external use (e.g. topical prefetch).
    #[must_use]
    pub fn index(&self) -> &dyn VectorIndex {
        self.index.as_ref()
    }

    /// Access the storage layer for external use (e.g. MCP resource listing).
    #[must_use]
    pub fn storage(&self) -> &SqliteStorage {
        &self.storage
    }

    /// Return a table of contents for the corpus.
    ///
    /// Lists all documents and their sections as metadata-only entries.
    /// When `document_id` is provided, returns only sections from that document.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    #[instrument(skip(self))]
    pub async fn toc(&self, document_id: Option<&str>) -> Result<Vec<TocEntry>, QueryError> {
        let docs = match document_id {
            Some(id) => {
                let cid = ContentId(id.to_string());
                match self.storage.get_document(&cid).await? {
                    Some(doc) => vec![doc],
                    None => vec![],
                }
            }
            None => self.storage.list_documents().await?,
        };

        let mut entries = Vec::new();
        for doc in &docs {
            let sections = self.storage.list_sections(&doc.id).await?;
            for section in sections {
                let claims = self.storage.list_claims(&section.id).await?;
                entries.push(TocEntry {
                    document_id: doc.id.clone(),
                    section_id: section.id,
                    heading_path: section.heading_path,
                    depth: section.depth,
                    claims_available: claims.len(),
                    token_count: count_tokens(&section.text),
                });
            }
        }

        Ok(entries)
    }

    /// Search the corpus for content relevant to a natural language query.
    ///
    /// Performs multi-resolution vector search and enriches results with
    /// content from storage.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] if embedding, search, or storage operations fail.
    #[instrument(skip(self), fields(query_len = query.len(), top_k))]
    pub async fn survey(&self, query: &str, top_k: usize) -> Result<Vec<SurveyResult>, QueryError> {
        let searcher = MultiResolutionSearch::new(self.embedder.as_ref(), self.index.as_ref());
        let config = SearchConfig {
            raw_k: top_k.max(10) * 3,
            top_k,
        };

        let scored = searcher.search(query, config)?;

        let mut results = Vec::with_capacity(scored.len());
        for sr in scored {
            let content_id = sr.vector_id.content_id().to_string();
            let resolution = sr.resolution;

            let (text, heading_path) = self
                .resolve_content(&sr.vector_id, resolution)
                .await
                .unwrap_or_else(|_| (format!("[content unavailable: {content_id}]"), None));

            results.push(SurveyResult {
                content_id,
                resolution: resolution.to_string(),
                score: sr.score,
                text,
                heading_path,
            });
        }

        Ok(results)
    }

    /// Read the full text of a section by its hierarchical ID.
    ///
    /// Returns the section content with heading path and the count of
    /// claims available for extraction.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SectionNotFound`] if no section exists with the
    /// given ID, or [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn read_section(&self, section_id: &str) -> Result<SectionDetail, QueryError> {
        let sid = SectionId(section_id.to_string());

        let section =
            self.storage
                .get_section(&sid)
                .await?
                .ok_or_else(|| QueryError::SectionNotFound {
                    id: section_id.to_string(),
                })?;

        let claims = self.storage.list_claims(&sid).await?;

        Ok(SectionDetail {
            section_id: section_id.to_string(),
            heading_path: section.heading_path,
            text: section.text,
            summary: section.summary,
            claims_available: claims.len(),
        })
    }

    /// Extract atomic claims from a section, optionally filtered by query relevance.
    ///
    /// When a query is provided, claims are scored by cosine similarity to the
    /// query embedding and returned in descending relevance order. Without a
    /// query, all claims are returned in document order.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SectionNotFound`] if the section does not exist,
    /// or [`QueryError`] on embedding/storage failures.
    #[instrument(skip(self))]
    pub async fn extract_claims(
        &self,
        section_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<ClaimResult>, QueryError> {
        let sid = SectionId(section_id.to_string());

        // Verify section exists
        self.storage
            .get_section(&sid)
            .await?
            .ok_or_else(|| QueryError::SectionNotFound {
                id: section_id.to_string(),
            })?;

        let claims = self.storage.list_claims(&sid).await?;

        if claims.is_empty() {
            return Ok(Vec::new());
        }

        match query {
            Some(q) if !q.is_empty() => {
                // Embed query and all claim texts, compute cosine similarity
                let claim_texts: Vec<&str> = claims.iter().map(|c| c.text.as_str()).collect();
                let mut all_texts = vec![q];
                all_texts.extend(claim_texts.iter());

                let embeddings = self.embedder.embed(&all_texts)?;
                let query_vec = &embeddings[0];

                let mut scored: Vec<ClaimResult> = claims
                    .iter()
                    .enumerate()
                    .map(|(i, claim)| {
                        let claim_vec = &embeddings[i + 1];
                        let similarity = cosine_similarity(query_vec, claim_vec);
                        ClaimResult {
                            claim_id: claim.id.to_string(),
                            text: claim.text.clone(),
                            relevance: Some(similarity),
                        }
                    })
                    .collect();

                // Sort by relevance descending
                scored.sort_by(|a, b| {
                    b.relevance
                        .unwrap_or(0.0)
                        .partial_cmp(&a.relevance.unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                Ok(scored)
            }
            _ => {
                // No query — return all claims in document order
                Ok(claims
                    .into_iter()
                    .map(|c| ClaimResult {
                        claim_id: c.id.to_string(),
                        text: c.text,
                        relevance: None,
                    })
                    .collect())
            }
        }
    }

    /// Compress content items into shorter summaries for eviction.
    ///
    /// For each content ID, looks up the section text and generates an
    /// extractive summary (2 sentences). Returns the original and compressed
    /// token counts so the agent knows how much budget it saves.
    ///
    /// Content IDs that don't match any section are silently skipped.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    #[instrument(skip(self))]
    pub async fn compress_content(
        &self,
        content_ids: &[String],
    ) -> Result<Vec<CompressedItem>, QueryError> {
        let summarizer = ExtractiveSummaryGenerator::new();
        let mut results = Vec::with_capacity(content_ids.len());

        for id in content_ids {
            let sid = SectionId(id.clone());
            if let Some(section) = self.storage.get_section(&sid).await? {
                // Use existing summary if available, otherwise generate one
                let summary = section
                    .summary
                    .unwrap_or_else(|| summarizer.summarize(&section.text, 2));
                let original_tokens = count_tokens(&section.text);
                let compressed_tokens = count_tokens(&summary);

                results.push(CompressedItem {
                    original_id: id.clone(),
                    summary,
                    original_tokens,
                    compressed_tokens,
                });
            }
            // Silently skip unknown content IDs
        }

        Ok(results)
    }

    /// Find claims related to the given claim via the relationship index.
    ///
    /// Returns claims that reference, contradict, depend on, or update the
    /// given claim. Optionally filtered by relation type.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::ClaimNotFound`] if the claim does not exist,
    /// or [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn related_claims(
        &self,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
    ) -> Result<Vec<RelatedClaimResult>, QueryError> {
        let cid = ClaimId(claim_id.to_string());

        // Verify claim exists
        self.storage
            .get_claim(&cid)
            .await?
            .ok_or_else(|| QueryError::ClaimNotFound {
                id: claim_id.to_string(),
            })?;

        let related = self
            .storage
            .get_related_claims(&cid, relation_types)
            .await?;

        Ok(related
            .into_iter()
            .map(|r| RelatedClaimResult {
                claim_id: r.claim_id.0,
                text: r.text,
                relation_type: r.relation_type.to_string(),
                source_section: r.section_id.0,
                confidence: r.confidence,
            })
            .collect())
    }

    /// Search the symbol index with optional filters.
    ///
    /// Returns symbols matching the given filter criteria. All filter fields
    /// are optional — omitting all fields returns all symbols.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    #[instrument(skip(self))]
    pub async fn search_symbols(
        &self,
        filter: &SymbolFilter,
    ) -> Result<Vec<SymbolRecord>, QueryError> {
        Ok(self.storage.list_symbols(filter).await?)
    }

    /// Get the full definition of a symbol with surrounding source context.
    ///
    /// Returns the symbol metadata plus the source code lines covering
    /// the symbol with 3 lines of surrounding context, and a heading path
    /// showing the module hierarchy.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SymbolNotFound`] if no symbol with the given ID
    /// exists, or [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn get_symbol_definition(
        &self,
        symbol_id: &str,
    ) -> Result<SymbolDefinition, QueryError> {
        let sid = SymbolId(symbol_id.to_string());
        let symbol =
            self.storage
                .get_symbol(&sid)
                .await?
                .ok_or_else(|| QueryError::SymbolNotFound {
                    id: symbol_id.to_string(),
                })?;

        // Build heading path from module path + symbol name
        let mut heading_path: Vec<String> = symbol
            .module_path
            .split("::")
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        heading_path.push(symbol.name.clone());

        // Read source file and extract context lines
        let source_context = self
            .read_source_context(&symbol.file_path, symbol.line_start, symbol.line_end)
            .await;

        Ok(SymbolDefinition {
            id: symbol.id.0.clone(),
            name: symbol.name,
            kind: symbol.kind,
            visibility: symbol.visibility,
            signature: symbol.signature,
            doc_comment: symbol.doc_comment,
            file_path: symbol.file_path,
            line_start: symbol.line_start,
            line_end: symbol.line_end,
            heading_path,
            source_context,
        })
    }

    /// Get all references for a symbol, optionally filtered by reference kind.
    ///
    /// Returns cross-references where the given symbol is the target (i.e.
    /// callers, implementors, importers of the symbol).
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SymbolNotFound`] if the symbol does not exist,
    /// or [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn get_symbol_references(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
    ) -> Result<Vec<SymbolRefResult>, QueryError> {
        let sid = SymbolId(symbol_id.to_string());

        // Verify symbol exists
        self.storage
            .get_symbol(&sid)
            .await?
            .ok_or_else(|| QueryError::SymbolNotFound {
                id: symbol_id.to_string(),
            })?;

        let refs = self.storage.query_refs(&sid, ref_kind).await?;

        let mut results = Vec::with_capacity(refs.len());
        for r in refs {
            let from = self.storage.get_symbol(&r.from_symbol_id).await?;
            let to = self.storage.get_symbol(&r.to_symbol_id).await?;

            if let (Some(from_sym), Some(to_sym)) = (from, to) {
                results.push(SymbolRefResult {
                    from_symbol_id: from_sym.id.0,
                    from_name: from_sym.name,
                    from_file: from_sym.file_path,
                    from_line: from_sym.line_start,
                    to_symbol_id: to_sym.id.0,
                    to_name: to_sym.name,
                    to_file: to_sym.file_path,
                    to_line: to_sym.line_start,
                    ref_kind: r.ref_kind.to_string(),
                });
            }
        }

        Ok(results)
    }

    /// Read source file lines for symbol context display.
    ///
    /// Returns the symbol's source lines with 3 lines of surrounding context.
    /// Falls back to a placeholder if the file cannot be read.
    async fn read_source_context(&self, file_path: &str, line_start: u32, line_end: u32) -> String {
        let Ok(content) = tokio::fs::read_to_string(file_path).await else {
            return format!("[source unavailable: {file_path}]");
        };

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        // 3 lines of context before and after, clamped to file bounds
        let ctx = 3;
        let start = (line_start as usize).saturating_sub(1).saturating_sub(ctx);
        let end = (line_end as usize)
            .min(total)
            .saturating_add(ctx)
            .min(total);

        lines[start..end].join("\n")
    }

    /// Resolve a vector ID to its content text and optional heading path.
    async fn resolve_content(
        &self,
        vector_id: &VectorId,
        resolution: Resolution,
    ) -> Result<(String, Option<Vec<String>>), QueryError> {
        let content_id = vector_id.content_id();

        match resolution {
            Resolution::Summary => {
                if vector_id.is_doc_summary() {
                    // Document summary — look up document record
                    let doc_id = ContentId(content_id.to_string());
                    if let Some(doc) = self.storage.get_document(&doc_id).await? {
                        let text = doc
                            .summary
                            .unwrap_or_else(|| format!("[no summary for document: {}]", doc.title));
                        Ok((text, None))
                    } else {
                        Ok((format!("[document not found: {content_id}]"), None))
                    }
                } else {
                    // Section summary — look up section record
                    let sid = SectionId(content_id.to_string());
                    if let Some(section) = self.storage.get_section(&sid).await? {
                        let text = section
                            .summary
                            .unwrap_or_else(|| format!("[no summary for section: {content_id}]"));
                        Ok((text, Some(section.heading_path)))
                    } else {
                        Ok((format!("[section not found: {content_id}]"), None))
                    }
                }
            }
            Resolution::Section => {
                let sid = SectionId(content_id.to_string());
                if let Some(section) = self.storage.get_section(&sid).await? {
                    Ok((section.text, Some(section.heading_path)))
                } else {
                    Ok((format!("[section not found: {content_id}]"), None))
                }
            }
            Resolution::Claim => {
                let cid = ClaimId(content_id.to_string());
                if let Some(claim) = self.storage.get_claim(&cid).await? {
                    Ok((claim.text, None))
                } else {
                    Ok((format!("[claim not found: {content_id}]"), None))
                }
            }
        }
    }
}

/// Compute cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::HnswIndex;
    use crate::storage::SqliteStorage;
    use crate::types::{
        Claim, ClaimId, ClaimRelationship, ContentId, DocumentTree, Section, SectionId,
    };

    /// Deterministic mock embedder for testing.
    struct MockEmbedder {
        dim: usize,
    }

    impl Embedder for MockEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
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

    fn make_test_doc() -> DocumentTree {
        let claims = vec![
            Claim {
                id: ClaimId("c1".into()),
                text: "JWT tokens use RS256 signing algorithm.".into(),
                section_id: SectionId("docs/auth.md#tokens".into()),
            },
            Claim {
                id: ClaimId("c2".into()),
                text: "Tokens expire after 24 hours by default.".into(),
                section_id: SectionId("docs/auth.md#tokens".into()),
            },
        ];

        let section = Section {
            id: SectionId("docs/auth.md#tokens".into()),
            heading_path: vec!["Authentication".into(), "Tokens".into()],
            depth: 2,
            text: "JWT tokens use RS256 signing. Tokens expire after 24 hours.".into(),
            structural_nodes: vec![],
            children: vec![],
            claims,
            summary: Some("Token authentication details.".into()),
        };

        DocumentTree {
            id: ContentId("docs/auth.md".into()),
            title: "Authentication Guide".into(),
            source_path: "docs/auth.md".into(),
            sections: vec![section],
            summary: Some("Complete authentication reference.".into()),
        }
    }

    async fn setup_service() -> QueryService {
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let doc = make_test_doc();
        storage.insert_document(&doc).await.unwrap();

        // Insert vectors into the index for each content piece
        let texts_and_ids = [
            (
                "doc-summary::docs/auth.md",
                "Complete authentication reference.",
            ),
            (
                "sec-summary::docs/auth.md#tokens",
                "Token authentication details.",
            ),
            (
                "section::docs/auth.md#tokens",
                "JWT tokens use RS256 signing. Tokens expire after 24 hours.",
            ),
            ("claim::c1", "JWT tokens use RS256 signing algorithm."),
            ("claim::c2", "Tokens expire after 24 hours by default."),
        ];

        for (id, text) in &texts_and_ids {
            let vecs = embedder.embed(&[*text]).unwrap();
            index.insert(id, &vecs[0]).unwrap();
        }

        QueryService::new(storage, embedder, index)
    }

    // --- survey tests ---

    #[tokio::test]
    async fn survey_returns_results_for_relevant_query() {
        let service = setup_service().await;
        let results = service
            .survey("JWT authentication tokens", 5)
            .await
            .unwrap();

        assert!(!results.is_empty(), "survey should return results");
        for r in &results {
            assert!(r.score > 0.0);
            assert!(!r.text.is_empty());
            assert!(!r.content_id.is_empty());
        }
    }

    #[tokio::test]
    async fn survey_results_sorted_by_score() {
        let service = setup_service().await;
        let results = service.survey("token signing RS256", 10).await.unwrap();

        for window in results.windows(2) {
            assert!(window[0].score >= window[1].score);
        }
    }

    #[tokio::test]
    async fn survey_respects_top_k() {
        let service = setup_service().await;
        let results = service.survey("tokens", 2).await.unwrap();

        assert!(results.len() <= 2);
    }

    #[tokio::test]
    async fn survey_enriches_section_results_with_heading_path() {
        let service = setup_service().await;
        let results = service.survey("JWT tokens signing", 10).await.unwrap();

        let section_result = results.iter().find(|r| r.resolution == "section");
        if let Some(sr) = section_result {
            assert!(
                sr.heading_path.is_some(),
                "section results should have heading_path"
            );
        }
    }

    // --- read_section tests ---

    #[tokio::test]
    async fn read_section_returns_existing_section() {
        let service = setup_service().await;
        let detail = service.read_section("docs/auth.md#tokens").await.unwrap();

        assert_eq!(detail.section_id, "docs/auth.md#tokens");
        assert_eq!(
            detail.heading_path,
            vec!["Authentication".to_string(), "Tokens".to_string()]
        );
        assert!(detail.text.contains("JWT tokens"));
        assert_eq!(detail.claims_available, 2);
        assert_eq!(
            detail.summary.as_deref(),
            Some("Token authentication details.")
        );
    }

    #[tokio::test]
    async fn read_section_not_found() {
        let service = setup_service().await;
        let result = service.read_section("nonexistent#section").await;

        assert!(matches!(result, Err(QueryError::SectionNotFound { .. })));
    }

    // --- extract_claims tests ---

    #[tokio::test]
    async fn extract_claims_returns_all_claims_without_query() {
        let service = setup_service().await;
        let claims = service
            .extract_claims("docs/auth.md#tokens", None)
            .await
            .unwrap();

        assert_eq!(claims.len(), 2);
        assert!(claims[0].relevance.is_none(), "no relevance without query");
        assert!(claims.iter().any(|c| c.text.contains("RS256")));
        assert!(claims.iter().any(|c| c.text.contains("24 hours")));
    }

    #[tokio::test]
    async fn extract_claims_with_query_returns_scored_results() {
        let service = setup_service().await;
        let claims = service
            .extract_claims("docs/auth.md#tokens", Some("signing algorithm"))
            .await
            .unwrap();

        assert_eq!(claims.len(), 2);
        for c in &claims {
            assert!(c.relevance.is_some(), "should have relevance with query");
        }
        // Results should be sorted by relevance descending
        assert!(claims[0].relevance.unwrap() >= claims[1].relevance.unwrap());
    }

    #[tokio::test]
    async fn extract_claims_section_not_found() {
        let service = setup_service().await;
        let result = service.extract_claims("nonexistent#section", None).await;

        assert!(matches!(result, Err(QueryError::SectionNotFound { .. })));
    }

    #[tokio::test]
    async fn extract_claims_empty_section() {
        let service = setup_service().await;

        // Insert a section with no claims
        let doc = DocumentTree {
            id: ContentId("empty-doc".into()),
            title: "Empty".into(),
            source_path: "empty.md".into(),
            sections: vec![Section {
                id: SectionId("empty.md#intro".into()),
                heading_path: vec!["Intro".into()],
                depth: 1,
                text: "Just some text.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![],
                summary: None,
            }],
            summary: None,
        };
        service.storage.insert_document(&doc).await.unwrap();

        let claims = service
            .extract_claims("empty.md#intro", None)
            .await
            .unwrap();
        assert!(claims.is_empty());
    }

    // --- compress_content tests ---

    #[tokio::test]
    async fn compress_known_section_returns_summary() {
        let service = setup_service().await;
        let results = service
            .compress_content(&["docs/auth.md#tokens".to_string()])
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_id, "docs/auth.md#tokens");
        assert!(!results[0].summary.is_empty());
        assert!(results[0].original_tokens > 0);
        assert!(results[0].compressed_tokens <= results[0].original_tokens);
    }

    #[tokio::test]
    async fn compress_unknown_section_is_skipped() {
        let service = setup_service().await;
        let results = service
            .compress_content(&["nonexistent#section".to_string()])
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn compress_empty_list_returns_empty() {
        let service = setup_service().await;
        let results = service.compress_content(&[]).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn compress_uses_existing_summary_when_available() {
        let service = setup_service().await;
        let results = service
            .compress_content(&["docs/auth.md#tokens".to_string()])
            .await
            .unwrap();

        // The test section has a pre-generated summary "Token authentication details."
        assert_eq!(results[0].summary, "Token authentication details.");
    }

    // --- cosine_similarity tests ---

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-5);
    }

    #[test]
    fn cosine_similarity_zero_vector() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < f32::EPSILON);
    }

    // --- related_claims tests ---

    #[tokio::test]
    async fn related_claims_returns_related() {
        let service = setup_service().await;

        // Insert relationships
        let relationships = vec![ClaimRelationship {
            source_claim_id: ClaimId("c1".into()),
            target_claim_id: ClaimId("c2".into()),
            relation_type: RelationType::References,
            confidence: 0.8,
        }];
        service
            .storage
            .insert_claim_relationships(&relationships)
            .await
            .unwrap();

        let related = service.related_claims("c1", None).await.unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].claim_id, "c2");
        assert_eq!(related[0].relation_type, "references");
        assert_eq!(related[0].source_section, "docs/auth.md#tokens");
    }

    #[tokio::test]
    async fn related_claims_filters_by_type() {
        let service = setup_service().await;

        let relationships = vec![
            ClaimRelationship {
                source_claim_id: ClaimId("c1".into()),
                target_claim_id: ClaimId("c2".into()),
                relation_type: RelationType::References,
                confidence: 0.8,
            },
            ClaimRelationship {
                source_claim_id: ClaimId("c1".into()),
                target_claim_id: ClaimId("c2".into()),
                relation_type: RelationType::Updates,
                confidence: 0.6,
            },
        ];
        service
            .storage
            .insert_claim_relationships(&relationships)
            .await
            .unwrap();

        let related = service
            .related_claims("c1", Some(&[RelationType::Updates]))
            .await
            .unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].relation_type, "updates");
    }

    #[tokio::test]
    async fn related_claims_not_found() {
        let service = setup_service().await;
        let result = service.related_claims("nonexistent", None).await;
        assert!(matches!(result, Err(QueryError::ClaimNotFound { .. })));
    }

    // --- toc tests ---

    /// Build a multi-doc corpus with nested headings for toc testing.
    async fn setup_multi_doc_service() -> QueryService {
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let docs = vec![
            DocumentTree {
                id: ContentId("docs/auth.md".into()),
                title: "Authentication Guide".into(),
                source_path: "docs/auth.md".into(),
                sections: vec![
                    Section {
                        id: SectionId("docs/auth.md#tokens".into()),
                        heading_path: vec!["Authentication".into(), "Tokens".into()],
                        depth: 2,
                        text: "JWT tokens use RS256 signing.".into(),
                        structural_nodes: vec![],
                        children: vec![],
                        claims: vec![
                            Claim {
                                id: ClaimId("auth-c1".into()),
                                text: "JWT tokens use RS256.".into(),
                                section_id: SectionId("docs/auth.md#tokens".into()),
                            },
                            Claim {
                                id: ClaimId("auth-c2".into()),
                                text: "Tokens expire after 24h.".into(),
                                section_id: SectionId("docs/auth.md#tokens".into()),
                            },
                        ],
                        summary: None,
                    },
                    Section {
                        id: SectionId("docs/auth.md#oauth".into()),
                        heading_path: vec!["Authentication".into(), "OAuth".into()],
                        depth: 2,
                        text: "OAuth 2.0 with PKCE.".into(),
                        structural_nodes: vec![],
                        children: vec![],
                        claims: vec![Claim {
                            id: ClaimId("auth-c3".into()),
                            text: "OAuth 2.0 is supported.".into(),
                            section_id: SectionId("docs/auth.md#oauth".into()),
                        }],
                        summary: None,
                    },
                ],
                summary: Some("Auth reference.".into()),
            },
            DocumentTree {
                id: ContentId("docs/api.md".into()),
                title: "API Reference".into(),
                source_path: "docs/api.md".into(),
                sections: vec![Section {
                    id: SectionId("docs/api.md#rate-limits".into()),
                    heading_path: vec!["API Reference".into(), "Rate Limits".into()],
                    depth: 2,
                    text: "100 requests per minute.".into(),
                    structural_nodes: vec![],
                    children: vec![],
                    claims: vec![Claim {
                        id: ClaimId("api-c1".into()),
                        text: "Rate limit is 100/min.".into(),
                        section_id: SectionId("docs/api.md#rate-limits".into()),
                    }],
                    summary: None,
                }],
                summary: Some("API docs.".into()),
            },
        ];

        for doc in &docs {
            storage.insert_document(doc).await.unwrap();
        }

        QueryService::new(storage, embedder, index)
    }

    #[tokio::test]
    async fn toc_returns_correct_tree_for_multi_doc_corpus() {
        let service = setup_multi_doc_service().await;
        let entries = service.toc(None).await.unwrap();

        // Should have 3 sections total across 2 documents
        assert_eq!(entries.len(), 3, "expected 3 sections total");

        // Verify auth doc sections
        let auth_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.document_id.as_ref() == "docs/auth.md")
            .collect();
        assert_eq!(auth_entries.len(), 2, "auth doc should have 2 sections");

        let tokens_entry = auth_entries
            .iter()
            .find(|e| e.section_id.as_ref() == "docs/auth.md#tokens")
            .expect("should find tokens section");
        assert_eq!(tokens_entry.heading_path, vec!["Authentication", "Tokens"]);
        assert_eq!(tokens_entry.depth, 2);
        assert_eq!(tokens_entry.claims_available, 2);
        assert!(tokens_entry.token_count > 0);

        // Verify api doc section
        let api_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.document_id.as_ref() == "docs/api.md")
            .collect();
        assert_eq!(api_entries.len(), 1, "api doc should have 1 section");
        assert_eq!(api_entries[0].claims_available, 1);
    }

    #[tokio::test]
    async fn toc_filters_by_document_id() {
        let service = setup_multi_doc_service().await;
        let entries = service.toc(Some("docs/api.md")).await.unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].document_id.as_ref(), "docs/api.md");
        assert_eq!(entries[0].section_id.as_ref(), "docs/api.md#rate-limits");
    }

    #[tokio::test]
    async fn toc_returns_empty_for_unknown_document() {
        let service = setup_multi_doc_service().await;
        let entries = service.toc(Some("nonexistent.md")).await.unwrap();

        assert!(entries.is_empty());
    }
}
