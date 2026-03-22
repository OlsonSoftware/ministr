//! Multi-resolution query pipeline.
//!
//! Searches across all three resolution levels (summaries, sections, claims),
//! merges results, and applies resolution-aware scoring. The query pipeline
//! embeds the query text, searches the vector index, then enriches results
//! with metadata from storage.

use std::collections::HashMap;

use tracing::instrument;

use crate::error::IndexError;
use crate::index::{SearchResult as RawSearchResult, SparseIndex, VectorIndex};
use crate::types::{Resolution, VectorId};

/// Default number of raw candidates to retrieve before re-ranking.
const DEFAULT_RAW_K: usize = 30;

/// Resolution weight for claim-level results (highest precision).
const CLAIM_WEIGHT: f32 = 1.0;

/// Resolution weight for section-level results.
const SECTION_WEIGHT: f32 = 0.85;

/// Resolution weight for summary-level results (broadest).
const SUMMARY_WEIGHT: f32 = 0.7;

/// Resolution weight for code symbol stubs (signature + doc, high precision).
const SYMBOL_STUB_WEIGHT: f32 = 0.95;

/// Resolution weight for code symbol full source.
const SYMBOL_FULL_WEIGHT: f32 = 0.9;

/// Word count threshold: queries with fewer words than this are considered
/// "broad" and get a summary boost.
const BROAD_QUERY_WORD_THRESHOLD: usize = 4;

/// Boost applied to summary results for broad (short) queries.
const BROAD_QUERY_SUMMARY_BOOST: f32 = 0.15;

/// RRF smoothing constant (k in the formula `1/(k + rank)`).
/// The standard value from the original RRF paper.
const RRF_K: f32 = 60.0;

/// A scored search result from the multi-resolution pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredResult {
    /// The vector ID identifying the matched content.
    pub vector_id: VectorId,
    /// Raw distance from the vector index (lower = more similar).
    pub raw_distance: f32,
    /// Resolution level of the result.
    pub resolution: Resolution,
    /// Final score after resolution weighting (higher = better).
    pub score: f32,
}

/// Configuration for multi-resolution search.
#[derive(Debug, Clone, Copy)]
pub struct SearchConfig {
    /// Number of raw candidates to fetch from the vector index.
    pub raw_k: usize,
    /// Number of final results to return after re-ranking.
    pub top_k: usize,
    /// Weight for sparse retrieval in hybrid search (0.0–1.0).
    ///
    /// - `0.0` — dense only (default, backward compatible)
    /// - `0.5` — equal weight to dense and sparse
    /// - `1.0` — sparse only
    ///
    /// When > 0.0, the search pipeline uses RRF fusion to merge
    /// dense (HNSW) and sparse (inverted index) results.
    pub sparse_weight: f32,
    /// Number of top candidates to pass through cross-encoder reranking.
    ///
    /// When `Some(n)`, the top `n` candidates from vector search are
    /// reranked by a cross-encoder model before truncating to `top_k`.
    /// When `None`, reranking is skipped (default).
    pub rerank_top_k: Option<usize>,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            raw_k: DEFAULT_RAW_K,
            top_k: 10,
            sparse_weight: 0.0,
            rerank_top_k: None,
        }
    }
}

/// Multi-resolution search over a vector index.
///
/// Embeds a query, retrieves candidates from the vector index, applies
/// resolution-aware scoring, and returns ranked results. Optionally performs
/// hybrid search by fusing dense and sparse retrieval via RRF.
///
/// # Examples
///
/// ```no_run
/// use iris_core::search::{MultiResolutionSearch, SearchConfig};
/// use iris_core::embedding::FastEmbedder;
/// use iris_core::index::HnswIndex;
///
/// let embedder = FastEmbedder::new("all-MiniLM-L6-v2", None).unwrap();
/// let index = HnswIndex::new(384, 10_000).unwrap();
/// let searcher = MultiResolutionSearch::new(&embedder, &index);
/// let results = searcher.search("authentication tokens", SearchConfig::default()).unwrap();
/// ```
pub struct MultiResolutionSearch<'a, E: ?Sized, I: ?Sized> {
    embedder: &'a E,
    index: &'a I,
    sparse_embedder: Option<&'a dyn crate::embedding::SparseEmbedder>,
    sparse_index: Option<&'a dyn SparseIndex>,
}

