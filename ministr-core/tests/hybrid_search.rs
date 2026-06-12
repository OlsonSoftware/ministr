//! End-to-end hybrid (dense + sparse) retrieval test.
//!
//! The dense (HNSW) and sparse (inverted-index) machinery plus the RRF fusion in
//! [`MultiResolutionSearch`] all exist, but nothing exercised the *fused* path
//! end-to-end (only `inverted.rs` unit tests). This pins the contract
//! deterministically with mock embedders and crafted indices (no model
//! download): a query whose exact identifier lives in one section is ranked #1
//! only once sparse fusion is enabled — the exact-identifier recovery that
//! hybrid retrieval buys for code (rq4).

use ministr_core::embedding::{Embedder, SparseEmbedder, SparseVector};
use ministr_core::error::IndexError;
use ministr_core::index::{HnswIndex, InvertedIndex, SparseIndex, VectorIndex};
use ministr_core::search::{MultiResolutionSearch, SearchConfig};
use ministr_core::types::VectorId;

/// Dense embedder that returns one fixed query vector regardless of input — lets
/// the test control dense ranking directly via the inserted document vectors.
struct FixedDenseEmbedder {
    query: Vec<f32>,
}

impl Embedder for FixedDenseEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts.iter().map(|_| self.query.clone()).collect())
    }

    fn dimension(&self) -> usize {
        self.query.len()
    }
}

/// Sparse embedder that activates one fixed vocabulary index (standing in for
/// the query's exact identifier token) regardless of input.
struct FixedSparseEmbedder {
    index: u32,
}

impl SparseEmbedder for FixedSparseEmbedder {
    fn embed_sparse(&self, texts: &[&str]) -> Result<Vec<SparseVector>, IndexError> {
        Ok(texts
            .iter()
            .map(|_| SparseVector {
                indices: vec![self.index],
                values: vec![1.0],
            })
            .collect())
    }
}

const TARGET: &str = "sym-rate_limiter::TokenBucket";
const DISTRACTOR_A: &str = "sym-cache::LruCache";
const DISTRACTOR_B: &str = "sym-retry::Backoff";
const IDENT_TOKEN: u32 = 7;

fn vid(content: &str) -> VectorId {
    VectorId::section(content)
}

#[test]
fn hybrid_recovers_exact_identifier_that_dense_misses() {
    // Dense index: craft cosine so a DISTRACTOR is closest to the query and the
    // TARGET is the farthest — dense-only must NOT rank the target first.
    let dense = HnswIndex::new(4, 1000).unwrap();
    dense
        .insert(vid(DISTRACTOR_A).as_str(), &[1.0, 0.0, 0.0, 0.0])
        .unwrap();
    dense
        .insert(vid(DISTRACTOR_B).as_str(), &[0.9, 0.1, 0.0, 0.0])
        .unwrap();
    dense
        .insert(vid(TARGET).as_str(), &[0.0, 1.0, 0.0, 0.0])
        .unwrap();

    // Sparse index: only the TARGET activates the query's identifier token.
    let sparse = InvertedIndex::new();
    sparse
        .insert_sparse(vid(TARGET).as_str(), &[IDENT_TOKEN], &[1.0])
        .unwrap();
    sparse
        .insert_sparse(vid(DISTRACTOR_A).as_str(), &[3], &[1.0])
        .unwrap();
    sparse
        .insert_sparse(vid(DISTRACTOR_B).as_str(), &[4], &[1.0])
        .unwrap();

    let dense_embedder = FixedDenseEmbedder {
        query: vec![1.0, 0.0, 0.0, 0.0],
    };
    let sparse_embedder = FixedSparseEmbedder { index: IDENT_TOKEN };

    let searcher =
        MultiResolutionSearch::new(&dense_embedder, &dense).with_sparse(&sparse_embedder, &sparse);

    // Dense-only (sparse_weight = 0): the distractor closest in vector space wins,
    // and the exact-identifier target is NOT first.
    let dense_only = SearchConfig {
        raw_k: 10,
        top_k: 3,
        sparse_weight: 0.0,
        rerank_top_k: None,
    };
    let dense_results = searcher.search("TokenBucket", dense_only).unwrap();
    assert!(!dense_results.is_empty(), "dense search returned nothing");
    assert_eq!(
        dense_results[0].vector_id.content_id(),
        DISTRACTOR_A,
        "dense-only should rank the vector-space-nearest distractor first"
    );
    assert_ne!(
        dense_results[0].vector_id.content_id(),
        TARGET,
        "dense-only should NOT recover the exact-identifier target"
    );

    // Hybrid (sparse_weight > 0): RRF fuses in the exact sparse match and
    // promotes the target to #1.
    let hybrid = SearchConfig {
        raw_k: 10,
        top_k: 3,
        sparse_weight: 0.7,
        rerank_top_k: None,
    };
    let hybrid_results = searcher.search("TokenBucket", hybrid).unwrap();
    assert_eq!(
        hybrid_results[0].vector_id.content_id(),
        TARGET,
        "hybrid RRF should recover the exact-identifier target to #1"
    );
}

