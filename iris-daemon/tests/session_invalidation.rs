//! Regression: watcher-driven re-ingestion must propagate section
//! invalidation into every session on the corpus.
//!
//! `SessionRegistry::invalidate_all` and the wrapper `Session::invalidate_sections`
//! are documented as the cross-session coherence path, but `iris_references`
//! showed zero production callers beyond `coherence::spawn_coherence_task` —
//! which itself has no callers. The daemon's `indexer::spawn_watcher`/`run`
//! pipeline flushes prefetch + re-ingests but never touches `handle.sessions`.
//! Consequence: after a file edit, delivered items stay "fresh" and no
//! `CoherenceAlert` is enqueued for the consumer.
//!
//! This test subscribes a session to a corpus, directly broadcasts a
//! `CoherenceEvent` (matching what `broadcast_events` does inside the
//! watcher), and asserts the session's stale set + pending alerts reflect it.
#![allow(clippy::missing_panics_doc, clippy::too_many_lines)]

use std::path::PathBuf;
use std::sync::Arc;

use iris_api::coherence::{CoherenceEvent, CoherenceKind};
use iris_api::corpus::{CorpusInfo, IndexingStatus};
use iris_core::embedding::Embedder;
use iris_core::error::IndexError;
use iris_core::index::{HnswIndex, VectorIndex};
use iris_core::ingestion::IngestionProgress;
use iris_core::service::QueryService;
use iris_core::session::prefetch::PrefetchEngine;
use iris_core::session::{AccessMode, BudgetConfig, SessionRegistry};
use iris_core::storage::SqliteStorage;
use iris_core::types::{ContentId, Resolution};
use iris_daemon::registry::{CorpusHandle, CorpusRegistry};
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
        info: RwLock::new(CorpusInfo {
            id: corpus_id.to_string(),
            paths,
            status: IndexingStatus::Idle,
            files_indexed: 0,
            sections_count: 0,
            embeddings_count: 0,
            active_sessions: 0,
            last_indexed: None,
            symbols_count: 0,
        }),
        storage,
        index,
        service,
        sessions: tokio::sync::Mutex::new(SessionRegistry::new(BudgetConfig::default())),
        prefetch: Arc::new(tokio::sync::Mutex::new(
            PrefetchEngine::with_default_capacity(),
        )),
        progress: Arc::new(IngestionProgress::new()),
        cancel: CancellationToken::new(),
        data_dir,
        coherence_tx: tokio::sync::broadcast::channel(16).0,
    }
}

#[tokio::test]
async fn session_invalidation_propagates_on_coherence_broadcast() {
    let tmp = tempfile::TempDir::new().unwrap();
    let corpus_dir = tmp.path().join("corpus");
    std::fs::create_dir_all(&corpus_dir).unwrap();

    let db_path = tmp.path().join("content.db");
    let storage = Arc::new(SqliteStorage::open(&db_path).unwrap());
    let dim = 16;
    let embedder: Arc<dyn Embedder> = Arc::new(HashEmbedder { dim });
    let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 1_000).unwrap());

    let query_storage = SqliteStorage::open(&db_path).unwrap();
    let service = QueryService::new(query_storage, Arc::clone(&embedder), Arc::clone(&index));

    let corpus_id = "session-invalidation-test".to_string();
    let paths_vec: Vec<String> = vec![corpus_dir.to_string_lossy().into_owned()];
    let handle = build_handle(
        &corpus_id,
        Arc::clone(&storage),
        Arc::clone(&index),
        service,
        tmp.path().to_path_buf(),
        paths_vec.clone(),
    );

    // Grab the broadcast sender BEFORE the handle is moved — that's how
    // the production `register` path keeps a tx around too.
    let coherence_tx = handle.coherence_tx.clone();

    let config = iris_core::config::IrisConfig {
        data_dir: tmp.path().to_path_buf(),
        ..iris_core::config::IrisConfig::default()
    };
    let registry = Arc::new(CorpusRegistry::new(Arc::clone(&embedder), config));
    registry
        .corpora()
        .write()
        .await
        .insert(corpus_id.clone(), handle);

    // Wire the session invalidator the same way `register` does.
    iris_daemon::registry::spawn_session_invalidator(Arc::clone(&registry), corpus_id.clone());
    // Give the spawned task a chance to acquire its broadcast subscriber
    // before we send — `broadcast::send` errors if no receivers exist.
    for _ in 0..50 {
        tokio::task::yield_now().await;
        if coherence_tx.receiver_count() > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        coherence_tx.receiver_count() > 0,
        "invalidator task must have subscribed to coherence_tx"
    );

    // Mint a session and deliver a section to it.
    let sid = "s1".to_string();
    let section_id = "auth.md#tokens".to_string();
    {
        let corpora = registry.corpora().read().await;
        let handle = corpora.get(&corpus_id).unwrap();
        let mut sessions = handle.sessions.lock().await;
        let entry = sessions.get_or_create(&sid, None, AccessMode::ReadWrite);
        entry.session.record_delivery(
            &ContentId(section_id.clone()),
            Resolution::Section,
            8,
            1,
            "hash-before".to_string(),
        );
        assert!(
            !entry.session.is_stale(&ContentId(section_id.clone())),
            "sanity: delivered item should not start stale"
        );
    }

    // Broadcast a coherence event exactly as the watcher does after a
    // re-ingest. This is the payload `broadcast_events` emits.
    coherence_tx
        .send(CoherenceEvent {
            timestamp_ms: 1,
            corpus_id: corpus_id.clone(),
            kind: CoherenceKind::Modified,
            path: "auth.md".to_string(),
            affected_sections: vec![section_id.clone()],
            duration_ms: 0,
        })
        .expect("broadcast send");

    // The invalidator runs asynchronously — poll a short bounded window.
    let mut observed = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let corpora = registry.corpora().read().await;
        let handle = corpora.get(&corpus_id).unwrap();
        let sessions = handle.sessions.lock().await;
        if let Some(entry) = sessions.get_session(&sid)
            && entry.session.is_stale(&ContentId(section_id.clone()))
        {
            observed = true;
            break;
        }
    }

    assert!(
        observed,
        "session.is_stale should return true after a coherence broadcast affecting the section"
    );

    // And a CoherenceAlert must have been enqueued so the MCP client can
    // surface drift to the caller.
    let corpora = registry.corpora().read().await;
    let handle = corpora.get(&corpus_id).unwrap();
    let mut sessions = handle.sessions.lock().await;
    let entry = sessions.get_or_create(&sid, None, AccessMode::ReadWrite);
    let alerts = entry.session.drain_alerts();
    assert!(
        !alerts.is_empty(),
        "at least one CoherenceAlert must be pending after invalidation"
    );
    assert!(
        alerts[0].stale_content_ids.contains(&section_id),
        "alert should name the invalidated section"
    );
}
