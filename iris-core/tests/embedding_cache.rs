//! Integration test: content-addressable embedding cache with warm-load.
//!
//! Verifies that the `CachedEmbedder` skips re-embedding unchanged chunks
//! on second ingestion, simulating a warm session restart.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use iris_core::embedding::Embedder;
use iris_core::embedding::cache::{CachedEmbedder, EmbeddingCache};
use iris_core::index::{HnswIndex, VectorIndex};
use iris_core::ingestion::IngestionPipeline;
use iris_core::storage::{SqliteStorage, Storage};

/// Mock embedder that counts total texts embedded.
struct CountingEmbedder {
    dim: usize,
    embed_count: AtomicUsize,
}

impl CountingEmbedder {
    fn new(dim: usize) -> Self {
        Self {
            dim,
            embed_count: AtomicUsize::new(0),
        }
    }

    fn total_embedded(&self) -> usize {
        self.embed_count.load(Ordering::Relaxed)
    }
}

impl Embedder for CountingEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, iris_core::error::IndexError> {
        self.embed_count.fetch_add(texts.len(), Ordering::Relaxed);
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

#[tokio::test]
async fn warm_load_skips_cached_embeddings() {
    let tmp = tempfile::tempdir().unwrap();
    let dim = 32;

    // Write a small Rust file to ingest
    let src_dir = tmp.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        r#"//! A simple library.

/// Greet someone by name.
pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

/// Add two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    )
    .unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let index = HnswIndex::new(dim, 10_000).unwrap();

    // First ingestion — all embeddings are fresh (cache misses)
    let inner1 = Arc::new(CountingEmbedder::new(dim));
    let inner1_ref = Arc::clone(&inner1);
    let cache1 = EmbeddingCache::new(storage.conn());
    let embedder1 = CachedEmbedder::new(inner1, cache1, "test-model");

    let pipeline = IngestionPipeline::new();
    let stats1 = pipeline
        .ingest_directory_with_embeddings(&src_dir, &storage, &embedder1, &index)
        .await
        .unwrap();

    assert!(stats1.files_indexed > 0, "should index at least one file");
    let first_pass_embeds = inner1_ref.total_embedded();
    assert!(
        first_pass_embeds > 0,
        "first pass should compute embeddings"
    );
    assert_eq!(
        embedder1.cache_hits(),
        0,
        "first pass should have zero cache hits"
    );

    // Simulate warm restart: delete all file hashes to force re-parse,
    // but keep the embedding cache populated.
    // This simulates a scenario where file content is re-parsed but
    // chunk text is unchanged — the cache should serve all embeddings.
    let hashes = storage.list_file_hashes().await.unwrap();
    for h in &hashes {
        storage.delete_file_hash(&h.path).await.unwrap();
    }
    // Also delete documents so they get re-inserted
    let docs = storage.list_documents().await.unwrap();
    for doc in &docs {
        // Delete vectors too
        let sections = storage.list_sections(&doc.id).await.unwrap();
        for sec in &sections {
            let _ = index.delete(&iris_core::types::VectorId::section(sec.id.as_ref()).to_string());
            let _ =
                index.delete(&iris_core::types::VectorId::sec_summary(sec.id.as_ref()).to_string());
        }
        storage.delete_document(&doc.id).await.unwrap();
    }
    // Delete symbols for the file
    let _ = storage.delete_symbols_for_file("lib.rs").await;
    let _ = storage.delete_refs_for_file("lib.rs").await;

    // Second ingestion — same content, so cache should serve everything
    let inner2 = Arc::new(CountingEmbedder::new(dim));
    let inner2_ref = Arc::clone(&inner2);
    let cache2 = EmbeddingCache::new(storage.conn());
    let embedder2 = CachedEmbedder::new(inner2, cache2, "test-model");

    let stats2 = pipeline
        .ingest_directory_with_embeddings(&src_dir, &storage, &embedder2, &index)
        .await
        .unwrap();

    assert!(stats2.files_indexed > 0, "should re-index the file");
    assert_eq!(
        inner2_ref.total_embedded(),
        0,
        "second pass should NOT call inner embedder — all from cache"
    );
    assert!(
        embedder2.cache_hits() > 0,
        "second pass should have cache hits"
    );
    assert_eq!(
        embedder2.cache_misses(),
        0,
        "second pass should have zero cache misses"
    );
}

#[tokio::test]
async fn cache_handles_modified_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dim = 32;

    let src_dir = tmp.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "/// Original function.\npub fn foo() -> i32 { 1 }\n",
    )
    .unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let index = HnswIndex::new(dim, 10_000).unwrap();
    let inner = Arc::new(CountingEmbedder::new(dim));
    let inner_ref = Arc::clone(&inner);
    let cache = EmbeddingCache::new(storage.conn());
    let embedder = CachedEmbedder::new(inner, cache, "test-model");

    let pipeline = IngestionPipeline::new();

    // First ingestion
    pipeline
        .ingest_directory_with_embeddings(&src_dir, &storage, &embedder, &index)
        .await
        .unwrap();
    let after_first = inner_ref.total_embedded();
    assert!(after_first > 0);

    // Modify the file content
    std::fs::write(
        src_dir.join("lib.rs"),
        "/// Modified function.\npub fn bar() -> i32 { 2 }\n",
    )
    .unwrap();

    // Touch the mtime to force re-check (content hash will differ)
    // Since content changed, this file will be re-ingested.
    // The new text will be a cache miss.
    let stats = pipeline
        .ingest_directory_with_embeddings(&src_dir, &storage, &embedder, &index)
        .await
        .unwrap();

    assert!(
        stats.files_indexed > 0,
        "modified file should be re-indexed"
    );
    assert!(
        inner_ref.total_embedded() > after_first,
        "modified content should cause new embeddings"
    );
}