impl<'a, E, I> MultiResolutionSearch<'a, E, I>
where
    E: crate::embedding::Embedder + ?Sized,
    I: VectorIndex + ?Sized,
{
    /// Create a new multi-resolution search pipeline (dense only).
    #[must_use]
    pub fn new(embedder: &'a E, index: &'a I) -> Self {
        Self {
            embedder,
            index,
            sparse_embedder: None,
            sparse_index: None,
        }
    }

    /// Add sparse search components for hybrid retrieval.
    #[must_use]
    pub fn with_sparse(
        mut self,
        sparse_embedder: &'a dyn crate::embedding::SparseEmbedder,
        sparse_index: &'a dyn SparseIndex,
    ) -> Self {
        self.sparse_embedder = Some(sparse_embedder);
        self.sparse_index = Some(sparse_index);
        self
    }

    /// Search across all resolution levels and return scored, ranked results.
    ///
    /// The query is embedded, then `raw_k` candidates are retrieved from the
    /// vector index. When sparse components are configured and `sparse_weight > 0`,
    /// results from both dense and sparse retrieval are fused using RRF.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError`] if embedding or search fails.
    #[instrument(skip(self), fields(query_len = query.len()))]
    pub fn search(
        &self,
        query: &str,
        config: SearchConfig,
    ) -> Result<Vec<ScoredResult>, IndexError> {
        let query_vectors = self.embedder.embed(&[query])?;
        let query_vector =
            query_vectors
                .into_iter()
                .next()
                .ok_or_else(|| IndexError::EmbeddingFailed {
                    reason: "embedder returned no vectors".to_string(),
                })?;

        let raw_results = self.index.search_knn(&query_vector, config.raw_k)?;

        let is_broad_query = query.split_whitespace().count() < BROAD_QUERY_WORD_THRESHOLD;

        let sparse_pair = if config.sparse_weight > 0.0 {
            self.sparse_embedder.zip(self.sparse_index)
        } else {
            None
        };

        let mut scored = if let Some((sparse_embedder, sparse_index)) = sparse_pair {
            let sparse_vecs = sparse_embedder.embed_sparse(&[query])?;
            let sparse_vec =
                sparse_vecs
                    .into_iter()
                    .next()
                    .ok_or_else(|| IndexError::EmbeddingFailed {
                        reason: "sparse embedder returned no vectors".to_string(),
                    })?;

            let sparse_results = sparse_index.search_sparse(
                &sparse_vec.indices,
                &sparse_vec.values,
                config.raw_k,
            )?;

            // Build dense scored results
            let dense_scored: Vec<ScoredResult> = raw_results
                .iter()
                .filter_map(|r| score_result(r, is_broad_query))
                .collect();

            // Fuse using RRF
            rrf_fuse(&dense_scored, &sparse_results, config.sparse_weight)
        } else {
            raw_results
                .iter()
                .filter_map(|r| score_result(r, is_broad_query))
                .collect()
        };

        // Sort by score descending (higher is better)
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        scored.truncate(config.top_k);
        Ok(scored)
    }
}

/// Fuse dense and sparse results using Reciprocal Rank Fusion.
///
/// Each result gets an RRF score of `1/(RRF_K + rank)` from each list it appears in.
/// The dense and sparse contributions are weighted by `(1 - sparse_weight)` and
/// `sparse_weight` respectively. Results that appear only in the sparse list
/// (without a parseable `VectorId`) are excluded, since they can't carry
/// resolution metadata.
#[allow(clippy::cast_precision_loss)] // rank indices are small enough for f32
fn rrf_fuse(
    dense: &[ScoredResult],
    sparse: &[crate::index::SparseSearchResult],
    sparse_weight: f32,
) -> Vec<ScoredResult> {
    let dense_weight = 1.0 - sparse_weight;

    // Collect RRF scores keyed by vector ID string
    let mut rrf_scores: HashMap<String, f32> = HashMap::new();
    let mut result_map: HashMap<String, ScoredResult> = HashMap::new();

    // Dense contributions
    for (rank, result) in dense.iter().enumerate() {
        let id = result.vector_id.as_str().to_string();
        let rrf_score = dense_weight / (RRF_K + rank as f32 + 1.0);
        *rrf_scores.entry(id.clone()).or_default() += rrf_score;
        result_map.entry(id).or_insert_with(|| result.clone());
    }

    // Sparse contributions — only for IDs that parse as VectorIds
    for (rank, result) in sparse.iter().enumerate() {
        if let Some(vid) = VectorId::parse(&result.id) {
            let id = result.id.clone();
            let rrf_score = sparse_weight / (RRF_K + rank as f32 + 1.0);
            *rrf_scores.entry(id.clone()).or_default() += rrf_score;
            result_map.entry(id).or_insert_with(|| {
                let resolution = vid.resolution();
                ScoredResult {
                    vector_id: vid,
                    raw_distance: 0.0, // not available from sparse search
                    resolution,
                    score: 0.0, // will be overwritten
                }
            });
        }
    }

    // Build final results with RRF scores
    rrf_scores
        .into_iter()
        .filter_map(|(id, rrf_score)| {
            result_map.remove(&id).map(|mut r| {
                r.score = rrf_score;
                r
            })
        })
        .collect()
}

