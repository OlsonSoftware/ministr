//! Background indexing orchestrator.
//!
//! Runs the iris-core ingestion pipeline for a registered corpus and
//! updates the corpus status in the registry. Separated from
//! [`CorpusRegistry`] to keep the registry focused on lifecycle
//! management (SRP).

use std::path::PathBuf;
use std::sync::Arc;

use iris_api::corpus::IndexingStatus;
use iris_core::ingestion::IngestionPipeline;
use iris_core::storage::Storage;
use tracing::{error, info, warn};

use crate::registry::CorpusRegistry;

/// Run the full ingestion pipeline for a corpus.
///
/// Updates the corpus status through `Idle -> Indexing -> Idle/Error`,
/// then persists the vector index to disk.
pub async fn run(registry: &CorpusRegistry, corpus_id: &str, paths: &[String]) {
    let (storage, embedder, index, index_dir, progress) = {
        let corpora = registry.corpora().read().await;
        let Some(handle) = corpora.get(corpus_id) else {
            return;
        };
        (
            Arc::clone(&handle.storage),
            Arc::clone(registry.embedder()),
            Arc::clone(&handle.index),
            handle.data_dir.join("index"),
            Arc::clone(&handle.progress),
        )
    };

    registry
        .set_status(
            corpus_id,
            IndexingStatus::Indexing {
                files_done: 0,
                files_total: 0,
            },
        )
        .await;

    let local_paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let pipeline = IngestionPipeline::new().with_progress(Arc::clone(&progress));

    match pipeline
        .ingest_paths_with_embeddings(&local_paths, &*storage, &*embedder, &*index)
        .await
    {
        Ok(stats) => {
            info!(
                corpus_id,
                files_indexed = stats.files_indexed,
                files_skipped = stats.files_skipped,
                sections = stats.total_sections,
                embeddings = stats.total_embeddings,
                "indexing complete"
            );

            if let Err(e) = index.persist(&index_dir) {
                error!(corpus_id, error = %e, "failed to persist vector index");
            }

            // Query storage for total counts (not just incremental stats)
            // so the UI shows correct numbers even when files were skipped.
            let total_files = match storage.document_count().await {
                Ok(n) => n,
                Err(e) => {
                    warn!(corpus_id, error = %e, "failed to count documents, using incremental stats");
                    stats.files_indexed
                }
            };
            let total_sections = match storage.section_count().await {
                Ok(n) => n,
                Err(e) => {
                    warn!(corpus_id, error = %e, "failed to count sections, using incremental stats");
                    stats.total_sections
                }
            };

            registry
                .update_stats(corpus_id, total_files, total_sections, index.len())
                .await;
        }
        Err(iris_core::error::IngestionError::Cancelled) => {
            info!(corpus_id, "indexing cancelled");
            registry.set_status(corpus_id, IndexingStatus::Idle).await;
        }
        Err(e) => {
            error!(corpus_id, error = %e, "indexing failed");
            registry
                .set_status(
                    corpus_id,
                    IndexingStatus::Error {
                        message: e.to_string(),
                    },
                )
                .await;
        }
    }
}

/// Spawn a file watcher for a corpus that re-indexes on file changes.
///
/// Debounces events with a 2-second cooldown to avoid re-indexing
/// on every keystroke during active editing.
pub fn spawn_watcher(registry: Arc<CorpusRegistry>, corpus_id: String, paths: Vec<String>) {
    tokio::spawn(async move {
        let watch_paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();

        let mut watcher = match iris_core::coherence::FileWatcher::new(&watch_paths) {
            Ok(w) => w,
            Err(e) => {
                error!(corpus_id, error = %e, "failed to start file watcher");
                return;
            }
        };

        info!(corpus_id, "file watcher started");

        loop {
            // Wait for at least one change event.
            let Some(_event) = watcher.recv().await else {
                info!(corpus_id, "file watcher channel closed");
                break;
            };

            // Debounce: drain any queued events and wait 2 seconds for quiet.
            while let Ok(Some(_)) =
                tokio::time::timeout(std::time::Duration::from_secs(2), watcher.recv()).await
            {
                // more events, keep draining
            }

            info!(corpus_id, "file changes detected, re-indexing");
            run(&registry, &corpus_id, &paths).await;
        }
    });
}
