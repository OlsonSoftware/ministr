//! Regression: watcher-driven re-ingestion must flush the per-corpus
//! prefetch cache so stale warm entries aren't served after file edits.
//!
//! `PrefetchEngine::invalidate` and `PrefetchEngine::clear_cache` are
//! documented as "called by the coherence engine when source files change"
//! but both had zero production callers. `indexer::run` re-ingested files
//! but never notified the prefetch layer. This test exercises the public
//! `indexer::run` + file-change path and asserts a cache entry for a
//! re-ingested section is evicted afterward.
#![allow(clippy::missing_panics_doc)]

use std::path::PathBuf;
use std::sync::Arc;

use ministr_api::corpus::{CorpusInfo, IndexingStatus};
use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::{HnswIndex, VectorIndex};
use ministr_core::ingestion::IngestionProgress;
use ministr_core::service::QueryService;
use ministr_core::session::prefetch::{CacheEntry, PrefetchEngine, PrefetchStrategy};
use ministr_core::session::{SessionRegistry, UsageConfig};
use ministr_core::storage::SqliteStorage;
use ministr_core::types::Resolution;
use ministr_daemon::registry::{CorpusHandle, CorpusRegistry};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

struct HashEmbedder {
    dim: usize,
}

impl Embedder for HashEmbedder {
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
                } else {
                    // Avoid a zero vector, which some HNSW configurations
                    // reject as ill-conditioned.
                    v[0] = 1.0;
                }
                v
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

fn build_handle(
    corpus_id: &str,
    storage: Arc<SqliteStorage>,
    index: Arc<dyn VectorIndex>,
    service: QueryService,
    data_dir: PathBuf,
    paths: Vec<String>,
) -> CorpusHandle {
    CorpusHandle {
        info: Arc::new(RwLock::new(CorpusInfo {
            id: corpus_id.to_string(),
            display_name: corpus_id.to_string(),
            paths,
            status: IndexingStatus::Idle,
            files_indexed: 0,
            sections_count: 0,
            embeddings_count: 0,
            active_sessions: 0,
            last_indexed: None,
            symbols_count: 0,
        })),
        storage,
        index,
        service,
        sessions: Arc::new(tokio::sync::Mutex::new(SessionRegistry::new(
            UsageConfig::default(),
        ))),
        prefetch: Arc::new(tokio::sync::Mutex::new(
            PrefetchEngine::with_default_capacity(),
        )),
        progress: Arc::new(IngestionProgress::new()),
        cancel: CancellationToken::new(),
        data_dir,
        tasks: Arc::new(std::sync::Mutex::new(Vec::new())),
        coherence_tx: tokio::sync::broadcast::channel(16).0,
    }
}

#[tokio::test]
async fn prefetch_cache_cleared_after_watcher_reingest() {
    let tmp = tempfile::TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus_dir).unwrap();
    let doc_path = corpus_dir.join("auth.md");
    std::fs::write(
        &doc_path,
        "# Auth\n\n## Tokens\n\nJWT tokens use RS256 signing.\n",
    )
    .unwrap();

    let db_path = tmp.path().join("content.db");
    let storage = Arc::new(SqliteStorage::open(&db_path).unwrap());
    let dim = 16;
    let embedder: Arc<dyn Embedder> = Arc::new(HashEmbedder { dim });
    let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 1_000).unwrap());

    // Separate storage connection for the QueryService — matches what
    // the production daemon does so both share the DB file.
    let query_storage = SqliteStorage::open(&db_path).unwrap();
    let service = QueryService::new(query_storage, Arc::clone(&embedder), Arc::clone(&index));

    let corpus_id = "prefetch-test".to_string();
    let paths_vec: Vec<String> = vec![corpus_dir.to_string_lossy().into_owned()];
    let handle = build_handle(
        &corpus_id,
        Arc::clone(&storage),
        Arc::clone(&index),
        service,
        tmp.path().to_path_buf(),
        paths_vec.clone(),
    );

    let config = ministr_core::config::MinistrConfig {
        data_dir: tmp.path().to_path_buf(),
        ..ministr_core::config::MinistrConfig::default()
    };
    let registry = Arc::new(CorpusRegistry::new(Arc::clone(&embedder), config));
    registry
        .corpora()
        .write()
        .await
        .insert(corpus_id.clone(), std::sync::Arc::new(handle));

    // Initial ingestion.
    ministr_daemon::indexer::run(&registry, &corpus_id, &paths_vec).await;

    // Manually warm the prefetch cache with a section's pre-edit text —
    // this simulates a prior `trigger_prefetch` that pre-warmed the section.
    let section_id = "auth.md#tokens".to_string();
    {
        let corpora = registry.corpora().read().await;
        let handle = corpora.get(&corpus_id).unwrap();
        let mut prefetch = handle.prefetch.lock().await;
        prefetch.cache_mut().insert_default(
            section_id.clone(),
            CacheEntry {
                content_id: section_id.clone(),
                text: "JWT tokens use RS256 signing.".into(),
                token_count: 8,
                heading_path: Some(vec!["Auth".into(), "Tokens".into()]),
                summary: None,
                resolution: Resolution::Section,
                claims_available: 0,
                strategy: PrefetchStrategy::Sequential,
            },
        );
        assert!(
            prefetch.cache().peek(&section_id).is_some(),
            "sanity: warmed entry should be present"
        );
    }

    // Simulate the editor overwriting the file — semantic content changed.
    std::fs::write(
        &doc_path,
        "# Auth\n\n## Tokens\n\nJWT tokens now use EdDSA signing with 30-minute expiry.\n",
    )
    .unwrap();

    // Drive a watcher-style re-ingestion through the same public path the
    // file watcher uses. This is what `spawn_watcher` does after a batch
    // of events.
    ministr_daemon::indexer::run(&registry, &corpus_id, &paths_vec).await;

    // The regression: without invalidation wiring, the old warm entry
    // survives and would be served on the next read.
    let corpora = registry.corpora().read().await;
    let handle = corpora.get(&corpus_id).unwrap();
    let prefetch = handle.prefetch.lock().await;
    assert!(
        prefetch.cache().peek(&section_id).is_none(),
        "prefetch cache should be flushed after watcher-driven re-ingest; \
         found stale entry with pre-edit text",
    );
}