/// Convert a raw search result into a scored result with resolution weighting.
///
/// Returns `None` if the vector ID cannot be parsed (i.e., not a multi-resolution ID).
fn score_result(raw: &RawSearchResult, is_broad_query: bool) -> Option<ScoredResult> {
    let vector_id = VectorId::parse(&raw.id)?;
    let resolution = vector_id.resolution();

    // Convert distance to similarity (cosine distance is in [0, 2], similarity in [0, 1])
    let similarity = 1.0 - (raw.distance / 2.0);

    let resolution_weight = match resolution {
        Resolution::Claim => CLAIM_WEIGHT,
        Resolution::Section => SECTION_WEIGHT,
        Resolution::Summary => {
            if is_broad_query {
                SUMMARY_WEIGHT + BROAD_QUERY_SUMMARY_BOOST
            } else {
                SUMMARY_WEIGHT
            }
        }
        Resolution::SymbolStub => SYMBOL_STUB_WEIGHT,
        Resolution::SymbolFull => SYMBOL_FULL_WEIGHT,
    };

    let score = similarity * resolution_weight;

    Some(ScoredResult {
        vector_id,
        raw_distance: raw.distance,
        resolution,
        score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::{Embedder, SparseEmbedder, SparseVector};
    use crate::index::{HnswIndex, InvertedIndex, SparseIndex, VectorIndex};

    /// Deterministic mock embedder that produces unit vectors.
    struct MockEmbedder {
        dim: usize,
    }

    impl Embedder for MockEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(texts
                .iter()
                .map(|t| {
                    // Hash-based deterministic vector for testability
                    let mut v = vec![0.0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += f32::from(b) / 255.0;
                    }
                    // Normalize
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

    fn setup_index_with_resolutions() -> (MockEmbedder, HnswIndex) {
        let dim = 8;
        let embedder = MockEmbedder { dim };
        let index = HnswIndex::new(dim, 1000).unwrap();

        // Insert vectors at different resolutions
        let texts = [
            ("doc-summary::doc1", "overview of authentication system"),
            ("sec-summary::doc1#auth", "auth section summary"),
            ("section::doc1#auth", "detailed auth section text"),
            ("claim::c1", "JWT tokens use RS256 signing"),
            ("claim::c2", "rate limits are 100 per minute"),
            ("section::doc1#rate", "rate limiting configuration"),
        ];

        for (id, text) in &texts {
            let vecs = embedder.embed(&[*text]).unwrap();
            index.insert(id, &vecs[0]).unwrap();
        }

        (embedder, index)
    }

    #[test]
    fn search_returns_scored_results() {
        let (embedder, index) = setup_index_with_resolutions();
        let searcher = MultiResolutionSearch::new(&embedder, &index);

        let results = searcher
            .search("authentication", SearchConfig::default())
            .unwrap();

        assert!(!results.is_empty());
        // All results should have valid resolutions
        for r in &results {
            assert!(r.score > 0.0);
            assert!(r.score <= 1.0);
        }
    }

    #[test]
    fn results_are_sorted_by_score_descending() {
        let (embedder, index) = setup_index_with_resolutions();
        let searcher = MultiResolutionSearch::new(&embedder, &index);

        let results = searcher
            .search("JWT signing tokens", SearchConfig::default())
            .unwrap();

        for window in results.windows(2) {
            assert!(window[0].score >= window[1].score);
        }
    }

    #[test]
    fn search_respects_top_k() {
        let (embedder, index) = setup_index_with_resolutions();
        let searcher = MultiResolutionSearch::new(&embedder, &index);

        let config = SearchConfig {
            raw_k: 30,
            top_k: 2,
            sparse_weight: 0.0,
            rerank_top_k: None,
        };
        let results = searcher.search("auth", config).unwrap();
        assert!(results.len() <= 2);
    }

    #[test]
    fn claims_weighted_higher_than_summaries() {
        // For same distance, claim score > summary score
        let claim_raw = RawSearchResult {
            id: "claim::c1".to_string(),
            distance: 0.3,
        };
        let claim_result = score_result(&claim_raw, false).unwrap();

        let summary_raw = RawSearchResult {
            id: "doc-summary::d1".to_string(),
            distance: 0.3,
        };
        let summary_result = score_result(&summary_raw, false).unwrap();

        assert!(claim_result.score > summary_result.score);
    }

    #[test]
    fn broad_query_boosts_summaries() {
        let raw = RawSearchResult {
            id: "doc-summary::d1".to_string(),
            distance: 0.3,
        };
        let narrow = score_result(&raw, false).unwrap();
        let broad = score_result(&raw, true).unwrap();

        assert!(broad.score > narrow.score);
    }

    #[test]
    fn invalid_vector_ids_filtered_out() {
        let raw = RawSearchResult {
            id: "plain-id-no-prefix".to_string(),
            distance: 0.1,
        };
        assert!(score_result(&raw, false).is_none());
    }

    #[test]
    fn search_empty_index() {
        let dim = 8;
        let embedder = MockEmbedder { dim };
        let index = HnswIndex::new(dim, 100).unwrap();
        let searcher = MultiResolutionSearch::new(&embedder, &index);

        let results = searcher
            .search("anything", SearchConfig::default())
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn score_result_resolutions_correct() {
        let make = |id: &str| {
            let raw = RawSearchResult {
                id: id.to_string(),
                distance: 0.2,
            };
            score_result(&raw, false).unwrap()
        };

        assert_eq!(make("doc-summary::d1").resolution, Resolution::Summary);
        assert_eq!(make("sec-summary::s1").resolution, Resolution::Summary);
        assert_eq!(make("section::s1").resolution, Resolution::Section);
        assert_eq!(make("claim::c1").resolution, Resolution::Claim);
        assert_eq!(
            make("symbol-stub::sym-foo").resolution,
            Resolution::SymbolStub
        );
        assert_eq!(
            make("symbol-full::sym-bar").resolution,
            Resolution::SymbolFull
        );
    }

    #[test]
    fn symbol_stub_weighted_higher_than_section() {
        let stub_raw = RawSearchResult {
            id: "symbol-stub::sym-foo".to_string(),
            distance: 0.3,
        };
        let stub_result = score_result(&stub_raw, false).unwrap();

        let section_raw = RawSearchResult {
            id: "section::s1".to_string(),
            distance: 0.3,
        };
        let section_result = score_result(&section_raw, false).unwrap();

        assert!(stub_result.score > section_result.score);
    }

    #[test]
    fn symbol_full_weighted_higher_than_section() {
        let full_raw = RawSearchResult {
            id: "symbol-full::sym-bar".to_string(),
            distance: 0.3,
        };
        let full_result = score_result(&full_raw, false).unwrap();

        let section_raw = RawSearchResult {
            id: "section::s1".to_string(),
            distance: 0.3,
        };
        let section_result = score_result(&section_raw, false).unwrap();

        assert!(full_result.score > section_result.score);
    }

    // --- RRF fusion tests ---

    #[test]
    fn rrf_fuse_dense_only() {
        let dense = vec![
            ScoredResult {
                vector_id: VectorId::parse("claim::c1").unwrap(),
                raw_distance: 0.2,
                resolution: Resolution::Claim,
                score: 0.9,
            },
            ScoredResult {
                vector_id: VectorId::parse("section::s1").unwrap(),
                raw_distance: 0.3,
                resolution: Resolution::Section,
                score: 0.8,
            },
        ];
        let sparse = vec![];

        let fused = rrf_fuse(&dense, &sparse, 0.5);
        assert_eq!(fused.len(), 2);
        // All results should have positive scores
        for r in &fused {
            assert!(r.score > 0.0);
        }
    }

    #[test]
    fn rrf_fuse_merges_overlapping_results() {
        let dense = vec![ScoredResult {
            vector_id: VectorId::parse("claim::c1").unwrap(),
            raw_distance: 0.2,
            resolution: Resolution::Claim,
            score: 0.9,
        }];
        let sparse = vec![crate::index::SparseSearchResult {
            id: "claim::c1".to_string(),
            score: 5.0,
        }];

        let fused = rrf_fuse(&dense, &sparse, 0.5);
        assert_eq!(fused.len(), 1);
        // Should have contributions from both dense and sparse
        let score = fused[0].score;
        let dense_only_score = 0.5 / (RRF_K + 1.0);
        assert!(
            score > dense_only_score,
            "fused score should exceed dense-only"
        );
    }

    #[test]
    fn rrf_fuse_sparse_introduces_new_results() {
        let dense = vec![ScoredResult {
            vector_id: VectorId::parse("claim::c1").unwrap(),
            raw_distance: 0.2,
            resolution: Resolution::Claim,
            score: 0.9,
        }];
        let sparse = vec![
            crate::index::SparseSearchResult {
                id: "claim::c1".to_string(),
                score: 5.0,
            },
            crate::index::SparseSearchResult {
                id: "section::s1".to_string(),
                score: 3.0,
            },
        ];

        let fused = rrf_fuse(&dense, &sparse, 0.5);
        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn rrf_fuse_ignores_unparseable_sparse_ids() {
        let dense = vec![];
        let sparse = vec![crate::index::SparseSearchResult {
            id: "not-a-vector-id".to_string(),
            score: 5.0,
        }];

        let fused = rrf_fuse(&dense, &sparse, 0.5);
        assert!(fused.is_empty());
    }

    // --- Hybrid search integration ---

    /// Mock sparse embedder that produces deterministic sparse vectors.
    struct MockSparseEmbedder;

    impl SparseEmbedder for MockSparseEmbedder {
        fn embed_sparse(&self, texts: &[&str]) -> Result<Vec<SparseVector>, IndexError> {
            Ok(texts
                .iter()
                .map(|t| {
                    // Hash text bytes into sparse indices
                    let mut indices = Vec::new();
                    let mut values = Vec::new();
                    for (i, b) in t.bytes().enumerate().take(10) {
                        indices.push((u32::from(b) + u32::try_from(i).unwrap()) % 100);
                        values.push(1.0);
                    }
                    SparseVector { indices, values }
                })
                .collect())
        }
    }

    #[test]
    fn hybrid_search_with_sparse_components() {
        let dim = 8;
        let embedder = MockEmbedder { dim };
        let index = HnswIndex::new(dim, 1000).unwrap();
        let sparse_embedder = MockSparseEmbedder;
        let sparse_index = InvertedIndex::new();

        // Insert into both indexes
        let texts = [
            ("claim::c1", "JWT tokens use RS256 signing"),
            ("section::s1", "rate limiting configuration"),
        ];

        for (id, text) in &texts {
            let vecs = embedder.embed(&[*text]).unwrap();
            index.insert(id, &vecs[0]).unwrap();

            let sparse_vecs = sparse_embedder.embed_sparse(&[*text]).unwrap();
            sparse_index
                .insert_sparse(id, &sparse_vecs[0].indices, &sparse_vecs[0].values)
                .unwrap();
        }

        let searcher = MultiResolutionSearch::new(&embedder, &index)
            .with_sparse(&sparse_embedder, &sparse_index);

        let config = SearchConfig {
            raw_k: 30,
            top_k: 10,
            sparse_weight: 0.5,
            rerank_top_k: None,
        };

        let results = searcher.search("JWT signing", config).unwrap();
        assert!(!results.is_empty());
        for r in &results {
            assert!(r.score > 0.0);
        }
    }

    #[test]
    fn hybrid_search_zero_weight_is_dense_only() {
        let dim = 8;
        let embedder = MockEmbedder { dim };
        let index = HnswIndex::new(dim, 1000).unwrap();

        let vecs = embedder.embed(&["test content"]).unwrap();
        index.insert("claim::c1", &vecs[0]).unwrap();

        // Even with sparse components attached, weight=0 should skip sparse
        let sparse_embedder = MockSparseEmbedder;
        let sparse_index = InvertedIndex::new();

        let searcher = MultiResolutionSearch::new(&embedder, &index)
            .with_sparse(&sparse_embedder, &sparse_index);

        let config = SearchConfig {
            raw_k: 30,
            top_k: 10,
            sparse_weight: 0.0,
            rerank_top_k: None,
        };

        let results = searcher.search("test", config).unwrap();
        assert!(!results.is_empty());
    }
}