#[test]
fn sparse_weight_is_inert_without_sparse_components() {
    // A dense-only searcher (no `with_sparse`) must ignore sparse_weight and not
    // panic — backward compatibility for the default survey path.
    let dense = HnswIndex::new(4, 1000).unwrap();
    dense
        .insert(vid(DISTRACTOR_A).as_str(), &[1.0, 0.0, 0.0, 0.0])
        .unwrap();
    dense
        .insert(vid(TARGET).as_str(), &[0.0, 1.0, 0.0, 0.0])
        .unwrap();

    let dense_embedder = FixedDenseEmbedder {
        query: vec![1.0, 0.0, 0.0, 0.0],
    };
    let searcher = MultiResolutionSearch::new(&dense_embedder, &dense);

    let cfg = SearchConfig {
        raw_k: 10,
        top_k: 2,
        sparse_weight: 0.7,
        rerank_top_k: None,
    };
    let results = searcher.search("anything", cfg).unwrap();
    assert_eq!(
        results[0].vector_id.content_id(),
        DISTRACTOR_A,
        "without sparse components, sparse_weight is ignored (pure dense)"
    );
}

/// Sparse embedder that activates the identifier token only for texts that
/// actually contain the identifier — content-sensitive, unlike
/// [`FixedSparseEmbedder`], so it can drive a real ingest.
struct ContentSparseEmbedder;

impl SparseEmbedder for ContentSparseEmbedder {
    fn embed_sparse(&self, texts: &[&str]) -> Result<Vec<SparseVector>, IndexError> {
        Ok(texts
            .iter()
            .map(|t| {
                if t.contains("TokenBucket") {
                    SparseVector {
                        indices: vec![IDENT_TOKEN],
                        values: vec![1.0],
                    }
                } else {
                    SparseVector {
                        indices: vec![3],
                        values: vec![1.0],
                    }
                }
            })
            .collect())
    }
}

/// rq4c — the production `QueryService::survey` path honors the configured
/// `sparse_weight`: the SAME ingested corpus answers differently with hybrid
/// fusion on (the exact-identifier doc wins) vs dense-only (the dense
/// tie-break picks the alphabetically-first content id, NOT the target).
///
/// The dense embedder returns one fixed vector for everything, so all dense
/// scores tie and the W2 deterministic tie-break (`content_id` ascending)
/// decides — `aaa.md`'s sections sort before `zzz.md`'s. Only the sparse
/// signal distinguishes the `TokenBucket` doc.
#[tokio::test]
async fn query_service_survey_honors_the_configured_sparse_weight() {
    use std::sync::Arc;

    use ministr_core::ingestion::IngestionPipeline;
    use ministr_core::service::QueryService;
    use ministr_core::storage::SqliteStorage;

    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("aaa.md"),
        "# Cache\n\nGeneric caching notes without the identifier.\n",
    )
    .unwrap();
    std::fs::write(
        src.join("zzz.md"),
        "# Limiter\n\nThe TokenBucket rate limiter refills per tick.\n",
    )
    .unwrap();

    // File-backed storage so the QueryService can open its own connection.
    let db_path = tmp.path().join("content.db");
    let storage = SqliteStorage::open(&db_path).unwrap();

    let dense_embedder = Arc::new(FixedDenseEmbedder {
        query: vec![1.0, 0.0, 0.0, 0.0],
    });
    let dense_index = Arc::new(HnswIndex::new(4, 1000).unwrap());
    let sparse_embedder: Arc<dyn SparseEmbedder> = Arc::new(ContentSparseEmbedder);
    let sparse_index: Arc<dyn SparseIndex> = Arc::new(InvertedIndex::new());

    let pipeline = IngestionPipeline::new()
        .with_sparse_indexing(Arc::clone(&sparse_embedder), Arc::clone(&sparse_index));
    pipeline
        .ingest_directory_with_embeddings(&src, &storage, dense_embedder.as_ref(), &*dense_index)
        .await
        .expect("ingest");

    let dense_only = QueryService::new(
        SqliteStorage::open(&db_path).unwrap(),
        Arc::clone(&dense_embedder) as Arc<dyn Embedder>,
        Arc::clone(&dense_index) as Arc<dyn VectorIndex>,
    );
    let hybrid = QueryService::new(
        SqliteStorage::open(&db_path).unwrap(),
        Arc::clone(&dense_embedder) as Arc<dyn Embedder>,
        Arc::clone(&dense_index) as Arc<dyn VectorIndex>,
    )
    .with_sparse(sparse_embedder, sparse_index, 0.9);

    let dense_top = dense_only.survey("TokenBucket", 2).await.expect("survey");
    assert!(!dense_top.is_empty());
    assert!(
        !dense_top[0].text.contains("TokenBucket"),
        "dense-only ties break to the alphabetically-first doc, not the target"
    );

    let hybrid_top = hybrid.survey("TokenBucket", 2).await.expect("survey");
    assert!(!hybrid_top.is_empty());
    assert!(
        hybrid_top[0].text.contains("TokenBucket"),
        "with sparse_weight=0.9 the exact-identifier doc must win, got: {}",
        hybrid_top[0].text
    );
}
