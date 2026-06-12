//! rq4b — sparse (hybrid) population during ingestion.
//!
//! With `IngestionPipeline::with_sparse_indexing`, every `(VectorId, text)`
//! pair the embed stage dense-embeds is also sparse-embedded into the
//! inverted index, the sidecar persists next to the HNSW files, and document
//! deletion (re-index replace / stale-file sweep) mirrors into the sparse
//! index. Without the builder call, ingestion stays dense-only.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ministr_core::embedding::{Embedder, SparseEmbedder, SparseVector};
use ministr_core::error::IndexError;
use ministr_core::index::{HnswIndex, InvertedIndex, SparseIndex, VectorIndex};
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::storage::SqliteStorage;

/// Deterministic dense mock (text-hash unit vectors, non-degenerate).
struct MockDense {
    dim: usize,
}

impl Embedder for MockDense {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                v[0] = 1.0;
                for (i, b) in t.bytes().enumerate() {
                    v[i % self.dim] += f32::from(b) / 255.0;
                }
                v
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

/// Deterministic sparse mock: each text maps its byte values to term ids, and
/// counts how many texts reached sparse inference.
struct MockSparse {
    embedded: AtomicUsize,
}

impl SparseEmbedder for MockSparse {
    fn embed_sparse(&self, texts: &[&str]) -> Result<Vec<SparseVector>, IndexError> {
        self.embedded.fetch_add(texts.len(), Ordering::Relaxed);
        Ok(texts
            .iter()
            .map(|t| {
                let mut indices: Vec<u32> = t
                    .bytes()
                    .take(16)
                    .enumerate()
                    .map(|(i, b)| (u32::from(b) * 7 + u32::try_from(i).unwrap()) % 997)
                    .collect();
                indices.sort_unstable();
                indices.dedup();
                let values = vec![1.0f32; indices.len()];
                SparseVector { indices, values }
            })
            .collect())
    }
}

fn write_corpus(dir: &std::path::Path) {
    std::fs::write(
        dir.join("alpha.md"),
        "# Alpha\n\nThe alpha document talks about quorum leases.\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("beta.md"),
        "# Beta\n\nThe beta document covers shard watermarks.\n",
    )
    .unwrap();
}

#[tokio::test]
async fn sparse_indexing_populates_inverted_index_for_every_dense_vector() {
    let tmp = tempfile::tempdir().unwrap();
    write_corpus(tmp.path());

    let dense = MockDense { dim: 8 };
    let index = HnswIndex::new(8, 1000).unwrap();
    let storage = SqliteStorage::open_in_memory().unwrap();
    let sparse_embedder = Arc::new(MockSparse {
        embedded: AtomicUsize::new(0),
    });
    let sparse_index: Arc<InvertedIndex> = Arc::new(InvertedIndex::new());

    let pipeline = IngestionPipeline::new().with_sparse_indexing(
        Arc::clone(&sparse_embedder) as Arc<dyn SparseEmbedder>,
        Arc::clone(&sparse_index) as Arc<dyn SparseIndex>,
    );
    let stats = pipeline
        .ingest_directory_with_embeddings(tmp.path(), &storage, &dense, &index)
        .await
        .expect("ingest");

    assert!(stats.total_embeddings > 0, "corpus produced embeddings");
    // Every dense vector has a sparse twin: identical id coverage.
    assert_eq!(
        sparse_index.len_sparse(),
        index.len(),
        "sparse and dense indexes must cover the same vectors"
    );
    assert!(
        sparse_embedder.embedded.load(Ordering::Relaxed) >= index.len(),
        "every embedded pair reached the sparse embedder"
    );
}

#[tokio::test]
async fn dense_only_ingestion_leaves_no_sparse_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    write_corpus(tmp.path());

    let dense = MockDense { dim: 8 };
    let index = HnswIndex::new(8, 1000).unwrap();
    let storage = SqliteStorage::open_in_memory().unwrap();

    // No with_sparse_indexing: the dense-only pipeline must not write a
    // sparse sidecar even when a corpus dir is configured.
    let corpus_dir = tmp.path().join("corpus");
    let pipeline = IngestionPipeline::new().with_corpus_dir(corpus_dir.clone());
    pipeline
        .ingest_directory_with_embeddings(tmp.path(), &storage, &dense, &index)
        .await
        .expect("ingest");

    assert!(
        !corpus_dir.join("sparse_index.json").exists(),
        "dense-only ingest must not create a sparse sidecar"
    );
}

#[tokio::test]
async fn sparse_sidecar_persists_next_to_the_corpus_dir_and_reloads() {
    let tmp = tempfile::tempdir().unwrap();
    write_corpus(tmp.path());
    let corpus_dir = tmp.path().join("corpus");

    let dense = MockDense { dim: 8 };
    let index = HnswIndex::new(8, 1000).unwrap();
    let storage = SqliteStorage::open_in_memory().unwrap();
    let sparse_index: Arc<InvertedIndex> = Arc::new(InvertedIndex::new());

    let pipeline = IngestionPipeline::new()
        .with_corpus_dir(corpus_dir.clone())
        .with_sparse_indexing(
            Arc::new(MockSparse {
                embedded: AtomicUsize::new(0),
            }),
            Arc::clone(&sparse_index) as Arc<dyn SparseIndex>,
        );
    pipeline
        .ingest_directory_with_embeddings(tmp.path(), &storage, &dense, &index)
        .await
        .expect("ingest");

    assert!(
        corpus_dir.join("sparse_index.json").exists(),
        "sparse sidecar persisted at end of ingest"
    );
    let reloaded = InvertedIndex::load_sparse(&corpus_dir).expect("load sidecar");
    assert_eq!(
        reloaded.len_sparse(),
        sparse_index.len_sparse(),
        "reloaded sidecar covers the same vectors"
    );
}

