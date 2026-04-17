//! Background indexing orchestrator.
//!
//! Runs the iris-core ingestion pipeline for a registered corpus and
//! updates the corpus status in the registry. Separated from
//! [`CorpusRegistry`] to keep the registry focused on lifecycle
//! management (SRP).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use iris_api::coherence::{CoherenceEvent, CoherenceKind};
use iris_api::corpus::IndexingStatus;
use iris_core::coherence::CoherenceEvent as CoreCoherenceEvent;
use iris_core::ingestion::IngestionPipeline;
use iris_core::storage::Storage;
use iris_core::types::ContentId;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::registry::CorpusRegistry;

/// How long the watcher waits for a *quiet* gap between events before
/// treating the burst as finished.
const DEBOUNCE_QUIET: Duration = Duration::from_secs(2);

/// Ceiling on a single debounce batch. Under sustained editing (where
/// events keep arriving within the quiet window) this forces a reindex
/// anyway so the observatory stays live.
const DEBOUNCE_MAX_WINDOW: Duration = Duration::from_secs(10);

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

            // Update symbol count separately (not part of ingestion stats).
            let total_symbols = match storage.symbol_count().await {
                Ok(n) => n,
                Err(e) => {
                    warn!(corpus_id, error = %e, "failed to count symbols");
                    0
                }
            };
            registry
                .update_symbols_count(corpus_id, total_symbols)
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
/// - Debounces events with [`DEBOUNCE_QUIET`] of silence between bursts.
/// - Caps a single batch at [`DEBOUNCE_MAX_WINDOW`] so sustained editing
///   (saves faster than the quiet window) can't indefinitely starve
///   reindex.
/// - Honors `cancel` so unregistering the corpus stops the watcher
///   instead of leaking an inotify/FSEvents subscription.
/// - Broadcasts a rich [`CoherenceEvent`] for each distinct file in the
///   batch so subscribers (observatory feed, answer-cache invalidator)
///   can render path-centric rows and invalidate targeted entries.
pub fn spawn_watcher(
    registry: Arc<CorpusRegistry>,
    corpus_id: String,
    paths: Vec<String>,
    cancel: CancellationToken,
) {
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
            // Wait for the first event of a new batch, with cancellation.
            let first = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    info!(corpus_id, "file watcher cancelled — corpus unregistered");
                    break;
                }
                event = watcher.recv() => {
                    if let Some(ev) = event {
                        ev
                    } else {
                        info!(corpus_id, "file watcher channel closed");
                        break;
                    }
                }
            };
            let mut batch: Vec<CoreCoherenceEvent> = vec![first];

            // Debounce: collect events until either we see `DEBOUNCE_QUIET`
            // of silence OR the batch has been accumulating longer than
            // `DEBOUNCE_MAX_WINDOW`. The max-window cap is what guarantees
            // reindex fires even under back-to-back saves faster than the
            // quiet gap.
            let batch_start = tokio::time::Instant::now();
            loop {
                if batch_start.elapsed() >= DEBOUNCE_MAX_WINDOW {
                    break;
                }
                let remaining = DEBOUNCE_MAX_WINDOW.saturating_sub(batch_start.elapsed());
                let wait = DEBOUNCE_QUIET.min(remaining);
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => break,
                    res = tokio::time::timeout(wait, watcher.recv()) => match res {
                        Ok(Some(ev)) => batch.push(ev),
                        Ok(None) | Err(_) => break,
                    }
                }
            }

            if cancel.is_cancelled() {
                info!(corpus_id, "file watcher cancelled mid-debounce");
                break;
            }

            // Coalesce by (path, kind) — keep the latest event per unique key.
            let mut latest: std::collections::HashMap<(PathBuf, &'static str), CoreCoherenceEvent> =
                std::collections::HashMap::new();
            for ev in batch {
                let key = (ev.path().to_path_buf(), kind_key(&ev));
                latest.insert(key, ev);
            }

            info!(
                corpus_id,
                changed = latest.len(),
                "file changes detected, re-indexing"
            );

            // Broadcast one rich event per distinct (path, kind) before
            // re-indexing so the UI shows activity without waiting for
            // ingestion to finish.
            broadcast_events(&registry, &corpus_id, latest.into_values().collect()).await;

            run(&registry, &corpus_id, &paths).await;
        }
    });
}

