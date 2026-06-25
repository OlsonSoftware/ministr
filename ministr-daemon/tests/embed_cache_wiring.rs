//! ingest-embed-cache-wiring — the daemon ingest path routes embeds through
//! the per-corpus embedding cache (`CachedEmbedder` over the corpus's
//! `embedding_cache` table), bringing the daemon to parity with the CLI
//! surface (which has cached since).
//!
//! The load-bearing assertion is the cache TABLE: after a daemon-driven
//! ingest, the corpus's `content.db` must hold cached vectors. Before this
//! wiring the daemon embedded via the bare pooled embedder and the table
//! stayed empty, so the assertion fails on the unwired code by construction.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ministr_core::config::MinistrConfig;
use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_daemon::registry::CorpusRegistry;

/// Deterministic text-hash embedder that counts how many texts reached
/// inference. Text-deterministic (identical bytes → identical vector), like
/// a real model, so the cache's dedup is transparent.
struct CountingEmbedder {
    dim: usize,
    inferred: AtomicUsize,
}

impl Embedder for CountingEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        self.inferred.fetch_add(texts.len(), Ordering::Relaxed);
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

/// Poll the registry until the corpus has finished its initial ingest.
///
/// `Idle` doubles as "not yet started", so waiting on status alone races the
/// async kick-off; embeddings landing in the index disambiguates.
async fn wait_until_indexed(registry: &Arc<CorpusRegistry>, corpus_id: &str) {
    use ministr_api::corpus::IndexingStatus;
    for _ in 0..600 {
        if let Ok(handle) = registry.get(corpus_id).await {
            let info = handle.current_info().await;
            if info.embeddings_count > 0 && matches!(info.status, IndexingStatus::Idle) {
                return;
            }
            if let IndexingStatus::Error { message } = info.status {
                panic!("indexing failed: {message}");
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("corpus {corpus_id} did not finish indexing in time");
}

#[tokio::test]
async fn daemon_ingest_populates_the_per_corpus_embedding_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // A real source directory with enough prose to produce several sections
    // (short sections also make sec-summary == section byte-duplicates
    // likely, which the intra-batch dedup collapses).
    let src = tmp.path().join("project");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("guide.md"),
        "# Alpha\n\nShort alpha section body.\n\n\
         # Beta\n\nShort beta section body.\n\n\
         # Gamma\n\nShort gamma section body.\n",
    )
    .unwrap();
    let src_str = src.to_string_lossy().to_string();

    let config = MinistrConfig {
        data_dir: data_dir.clone(),
        ..MinistrConfig::default()
    };

    let counting = Arc::new(CountingEmbedder {
        dim: 16,
        inferred: AtomicUsize::new(0),
    });
    let embedder: Arc<dyn Embedder> = Arc::clone(&counting) as _;
    let registry = Arc::new(CorpusRegistry::new(
        embedder,
        "counting-mock:test".to_string(),
        config,
    ));

    let (corpus_id, _started) = registry
        .register(std::slice::from_ref(&src_str))
        .await
        .unwrap();
    wait_until_indexed(&registry, &corpus_id).await;

    // 1. The per-corpus embedding cache is populated: without the
    //    CachedEmbedder wiring in the daemon indexer this table is empty.
    let db_path = data_dir.join("corpora").join(&corpus_id).join("content.db");
    assert!(db_path.exists(), "corpus content.db should exist");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let cached_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM embedding_cache", [], |r| r.get(0))
        .unwrap();
    assert!(
        cached_rows > 0,
        "daemon ingest must populate the per-corpus embedding cache (got 0 rows)"
    );

    // 2. Every cached row was inferred at most once: the number of texts that
    //    reached the model can't exceed the number of distinct cached texts
    //    (intra-batch dedup + cache hits collapse the rest).
    let inferred = counting.inferred.load(Ordering::Relaxed);
    assert!(
        i64::try_from(inferred).unwrap() <= cached_rows,
        "inferred {inferred} texts but only {cached_rows} unique cached rows — \
         duplicates reached the model"
    );
}