#[tokio::test]
async fn removed_file_is_swept_from_the_sparse_index_on_reingest() {
    let tmp = tempfile::tempdir().unwrap();
    write_corpus(tmp.path());

    let dense = MockDense { dim: 8 };
    let index = HnswIndex::new(8, 1000).unwrap();
    let storage = SqliteStorage::open_in_memory().unwrap();
    let sparse_index: Arc<InvertedIndex> = Arc::new(InvertedIndex::new());

    let make_pipeline = || {
        IngestionPipeline::new().with_sparse_indexing(
            Arc::new(MockSparse {
                embedded: AtomicUsize::new(0),
            }),
            Arc::clone(&sparse_index) as Arc<dyn SparseIndex>,
        )
    };

    make_pipeline()
        .ingest_directory_with_embeddings(tmp.path(), &storage, &dense, &index)
        .await
        .expect("first ingest");
    let with_both = sparse_index.len_sparse();
    assert!(with_both > 0);

    // Remove beta.md and re-ingest: the stale-document sweep must tombstone
    // beta's sparse entries exactly as it deletes its dense vectors.
    std::fs::remove_file(tmp.path().join("beta.md")).unwrap();
    make_pipeline()
        .ingest_directory_with_embeddings(tmp.path(), &storage, &dense, &index)
        .await
        .expect("re-ingest");

    assert_eq!(
        sparse_index.len_sparse(),
        index.len(),
        "after sweeping the removed file, sparse and dense coverage match again"
    );
    assert!(
        sparse_index.len_sparse() < with_both,
        "beta's sparse entries are gone"
    );
}

/// End-to-end hybrid retrieval with the REAL zero-model AST encoder
/// (rq-ast-sparse-encoder): ingest through the production pipeline, then an
/// exact-identifier query must surface the right document via the sparse leg
/// — no model download, fully deterministic, CI-safe.
#[tokio::test]
async fn ast_encoder_hybrid_finds_exact_identifier_end_to_end() {
    use ministr_core::embedding::AstSparseEncoder;
    use ministr_core::search::{MultiResolutionSearch, SearchConfig};

    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("retry.rs"),
        "/// Computes the exponential backoff delay.\npub fn compute_backoff_delay(attempt: u32) -> u64 {\n    1u64 << attempt\n}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("quorum.rs"),
        "/// Tracks quorum lease ownership.\npub fn quorum_lease_owner(node: u32) -> u32 {\n    node\n}\n",
    )
    .unwrap();

    let dense = MockDense { dim: 8 };
    let index = HnswIndex::new(8, 1000).unwrap();
    let storage = SqliteStorage::open_in_memory().unwrap();
    let sparse_index: Arc<InvertedIndex> = Arc::new(InvertedIndex::new());
    let encoder = Arc::new(AstSparseEncoder::new(Arc::clone(&sparse_index)));

    IngestionPipeline::new()
        .with_sparse_indexing(
            Arc::clone(&encoder) as Arc<dyn SparseEmbedder>,
            Arc::clone(&sparse_index) as Arc<dyn SparseIndex>,
        )
        .ingest_directory_with_embeddings(tmp.path(), &storage, &dense, &index)
        .await
        .expect("ingest");
    assert!(sparse_index.len_sparse() > 0, "sparse index populated");

    // The dense mock is near-meaningless (byte hashes), so a hybrid hit on
    // the exact identifier demonstrates the AST sparse leg doing the work.
    let searcher = MultiResolutionSearch::new(&dense, &index)
        .with_sparse(encoder.as_ref(), sparse_index.as_ref());
    let config = SearchConfig {
        raw_k: 30,
        top_k: 5,
        sparse_weight: 0.6,
        rerank_top_k: None,
    };
    let results = searcher
        .search("compute_backoff_delay", config)
        .expect("hybrid search");
    assert!(
        results
            .first()
            .is_some_and(|r| r.vector_id.content_id().contains("retry.rs")),
        "exact-identifier query must rank the defining file first; got {:?}",
        results
            .iter()
            .map(|r| r.vector_id.content_id())
            .collect::<Vec<_>>()
    );

    // Determinism: the same query twice returns the identical ranking.
    let again = searcher
        .search("compute_backoff_delay", config)
        .expect("again");
    let ids = |rs: &[ministr_core::search::ScoredResult]| -> Vec<String> {
        rs.iter()
            .map(|r| r.vector_id.as_str().to_string())
            .collect()
    };
    assert_eq!(
        ids(&results),
        ids(&again),
        "hybrid ranking is deterministic"
    );
}
