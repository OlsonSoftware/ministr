//! Integration tests for the cross-stage *consistency* invariants of the
//! staged ingestion pipeline (f-ingest-gov-cancellation / fg2).
//!
//! The pipeline is Parse (producer) → bounded mpsc → Embed (consumer). Two
//! consistency guarantees must hold so `SQLite` and the vector index never
//! disagree about whether a file was indexed:
//!
//! 1. **No partial document on failure** — if embedding fails mid-stream, every
//!    document the producer persisted is rolled back (records + vectors +
//!    file-hash), leaving the corpus empty rather than torn.
//! 2. **Clean cancellation** — a cancel only stops the producer queueing *new*
//!    parses; the consumer drains the channel to close, so every
//!    persisted-and-sent document still gets its vectors. A cancel never leaves
//!    a document without its embeddings.
//!
//! These exercise the public ingest API end-to-end with in-memory fakes.

use std::path::Path;

use ministr_core::error::{IndexError, IngestionError};
use ministr_core::index::{HnswIndex, VectorIndex};
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::storage::{SqliteStorage, Storage};
use tokio_util::sync::CancellationToken;

const DIM: usize = 16;

/// Deterministic, non-degenerate mock embedder (normalised hash-based vectors).
struct MockEmbedder;

impl ministr_core::embedding::Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; DIM];
                for (i, b) in t.bytes().enumerate() {
                    v[i % DIM] += f32::from(b) / 255.0;
                }
                // Guarantee a non-zero vector so the degenerate guard never
                // legitimately drops one (keeps the count assertions exact).
                v[0] += 1.0;
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                for x in &mut v {
                    *x /= norm;
                }
                v
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        DIM
    }
}

/// Embedder that always fails — drives the rollback path.
struct FailingEmbedder;

impl ministr_core::embedding::Embedder for FailingEmbedder {
    fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Err(IndexError::EmbeddingFailed {
            reason: "injected embed failure".to_owned(),
        })
    }

    fn dimension(&self) -> usize {
        DIM
    }
}

/// Write a small multi-file markdown corpus that yields real sections.
fn write_corpus(dir: &Path) {
    for (name, body) in [
        ("alpha.md", "Alpha covers the parsing stage in depth"),
        (
            "beta.md",
            "Beta describes the embedding stage and its queue",
        ),
        (
            "gamma.md",
            "Gamma explains the persistence stage and rollback",
        ),
    ] {
        std::fs::write(
            dir.join(name),
            format!("# {name}\n\nThis is a paragraph: {body}, with enough words to form a real indexable section body.\n"),
        )
        .expect("write fixture");
    }
}

#[tokio::test]
async fn embed_failure_rolls_back_to_consistent_state() {
    let tmp = tempfile::tempdir().unwrap();
    write_corpus(tmp.path());

    let storage = SqliteStorage::open_in_memory().expect("storage");
    let index = HnswIndex::new(DIM, 10_000).expect("index");
    let pipeline = IngestionPipeline::new();

    let result = pipeline
        .ingest_directory_with_embeddings(tmp.path(), &storage, &FailingEmbedder, &index)
        .await;

    assert!(result.is_err(), "an embed failure must surface as an error");

    // The no-partial-document invariant: every document the producer persisted
    // before the embed failure is rolled back, so neither store disagrees.
    let docs = storage.list_documents().await.expect("list documents");
    assert!(
        docs.is_empty(),
        "embed failure must roll back every partially-indexed document, found {}",
        docs.len()
    );
    assert_eq!(
        index.len(),
        0,
        "no vectors remain in the index after rollback"
    );
}

#[tokio::test]
async fn clean_run_leaves_storage_and_index_consistent() {
    let tmp = tempfile::tempdir().unwrap();
    write_corpus(tmp.path());

    let storage = SqliteStorage::open_in_memory().expect("storage");
    let index = HnswIndex::new(DIM, 10_000).expect("index");
    let pipeline = IngestionPipeline::new();

    let stats = pipeline
        .ingest_directory_with_embeddings(tmp.path(), &storage, &MockEmbedder, &index)
        .await
        .expect("clean ingest succeeds");

    assert!(stats.files_indexed >= 1, "the corpus was indexed");
    let docs = storage.list_documents().await.expect("list documents");
    assert_eq!(
        docs.len(),
        stats.files_indexed,
        "one document per indexed file"
    );
    assert!(stats.total_embeddings > 0, "embeddings were produced");
    assert_eq!(
        index.len(),
        stats.total_embeddings,
        "every produced embedding is live in the index — `SQLite` and index agree"
    );
}

#[tokio::test]
async fn pre_cancelled_token_indexes_nothing_and_stays_consistent() {
    let tmp = tempfile::tempdir().unwrap();
    write_corpus(tmp.path());

    let storage = SqliteStorage::open_in_memory().expect("storage");
    let index = HnswIndex::new(DIM, 10_000).expect("index");
    let pipeline = IngestionPipeline::new();

    let ct = CancellationToken::new();
    ct.cancel(); // tripped before any file is parsed

    let result = pipeline
        .ingest_directory_with_embeddings_rooted(
            tmp.path(),
            &storage,
            &MockEmbedder,
            &index,
            None,
            Some(&ct),
        )
        .await;

    // Cancellation surfaces as a distinct, non-torn outcome rather than a
    // silent partial success — and, critically, leaves both stores empty.
    assert!(
        matches!(result, Err(IngestionError::Cancelled)),
        "a pre-cancelled token yields Cancelled, got {result:?}"
    );
    let docs = storage.list_documents().await.expect("list documents");
    assert!(
        docs.is_empty(),
        "cancellation leaves no torn document state, found {}",
        docs.len()
    );
    assert_eq!(index.len(), 0, "no vectors written under cancellation");
}
