//! Multi-resolution query pipeline.
//!
//! Searches across all three resolution levels (summaries, sections, claims),
//! merges results, and applies resolution-aware scoring. The query pipeline
//! embeds the query text, searches the vector index, then enriches results
//! with metadata from storage.

use tracing::instrument;

use crate::error::IndexError;
use crate::index::{SearchResult as RawSearchResult, VectorIndex};
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
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            raw_k: DEFAULT_RAW_K,
            top_k: 10,
        }
    }
}

/// Multi-resolution search over a vector index.
///
/// Embeds a query, retrieves candidates from the vector index, applies
/// resolution-aware scoring, and returns ranked results.
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
}

impl<'a, E, I> MultiResolutionSearch<'a, E, I>
where
    E: crate::embedding::Embedder + ?Sized,
    I: VectorIndex + ?Sized,
{
    /// Create a new multi-resolution search pipeline.
    #[must_use]
    pub fn new(embedder: &'a E, index: &'a I) -> Self {
        Self { embedder, index }
    }

    /// Search across all resolution levels and return scored, ranked results.
    ///
    /// The query is embedded, then `raw_k` candidates are retrieved from the
    /// vector index. Each candidate is scored based on its distance and
    /// resolution level, then the top `top_k` results are returned.
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

        let mut scored: Vec<ScoredResult> = raw_results
            .iter()
            .filter_map(|r| score_result(r, is_broad_query))
            .collect();

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
    use crate::embedding::Embedder;
    use crate::index::{HnswIndex, VectorIndex};

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
}
