//! Background indexing orchestrator.
//!
//! Runs the ministr-core ingestion pipeline for a registered corpus and
//! updates the corpus status in the registry. Separated from
//! [`CorpusRegistry`] to keep the registry focused on lifecycle
//! management (SRP).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ministr_api::coherence::{CoherenceEvent, CoherenceKind};
use ministr_api::corpus::IndexingStatus;
use ministr_core::coherence::CoherenceEvent as CoreCoherenceEvent;
use ministr_core::embedding::EmbeddingService;
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::storage::Storage;
use ministr_core::types::ContentId;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::embedder_pool::PooledEmbedder;
use crate::registry::{CorpusRegistry, apply_dimension};

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
/// then persists the vector index to disk. After a successful ingest the
/// per-corpus prefetch cache is flushed so that subsequent reads don't
/// serve stale warm entries for sections whose text was just rewritten.
// Sequential ingest pipeline; many distinct steps (status transitions,
// stats refresh, resolver heal, cloud durability hook). Extracting a
// helper just to satisfy the lint would obscure the flow.
#[allow(clippy::too_many_lines)]
pub async fn run(registry: &CorpusRegistry, corpus_id: &str, paths: &[String]) {
    let (
        storage,
        model,
        dimension,
        parser,
        min_section_tokens,
        index,
        data_dir,
        index_dir,
        progress,
        prefetch,
    ) = {
        let corpora = registry.corpora().read().await;
        let Some(handle) = corpora.get(corpus_id) else {
            return;
        };
        (
            Arc::clone(&handle.storage),
            handle.model.clone(),
            handle.dimension,
            handle.parser,
            handle.min_section_tokens,
            Arc::clone(&handle.index),
            handle.data_dir.clone(),
            handle.data_dir.join("index"),
            Arc::clone(&handle.progress),
            Arc::clone(&handle.prefetch),
        )
    };

    // cq-status: mark the corpus Queued BEFORE waiting on an indexing slot, so a
    // corpus sitting in the queue reports a distinct "queued" state rather than
    // keeping its prior (Idle/"indexed") status with 0 files — otherwise a
    // not-yet-started corpus misleadingly looks finished-but-empty. This
    // replaces the earlier band-aid (b44874e) that set `Indexing` before the
    // wait (which then misreported a queued corpus as actively indexing).
    registry.set_status(corpus_id, IndexingStatus::Queued).await;

    // Acquire an indexing slot (f-ingest-coordinator): the scheduler serializes
    // same-corpus indexing (a corpus never indexes concurrently with itself) and
    // bounds total concurrency across corpora. Held for the whole ingest +
    // persist + heal. This replaces the old INDEXING_SEMAPHORE(1) band-aid —
    // embedding now runs off the Tokio runtime via the shared EmbeddingService,
    // so multiple distinct corpora index concurrently up to the bound without
    // starving the runtime.
    let _slot = registry.scheduler().acquire(corpus_id).await;

    // The permit is granted — transition Queued → Indexing now that work is
    // actually starting. `merge_live_info` overlays live progress only on the
    // `Indexing` state, so the file/section counts begin updating from here.
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

    // parity-seam-registry-routing: ingest with the corpus's OWN embedder —
    // the same one `create_handle` bound this corpus's index + QueryService to
    // (resolved from its `.ministr.toml` `[corpus] model` via the shared seam,
    // recorded on `handle.model`). The default model returns the daemon's
    // seeded shared embedder; a per-corpus model is built + cached once. Ingest
    // and query therefore share one vector space per corpus — correct by
    // construction. By the time `run` fires, `create_handle` has already built
    // (or failed) this model, so the lookup is a cache hit; the error arm is a
    // defensive guard, not the normal path.
    let PooledEmbedder {
        embedder: pooled_embedder,
        service: pooled_service,
    } = match registry.embedder_for(&model) {
        Ok(pooled) => pooled,
        Err(e) => {
            error!(corpus_id, model = %model, error = %e, "embedder unavailable for corpus model — cannot index");
            registry
                .set_status(
                    corpus_id,
                    IndexingStatus::Error {
                        message: e.to_string(),
                    },
                )
                .await;
            return;
        }
    };
    if model != registry.config().default_model {
        info!(corpus_id, model = %model, "indexing with per-corpus embedding model");
    }

    // parity-registry-knobs: when the corpus configures a Matryoshka truncation
    // `dimension`, ingest must produce TRUNCATED vectors so they land in the
    // truncated-dim HNSW index `create_handle` built (via the same
    // `apply_dimension` seam) — otherwise ingest writes full-dim vectors into a
    // truncated index. We wrap the pooled embedder identically, then route it
    // through a fresh per-corpus `EmbeddingService` so the truncated embed still
    // runs off the Tokio runtime (ADR 0001 D1). The default-dimension path keeps
    // the shared, model-pooled service unchanged.
    let (embedder, service) = match apply_dimension(&pooled_embedder, dimension) {
        Ok((embedder, _dual)) => {
            let service = if dimension.is_some() {
                info!(
                    corpus_id,
                    dimension, "indexing with per-corpus Matryoshka truncation"
                );
                Arc::new(EmbeddingService::with_model(Arc::clone(&embedder)))
            } else {
                pooled_service
            };
            (embedder, service)
        }
        Err(e) => {
            error!(corpus_id, model = %model, error = %e, "invalid per-corpus dimension — cannot index");
            registry
                .set_status(
                    corpus_id,
                    IndexingStatus::Error {
                        message: e.to_string(),
                    },
                )
                .await;
            return;
        }
    };

    // ADR 0001 D1 (refined by parity-seam-registry-routing): route embedding
    // through a dedicated EmbeddingService so the synchronous, GPU-bound embed()
    // runs on its own thread and the pipeline's embed consumer never pins a
    // Tokio worker. Corpora sharing a model+dimension share one queue.
    //
    // parity-meta-toml-load: apply this corpus's resolved per-corpus `meta.toml`
    // knobs — `parser` (override auto-detection) + `min_section_tokens` (section
    // merge threshold) — the SAME knobs the CLI's `run_corpus_ingestion` applies,
    // so the two ingestion surfaces stay in parity.
    let pipeline = IngestionPipeline::new()
        .with_progress(Arc::clone(&progress))
        .with_parser_override(parser)
        .with_min_section_tokens(min_section_tokens)
        .with_embedding_service(service);

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

            // Flush the prefetch warm cache — any entries it holds were
            // parsed before this ingest ran and may no longer match what
            // storage/HNSW now contain (a file edit silently rewrites a
            // section's text while keeping its ID). `PrefetchEngine::invalidate`
            // and `clear_cache` were both documented as "called by the
            // coherence engine when source files change" but neither had a
            // production caller, so warm hits could serve pre-edit text
            // indefinitely. Conservative full-clear avoids threading affected
            // section IDs through `run()` — default capacity is 50 entries
            // so the cost is bounded; re-warming happens on the next reads.
            prefetch.lock().await.clear_cache();

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

            // Resolver auto-heal: re-resolve any files whose stored
            // `resolver_version` is below `ingestion::RESOLVER_VERSION`.
            // Runs unconditionally after every initial index because the
            // ingest path skips unchanged files (via the corpus-merkle
            // short-circuit and per-file mtime check), which means a
            // resolver-only logic bump on previously-indexed corpora
            // produces zero ingest work — and therefore zero stamp
            // refreshes — without this explicit heal. The heal is fast
            // for already-fresh corpora: `list_file_hashes` plus a
            // single filter, no per-file work.
            match pipeline
                .re_resolve_stale_files(&local_paths, &*storage)
                .await
            {
                Ok(0) => {}
                Ok(healed) => info!(corpus_id, healed, "resolver auto-heal completed"),
                Err(e) => warn!(
                    corpus_id,
                    error = %e,
                    "resolver auto-heal failed"
                ),
            }

            // Cloud durability hook: fires the blob-upload reactor on
            // every successful ingest. No-op when no sink is wired.
            registry.notify_complete(corpus_id, &data_dir);
        }
        Err(ministr_core::error::IngestionError::Cancelled) => {
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

        let mut watcher = match ministr_core::coherence::FileWatcher::new(&watch_paths) {
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
    storage: &ministr_core::storage::SqliteStorage,
    source_path: &str,
) -> Vec<String> {
    let Ok(docs) = storage.list_documents().await else {
        return Vec::new();
    };

    // Pass 1: exact string match (fast, covers the common case).
    let mut matched: Vec<&ministr_core::storage::traits::DocumentRecord> = docs
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
