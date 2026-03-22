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
use crate::storage::{SqliteStorage, Storage};
use crate::token::count_tokens;
use crate::types::{Resolution, SectionId, VectorId};

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
                    let doc_id = crate::types::ContentId(content_id.to_string());
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
                let cid = crate::types::ClaimId(content_id.to_string());
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
    use crate::types::{Claim, ClaimId, ContentId, DocumentTree, Section, SectionId};

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
}