/// Stable kind tag for coalescing — map the enum to one of three strings.
fn kind_key(event: &CoreCoherenceEvent) -> &'static str {
    match event {
        CoreCoherenceEvent::Created(_) => "created",
        CoreCoherenceEvent::Modified(_) => "modified",
        CoreCoherenceEvent::Removed(_) => "removed",
    }
}

/// Broadcast each watcher event onto the corpus's coherence channel,
/// stamping in the timestamp and the pre-change affected section IDs.
async fn broadcast_events(
    registry: &CorpusRegistry,
    corpus_id: &str,
    events: Vec<CoreCoherenceEvent>,
) {
    // Snapshot everything we need from the handle up front so we release
    // the registry lock before the per-event storage queries.
    let (storage, tx) = {
        let corpora = registry.corpora().read().await;
        let Some(handle) = corpora.get(corpus_id) else {
            return;
        };
        (Arc::clone(&handle.storage), handle.coherence_tx.clone())
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));

    for ev in events {
        let path = ev.path().to_string_lossy().into_owned();
        let kind = match ev {
            CoreCoherenceEvent::Created(_) => CoherenceKind::Created,
            CoreCoherenceEvent::Modified(_) => CoherenceKind::Modified,
            CoreCoherenceEvent::Removed(_) => CoherenceKind::Removed,
        };
        let affected = match kind {
            // Newly-created files have no pre-existing sections to invalidate.
            CoherenceKind::Created => Vec::new(),
            CoherenceKind::Modified | CoherenceKind::Removed => {
                affected_sections_for(&storage, &path).await
            }
        };

        let event = CoherenceEvent {
            timestamp_ms: now_ms,
            corpus_id: corpus_id.to_string(),
            kind,
            path,
            affected_sections: affected,
            duration_ms: 0,
        };
        // `send` errors only when there are no subscribers — ignore.
        let _ = tx.send(event);
    }
}

/// Look up the section IDs currently indexed under `source_path`.
///
/// Does a **two-pass** path match: first a fast exact-string match against
/// `doc.source_path`, then a canonical-path fallback for any doc that
/// didn't match (handles macOS `/tmp` vs `/private/tmp` symlinks, case
/// normalization on HFS, absolute-vs-relative corpus registration, etc.).
/// The canonical fallback only runs when no exact match was found, so the
/// common case is one `list_documents` + O(1) comparisons per doc.
///
/// Returns empty on any storage error so a missing-document case never
/// fails the broadcast.
async fn affected_sections_for(
    storage: &iris_core::storage::SqliteStorage,
    source_path: &str,
) -> Vec<String> {
    let Ok(docs) = storage.list_documents().await else {
        return Vec::new();
    };

    // Pass 1: exact string match (fast, covers the common case).
    let mut matched: Vec<&iris_core::storage::traits::DocumentRecord> = docs
        .iter()
        .filter(|d| d.source_path == source_path)
        .collect();

    // Pass 2: fall back to canonical-form equality if nothing matched.
    // `canonicalize` is a syscall per candidate; only run it when we have
    // to, and only on docs that didn't already match.
    if matched.is_empty()
        && let Ok(notify_canonical) = std::fs::canonicalize(source_path)
    {
        for doc in &docs {
            if doc.source_path == source_path {
                continue;
            }
            if let Ok(doc_canonical) = std::fs::canonicalize(&doc.source_path)
                && doc_canonical == notify_canonical
            {
                matched.push(doc);
            }
        }
    }

    let mut out = Vec::new();
    for doc in matched {
        if let Ok(sections) = storage.list_sections(&ContentId(doc.id.0.clone())).await {
            out.extend(sections.into_iter().map(|s| s.id.0));
        }
    }
    out
}
