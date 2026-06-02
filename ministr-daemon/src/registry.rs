//! [`CorpusRegistry`] — lifecycle manager for indexed corpora.
//!
//! Owns the shared embedding model (loaded once) and a map of
//! [`CorpusHandle`] instances. Thread-safe via interior `RwLock`.
//!
//! Registered corpora are persisted to a JSON manifest at
//! `{data_dir}/corpora.json` so that they survive daemon restarts.
//!
//! Ingestion is delegated to the [`indexer`](crate::indexer) module (SRP).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use ministr_api::coherence::CoherenceEvent;
use ministr_api::corpora_repo::{CorporaRepo, CorpusRegistration};
use ministr_api::corpus::{CorpusInfo, IndexingStatus};
use ministr_api::corpus_restorer::{CorpusRestoreError, CorpusRestorer};
use ministr_core::config::MinistrConfig;
use ministr_core::corpus_id::{CorpusIdError, canonical_corpus_paths, corpus_id_from_paths};
use ministr_core::embedding::{
    DualEmbedder, Embedder, EmbeddingService, FastReranker, MatryoshkaEmbedder,
};
use ministr_core::index::{HnswIndex, VectorIndex, VectorIndexLoad};
use ministr_core::ingestion::IngestionProgress;
use ministr_core::service::QueryService;
use ministr_core::session::prefetch::PrefetchEngine;
use ministr_core::session::{SessionRegistry, UsageConfig};
use ministr_core::storage::{SqliteStorage, Storage};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::coordinator::IngestionCoordinator;
use crate::embedder_pool::{EmbedderPool, PooledEmbedder};
use crate::indexer;
use crate::scheduler::IngestionScheduler;

// -- Manifest persistence --

/// A single entry in the on-disk corpus manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestEntry {
    id: String,
    paths: Vec<String>,
}

/// Errors from corpus registry operations.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("storage: {0}")]
    Storage(String),
    #[error("index: {0}")]
    Index(String),
    #[error("embedder: {0}")]
    Embedder(String),
    #[error("corpus not found: {id}")]
    NotFound { id: String },
    #[error("identity changed: paths canonicalise to {actual}, expected {expected}")]
    IdentityChanged { expected: String, actual: String },
    #[error("invalid corpus paths: {0}")]
    InvalidPath(#[from] CorpusIdError),
}

/// Central registry managing all indexed corpora.
pub struct CorpusRegistry {
    embedder: Arc<dyn Embedder>,
    /// Resident per-model embedder + embedding-service pool
    /// (parity-seam-registry-routing). Seeded at construction with the default
    /// model's boot-built embedder + its dedicated `EmbeddingService` (the
    /// GPU-owning batch queue, ADR 0001 D1); any per-corpus `[corpus] model` is
    /// built + cached on first use so the daemon honors it for both ingest and
    /// query. Refines ADR 0001 D1 from one shared service to one service per
    /// distinct model actually in use.
    pool: EmbedderPool,
    /// Daemon-wide indexing concurrency policy: bounded parallelism +
    /// per-corpus exclusion. Replaces the old `INDEXING_SEMAPHORE(1)` band-aid.
    scheduler: IngestionScheduler,
    /// The explicit ingestion job queue (cq-queue). Every ingest entry point —
    /// `register`, `update_corpus_paths`, and the file watcher — enqueues here
    /// instead of spawning `indexer::run` directly, so the queue owns dispatch:
    /// bounded, fair (no head-of-line blocking), with Queued→Indexing status
    /// straddling the wait. Drains using the [`scheduler`](Self::scheduler) for
    /// the global bound.
    coordinator: IngestionCoordinator,
    /// `Arc<CorpusHandle>` (not bare `CorpusHandle`) so `get()` can hand
    /// a corpus out by cloning the `Arc` and dropping the map guard —
    /// callers no longer hold a `RwLockReadGuard` across their `.await`s,
    /// which previously serialised every request behind register /
    /// unregister and risked lock-order inversion.
    corpora: RwLock<HashMap<String, Arc<CorpusHandle>>>,
    config: MinistrConfig,
    /// Optional sink for coherence events — wired in by [`AppState::new`]
    /// after construction so `register` can spawn a pusher task that
    /// feeds the app-level ring buffer without the registry needing a
    /// direct handle to [`AppState`].
    coherence_sink: std::sync::OnceLock<Arc<RwLock<VecDeque<CoherenceEvent>>>>,
    /// Optional sink for per-corpus ingestion-completion events. Wired
    /// in cloud mode by `cmd_serve_http` after construction; the
    /// indexer fires this from the `Ok(stats)` exit point of every
    /// successful ingest (initial register, `update_corpus_paths`
    /// re-ingest, and watcher debounced re-runs). The reactor on the
    /// receive end exports a bundle and uploads to durable blob
    /// storage so corpus indexes survive ACA pod recycling. `None` on
    /// self-hosted serve, where the user's local disk is already
    /// durable.
    completion_tx: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<(String, PathBuf)>>,
    /// Durable registry repository. Wired in cloud mode by
    /// `cmd_serve_http` so the list of which corpora exist survives
    /// pod recycling — the on-disk `corpora.json` is pod-ephemeral on
    /// ACA. `None` on self-hosted serve, where the user's local disk
    /// is already durable and `corpora.json` is the source of truth.
    /// When set, `register` / `unregister` / `update_corpus_paths` fire
    /// idempotent writes through the repo, and `restore` reads its
    /// entry list from the repo instead of the manifest file.
    corpora_repo: std::sync::OnceLock<Arc<dyn CorporaRepo>>,
    /// PHASE3 chunk 5 — on-demand bundle restore hook. Wired in
    /// cloud mode by `cmd_serve_http`. When a `get()` misses on the
    /// in-memory map and a `cloud_corpora` row exists for the id,
    /// `ensure_present` calls `restorer.download(...)` then inserts
    /// the corpus via `register_restored`. `None` on self-hosted
    /// serve — a missing in-memory entry stays a `NotFound`.
    corpus_restorer: std::sync::OnceLock<Arc<dyn CorpusRestorer>>,
    /// Per-corpus async mutex serializing restore attempts so two
    /// concurrent queries for the same fresh id don't kick off two
    /// downloads. Cleared after a successful restore; future
    /// restore attempts will re-create as needed.
    restore_locks: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// A single managed corpus with its resources.
pub struct CorpusHandle {
    /// Wrapped in `Arc` so callers can extract a reference, drop the
    /// outer `corpora` map guard, and *then* await on `info.write()` /
    /// `info.read()`. Without the `Arc`, the only way to reach `info`
    /// is through the map guard, which would force the registry-map
    /// lock to be held across every per-corpus `info` await.
    pub info: Arc<RwLock<CorpusInfo>>,
    pub storage: Arc<SqliteStorage>,
    pub index: Arc<dyn VectorIndex>,
    pub service: QueryService,
    /// The effective embedding model this corpus was indexed + is queried with
    /// (parity-seam-registry-routing) — its `.ministr.toml` `[corpus] model`
    /// else the daemon default. `indexer::run` resolves the corpus's embedder
    /// from this so ingest and query share one vector space.
    pub model: String,
    /// The corpus's effective Matryoshka truncation dimension
    /// (parity-registry-knobs) — its `.ministr.toml` `[corpus] dimension`, else
    /// `None` (full model dimension). When `Some`, both `create_handle` (query)
    /// and `indexer::run` (ingest) wrap the pooled embedder in a
    /// [`MatryoshkaEmbedder`] at this dimension via [`apply_dimension`], so the
    /// HNSW index, ingest, and query all share ONE truncated vector space —
    /// exactly as the CLI's `init_infrastructure` does.
    pub dimension: Option<usize>,
    /// The corpus's effective two-stage rerank depth (parity-registry-knobs) —
    /// its `.ministr.toml` `[corpus] rerank_depth`. Only effective when
    /// `dimension` is `Some`; threaded into the `QueryService` via
    /// `with_matryoshka_rerank` (defaulting to 100, matching the CLI).
    pub rerank_depth: Option<usize>,
    /// The corpus's effective parser override (parity-meta-toml-load) — its
    /// per-corpus `meta.toml` `parser`, else `None` (auto-detect by extension).
    /// `indexer::run` applies this to the `IngestionPipeline`.
    pub parser: Option<ministr_core::parser::ParserKind>,
    /// The corpus's effective minimum standalone-section token count
    /// (parity-meta-toml-load) — its `meta.toml` `min_section_tokens`
    /// (default 50). `indexer::run` applies this to the `IngestionPipeline`.
    pub min_section_tokens: usize,
    /// `Arc` so `list()` (and any read-mostly status path) can clone the
    /// handle out and drop the corpora-map guard *before* awaiting the
    /// session lock. Accessors are unchanged — `Arc` derefs to the
    /// `Mutex` for `.lock()`/`.try_lock()`.
    pub sessions: Arc<tokio::sync::Mutex<SessionRegistry>>,
    pub prefetch: Arc<tokio::sync::Mutex<PrefetchEngine>>,
    pub progress: Arc<IngestionProgress>,
    pub cancel: CancellationToken,
    pub data_dir: PathBuf,
    /// Join handles for every background task spawned for this corpus
    /// (cache/session invalidators, coherence sink pusher, indexer +
    /// watcher). `unregister` awaits these *after* `cancel` so the tasks
    /// have actually exited — and released their `SQLite` / watcher file
    /// handles — before the caller deletes `data_dir`. Without this,
    /// `remove_dir_all` races open handles and fails on Windows.
    ///
    /// `Arc<std::sync::Mutex<…>>` (not the corpus `RwLock`): handles are
    /// pushed from `register` after the corpus is in the map, and the
    /// lock is only ever held for a `Vec` push / `mem::take`, never
    /// across an `.await`.
    pub tasks: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// Broadcast channel for coherence notifications — one event per
    /// file-system change the watcher observed, carrying path, kind, and
    /// the list of affected section IDs.
    pub coherence_tx: tokio::sync::broadcast::Sender<CoherenceEvent>,
}

impl CorpusHandle {
    /// Live view of this corpus, layering on-the-fly signals (`progress`
    /// while indexing, `index.len()` always) over the persisted `info`
    /// record.
    ///
    /// `info` is the registry's authoritative status. It changes via
    /// [`CorpusRegistry::set_status`] (start / error / cancel),
    /// [`CorpusRegistry::update_stats`] (post-success), and
    /// [`CorpusRegistry::update_symbols_count`]. The per-file counts
    /// only land at end-of-run, though, so reading `info` during a long
    /// ingest looks frozen — this is where the merge fills the gap.
    ///
    /// Why the `Indexing { .. }` guard: [`IngestionProgress::is_running`]
    /// is a one-way flag flipped by `start()` and cleared by `complete()`.
    /// Some pipeline error paths return early without calling `complete`,
    /// so a registry-level transition to `Error` or `Idle` can race with
    /// a stuck progress flag. Only merging when the persisted snapshot
    /// itself says `Indexing` keeps explicit error/cancel transitions
    /// from being masked by the merge.
    ///
    /// Used by [`CorpusRegistry::list`], which feeds the daemon's HTTP
    /// `GET /api/v1/corpora`, the Tauri `list_corpora` / `daemon_status`
    /// commands, the tray refresh loop, and `ministr status`. The MCP
    /// `ministr://status` resource lives in `ministr-mcp` and builds its
    /// own server-centric shape — it does not currently consume this.
    pub async fn current_info(&self) -> CorpusInfo {
        let info = self.info.read().await.clone();
        merge_live_info(info, &self.progress, self.index.len())
    }
}

/// Merge live signals into a persisted [`CorpusInfo`] snapshot.
///
/// Pulled out of [`CorpusHandle::current_info`] as a sync free function
/// so tests can exercise the merge precedence without constructing a
/// full handle (which would need storage, embedder, vector index, etc.).
fn merge_live_info(
    mut info: CorpusInfo,
    progress: &IngestionProgress,
    index_len: usize,
) -> CorpusInfo {
    // HNSW is the authoritative vector count — grows as embeddings
    // land, while the persisted field only stamps at end-of-run.
    info.embeddings_count = index_len;
    // Only merge live progress when the registry already considers
    // indexing in progress; see the `current_info` doc for why.
    if matches!(info.status, IndexingStatus::Indexing { .. }) && progress.is_running() {
        info.status = IndexingStatus::Indexing {
            files_done: progress.files_done(),
            files_total: progress.files_total(),
        };
        info.sections_count = progress.sections_done();
    }
    info
}

impl CorpusRegistry {
    pub fn new(embedder: Arc<dyn Embedder>, config: MinistrConfig) -> Self {
        let embedding_service = Arc::new(EmbeddingService::with_model(Arc::clone(&embedder)));
        // Seed the per-model pool with the default model's boot-built Arcs so
        // the common (default-model) path reuses them with no rebuild; any
        // per-corpus model is built + cached lazily by `embedder_for`.
        let pool = EmbedderPool::with_data_dir(config.data_dir.clone());
        pool.seed(
            &config.default_model,
            Arc::clone(&embedder),
            Arc::clone(&embedding_service),
        );
        Self {
            pool,
            scheduler: IngestionScheduler::with_default_concurrency(),
            coordinator: IngestionCoordinator::new(),
            embedder,
            corpora: RwLock::new(HashMap::new()),
            config,
            coherence_sink: std::sync::OnceLock::new(),
            completion_tx: std::sync::OnceLock::new(),
            corpora_repo: std::sync::OnceLock::new(),
            corpus_restorer: std::sync::OnceLock::new(),
            restore_locks: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// The ingestion scheduler — bounded concurrency + per-corpus exclusion.
    pub(crate) fn scheduler(&self) -> &IngestionScheduler {
        &self.scheduler
    }

    /// The ingestion coordinator — the explicit job queue (cq-queue). Reached
    /// from a running job so its completion can re-drive dispatch.
    pub(crate) fn coordinator(&self) -> &IngestionCoordinator {
        &self.coordinator
    }

    /// Enqueue a background indexing job for `corpus_id` over `paths`.
    ///
    /// The single entry point for all ingest requests: the coordinator marks
    /// the corpus Queued, dispatches it when a slot is free (Indexing), and
    /// tracks the spawned job in the corpus's `tasks` for `unregister`
    /// teardown. Returns once queued — not when indexing finishes.
    pub(crate) async fn enqueue_index(self: &Arc<Self>, corpus_id: String, paths: Vec<String>) {
        // cq-priority: estimate this job's indexing work from the corpus's
        // last-known indexed file count (cheap, in-memory) so the coordinator
        // dispatches small user code repos ahead of huge vendored trees
        // (shortest-job-first). A never-indexed corpus reads 0 → treated as
        // small → prompt first-time indexing. Clone the `info` Arc and drop the
        // corpora-map guard before awaiting the per-corpus info lock (the
        // registry's lock-ordering rule — never hold the map guard across an
        // `info` await).
        let priority = {
            let info = self
                .corpora
                .read()
                .await
                .get(&corpus_id)
                .map(|h| Arc::clone(&h.info));
            match info {
                Some(info) => info.read().await.files_indexed,
                None => 0,
            }
        };
        self.coordinator
            .enqueue(self, corpus_id, paths, priority)
            .await;
    }

    /// Wire a coherence sink so `register` will spawn a per-corpus
    /// pusher task that copies events into the supplied buffer.
    ///
    /// Intended to be called exactly once, immediately after construction.
    /// A second call is a no-op (the first wins).
    pub fn set_coherence_sink(&self, sink: Arc<RwLock<VecDeque<CoherenceEvent>>>) {
        let _ = self.coherence_sink.set(sink);
    }

    /// Wire an ingestion-completion sink. After this call, every
    /// successful ingest in [`indexer::run`] sends `(corpus_id, corpus_dir)`
    /// on `tx`. The receive end is the cloud durability reactor that
    /// exports a bundle and uploads it to blob storage.
    ///
    /// Intended to be called exactly once, immediately after construction.
    /// A second call is a no-op (the first wins). `None`-sink registries
    /// (the self-hosted serve and every test) see zero behavior change —
    /// `notify_complete` is fire-and-forget and silently drops when no
    /// sink is wired.
    pub fn set_completion_sink(&self, tx: tokio::sync::mpsc::UnboundedSender<(String, PathBuf)>) {
        let _ = self.completion_tx.set(tx);
    }

    /// Send a `(corpus_id, corpus_dir)` completion event if a sink is
    /// wired. Best-effort: if the receiver has been dropped the send
    /// error is ignored, matching the [`UsageSink`] convention.
    ///
    /// Called from [`indexer::run`] at the success exit point of every
    /// ingest path (initial register, `update_corpus_paths` re-ingest,
    /// watcher debounced re-run).
    pub(crate) fn notify_complete(&self, corpus_id: &str, corpus_dir: &std::path::Path) {
        if let Some(tx) = self.completion_tx.get() {
            let _ = tx.send((corpus_id.to_string(), corpus_dir.to_path_buf()));
        }
    }

    /// Wire a durable corpus registry repository. After this call,
    /// every `register` / `unregister` / `update_corpus_paths` mirrors
    /// its in-memory mutation to the repo, and `restore` reads its
    /// entry list from the repo instead of `corpora.json`.
    ///
    /// Intended to be called exactly once, immediately after
    /// construction. A second call is a no-op (the first wins). `None`
    /// is the self-hosted serve default — `corpora.json` stays the
    /// source of truth.
    pub fn set_corpora_repo(&self, repo: Arc<dyn CorporaRepo>) {
        let _ = self.corpora_repo.set(repo);
    }

    /// Best-effort upsert into the durable corpora repo. Mirrors the
    /// `save_manifest` contract: failures warn-log and continue — the
    /// in-memory registration is still usable this session, and a
    /// later mutation may succeed.
    async fn notify_repo_upsert(&self, corpus_id: &str, paths: &[String]) {
        let Some(repo) = self.corpora_repo.get() else {
            return;
        };
        let entry = CorpusRegistration {
            corpus_id: corpus_id.to_string(),
            paths: paths.to_vec(),
            display_name: Some(display_name_from_paths(paths)),
        };
        if let Err(e) = repo.upsert(&entry).await {
            warn!(corpus_id = %corpus_id, error = %e, "corpora_repo upsert failed");
        }
    }

    /// Best-effort remove from the durable corpora repo. Same
    /// failure contract as [`Self::notify_repo_upsert`].
    async fn notify_repo_remove(&self, corpus_id: &str) {
        let Some(repo) = self.corpora_repo.get() else {
            return;
        };
        if let Err(e) = repo.remove(corpus_id).await {
            warn!(corpus_id = %corpus_id, error = %e, "corpora_repo remove failed");
        }
    }

    /// Wire an on-demand bundle restorer (PHASE3 chunk 5, cloud mode).
    /// Intended to be called exactly once, immediately after
    /// construction. A second call is a no-op (the first wins).
    pub fn set_corpus_restorer(&self, restorer: Arc<dyn CorpusRestorer>) {
        let _ = self.corpus_restorer.set(restorer);
    }

    /// Acquire the per-corpus restore mutex, creating it if absent.
    /// Lock-of-locks pattern: brief acquisition of the outer
    /// `restore_locks` mutex to look up / insert; then return the inner
    /// `Arc<Mutex<()>>` so the caller can hold the per-corpus lock for
    /// the actual download.
    async fn restore_lock(&self, corpus_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.restore_locks.lock().await;
        map.entry(corpus_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// PHASE3 chunk 5 — on-demand bundle restore. If `corpus_id` is
    /// already in the in-memory map, returns its handle. Otherwise,
    /// when a [`CorpusRestorer`] is wired AND the corpus has a row in
    /// the durable [`CorporaRepo`], downloads the bundle and inserts
    /// a [`CorpusHandle`] pointing at the just-restored on-disk data
    /// — without spawning `indexer::run` (the bundle is already
    /// indexed). Returns `NotFound` when the restorer isn't wired,
    /// the repo lookup misses, or the bundle isn't in blob yet.
    ///
    /// Concurrent calls for the same `corpus_id` serialise on the
    /// per-corpus mutex so only one download happens.
    ///
    /// # Errors
    ///
    /// - [`RegistryError::NotFound`] — corpus is genuinely absent
    ///   (no repo row, no bundle in blob, or restorer not wired).
    /// - [`RegistryError::Storage`] — restorer or repo backend error.
    pub async fn ensure_present(
        &self,
        corpus_id: &str,
    ) -> Result<Arc<CorpusHandle>, RegistryError> {
        // Fast path: already in memory.
        if let Some(handle) = self.corpora.read().await.get(corpus_id).cloned() {
            return Ok(handle);
        }
        // Without a restorer wired we cannot lazy-load — preserve the
        // existing `NotFound` semantics on self-hosted serve.
        let Some(restorer) = self.corpus_restorer.get().cloned() else {
            return Err(RegistryError::NotFound {
                id: corpus_id.to_string(),
            });
        };
        // Check the durable registry for the canonical paths before
        // touching blob storage — if the repo doesn't list the corpus
        // we shouldn't pay download latency on a 404 anyway.
        let Some(repo) = self.corpora_repo.get().cloned() else {
            return Err(RegistryError::NotFound {
                id: corpus_id.to_string(),
            });
        };
        let rows = repo
            .list()
            .await
            .map_err(|e| RegistryError::Storage(format!("corpora_repo list: {e}")))?;
        let Some(registration) = rows.into_iter().find(|r| r.corpus_id == corpus_id) else {
            return Err(RegistryError::NotFound {
                id: corpus_id.to_string(),
            });
        };

        // Serialise concurrent restores of the same id behind a single
        // per-corpus mutex.
        let lock = self.restore_lock(corpus_id).await;
        let _guard = lock.lock().await;

        // Re-check the in-memory map after acquiring the lock — a
        // concurrent restorer may have already inserted.
        if let Some(handle) = self.corpora.read().await.get(corpus_id).cloned() {
            return Ok(handle);
        }

        let target = self.config.data_dir.join("corpora").join(corpus_id);
        info!(corpus_id, target = %target.display(), "restoring corpus bundle from blob");
        match restorer.download(corpus_id, &target).await {
            Ok(()) => {}
            Err(CorpusRestoreError::NotFound { .. }) => {
                return Err(RegistryError::NotFound {
                    id: corpus_id.to_string(),
                });
            }
            Err(e) => {
                return Err(RegistryError::Storage(format!("blob restore: {e}")));
            }
        }
        self.register_restored(corpus_id, &registration.paths, registration.display_name)
            .await
    }

    /// PHASE3 chunk 5 — insert a corpus into the in-memory map from
    /// already-on-disk data (the bundle import wrote
    /// `<data_dir>/corpora/<id>/{content.db,index}` already). Does NOT
    /// spawn `indexer::run` or a file watcher — the bundle is fully
    /// indexed and the source files don't live on the serve pod.
    ///
    /// # Errors
    ///
    /// - [`RegistryError::Storage`] — failure opening the on-disk
    ///   storage or index.
    pub async fn register_restored(
        &self,
        corpus_id: &str,
        paths: &[String],
        display_name: Option<String>,
    ) -> Result<Arc<CorpusHandle>, RegistryError> {
        // Atomic check-and-insert so a concurrent register on the same
        // id can't both pass and orphan a half-built handle.
        {
            let map = self.corpora.read().await;
            if let Some(handle) = map.get(corpus_id) {
                return Ok(Arc::clone(handle));
            }
        }
        let display = display_name.unwrap_or_else(|| display_name_from_paths(paths));
        let handle = self.create_handle(corpus_id, paths, display).await?;

        // Seed CorpusInfo.files_indexed from the restored content.db so
        // /api/v1/corpora reports the bundle's true count instead of the
        // create_handle placeholder zero. Without this, every lazy-restore
        // would report 0 files until the next `update_stats` call (which
        // only fires after a fresh ingest on this pod — and the serve pod
        // never re-indexes a cloud corpus). sections_count is best-effort:
        // failure to count is logged but doesn't fail the restore.
        match handle.storage.list_file_hashes().await {
            Ok(hashes) => {
                handle.info.write().await.files_indexed = hashes.len();
            }
            Err(e) => warn!(
                corpus_id,
                error = %e,
                "list_file_hashes failed after restore — files_indexed left at 0",
            ),
        }

        let handle = Arc::new(handle);
        {
            let mut map = self.corpora.write().await;
            if let Some(existing) = map.get(corpus_id) {
                // Lost the race — discard our handle, return theirs.
                return Ok(Arc::clone(existing));
            }
            map.insert(corpus_id.to_string(), Arc::clone(&handle));
        }
        info!(corpus_id, "corpus restored from blob");
        Ok(handle)
    }

    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    /// The embedder + embedding service for a corpus's effective model
    /// (parity-seam-registry-routing). The default model returns the seeded
    /// boot-built Arcs; any other model is built and cached on first use.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::Embedder`] if the model can't be built (e.g.
    /// an unknown or uninstalled model name).
    pub(crate) fn embedder_for(&self, model: &str) -> Result<PooledEmbedder, RegistryError> {
        self.pool.get(model).map_err(RegistryError::Embedder)
    }

    pub fn config(&self) -> &MinistrConfig {
        &self.config
    }

    /// Internal access to the corpora map (for the indexer and daemon).
    pub fn corpora(&self) -> &RwLock<HashMap<String, Arc<CorpusHandle>>> {
        &self.corpora
    }

    /// Restore previously registered corpora from durable storage.
    ///
    /// Source of truth:
    /// - When a `CorporaRepo` is wired (cloud mode), reads from the
    ///   repo — the on-disk `corpora.json` is pod-ephemeral on ACA.
    /// - Otherwise (self-hosted serve), reads `{data_dir}/corpora.json`.
    ///
    /// Skips entries whose source paths no longer exist on disk.
    /// Safe to call on an empty registry — idempotent with `register`.
    #[allow(clippy::too_many_lines)] // restore is one cohesive startup pass
    pub async fn restore(self: &Arc<Self>) {
        let repo_mode = self.corpora_repo.get().is_some();
        let entries = if let Some(repo) = self.corpora_repo.get() {
            match repo.list().await {
                Ok(rows) => rows
                    .into_iter()
                    .map(|r| ManifestEntry {
                        id: r.corpus_id,
                        paths: r.paths,
                    })
                    .collect::<Vec<_>>(),
                Err(e) => {
                    warn!(error = %e, "corpora_repo list failed — starting fresh");
                    return;
                }
            }
        } else {
            let manifest_path = self.manifest_path();
            match std::fs::read_to_string(&manifest_path) {
                Ok(json) => match serde_json::from_str::<Vec<ManifestEntry>>(&json) {
                    Ok(entries) => entries,
                    Err(e) => {
                        warn!(error = %e, "corrupt corpus manifest — starting fresh");
                        return;
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => {
                    warn!(error = %e, "failed to read corpus manifest");
                    return;
                }
            }
        };

        info!(
            count = entries.len(),
            source = if repo_mode { "repo" } else { "manifest" },
            "restoring corpora"
        );

        // One-time on-disk migration from un-canonicalised ids: if an
        // entry's stored id doesn't match the canonical id of its paths,
        // rename `corpora/{old_id}` → `corpora/{new_id}` so the new code
        // picks up the existing data instead of orphaning it. On collision
        // (both dirs exist) we leave the old dir in place — better to keep
        // a stale orphan than overwrite live data.
        //
        // Run on the blocking pool so the rename / metadata syscalls don't
        // park the async runtime thread during startup. The migration is
        // sequential because the entry list is small (one entry per
        // registered corpus) and the renames don't benefit from parallelism.
        let corpora_dir = self.config.data_dir.join("corpora");
        let migration_entries: Vec<(String, String)> = entries
            .iter()
            .filter_map(|e| {
                let new_id = corpus_id_from_paths(&e.paths).ok()?;
                (new_id != e.id).then_some((e.id.clone(), new_id))
            })
            .collect();
        if !migration_entries.is_empty() {
            let migration_dir = corpora_dir.clone();
            let _ = tokio::task::spawn_blocking(move || {
                for (old_id, new_id) in migration_entries {
                    let old_dir = migration_dir.join(&old_id);
                    let new_dir = migration_dir.join(&new_id);
                    if !old_dir.exists() {
                        continue;
                    }
                    if new_dir.exists() {
                        warn!(
                            old_id = %old_id,
                            new_id = %new_id,
                            "canonical-id collision on migration — leaving old dir as orphan"
                        );
                        continue;
                    }
                    match std::fs::rename(&old_dir, &new_dir) {
                        Ok(()) => info!(
                            old_id = %old_id,
                            new_id = %new_id,
                            "migrated corpus dir to canonical id"
                        ),
                        Err(e) => warn!(
                            error = %e,
                            old_id = %old_id,
                            "failed to migrate corpus dir"
                        ),
                    }
                }
            })
            .await;
        }

        // Partition into live vs dead entries. An entry is *dead* when
        // every one of its paths is a local path that no longer exists
        // (e.g. a `/tmp/ministr-e2e-test` left by a test run, or a project
        // the user deleted). Remote sources (http/git) can't be stat'd, so
        // any remote path keeps the entry alive.
        let (live, dead): (Vec<&ManifestEntry>, Vec<&ManifestEntry>) =
            entries.iter().partition(|e| entry_is_live(&e.paths));

        // Detect duplicate-by-canonical-id entries among the live set
        // (e.g. left over from before `canonical_corpus_path` normalised
        // `.`/`..`/repeated slashes). `register()` is itself idempotent so
        // the in-memory map collapses them, but the on-disk manifest
        // still carries the dupes — we persist a clean copy below.
        let mut canonical_ids_seen: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut duplicate_canonical = false;
        for entry in &live {
            if let Ok(canon_id) = corpus_id_from_paths(&entry.paths)
                && !canonical_ids_seen.insert(canon_id)
            {
                duplicate_canonical = true;
            }
        }

        // Cloud-serve mode (both a corpora_repo AND a corpus_restorer are
        // wired): the serve pod has no source files mounted, so calling
        // `register()` would invoke `indexer::run` on git-URL paths,
        // discover 0 files, fail the HNSW dump on an empty index, and
        // stamp `files_indexed=0` on the in-memory `CorpusInfo` —
        // permanently masking the bundle the worker uploaded to blob.
        // Use the lazy restorer instead: it downloads the bundle and
        // routes through `register_restored`, which never touches the
        // local indexer. Bundles that don't exist yet (worker hasn't
        // completed the first index) are logged and skipped — they'll
        // restore on demand via `ensure_present` on first query.
        let cloud_mode = self.corpus_restorer.get().is_some() && self.corpora_repo.get().is_some();
        if cloud_mode {
            for entry in &live {
                match self.ensure_present(&entry.id).await {
                    Ok(_) => {}
                    Err(RegistryError::NotFound { .. }) => {
                        info!(
                            corpus_id = %entry.id,
                            "no bundle in blob yet — will lazy-restore on first query",
                        );
                    }
                    Err(e) => {
                        use std::error::Error as _;
                        use std::fmt::Write as _;
                        let mut chain = format!("{e}");
                        let mut src: Option<&dyn std::error::Error> = e.source();
                        while let Some(s) = src {
                            let _ = write!(chain, " — caused by: {s}");
                            src = s.source();
                        }
                        warn!(
                            corpus_id = %entry.id,
                            error = %chain,
                            "failed to restore corpus from blob",
                        );
                    }
                }
            }
        } else {
            // Self-hosted: restore corpora CONCURRENTLY (bounded) so they all
            // surface at once instead of trickling into the GUI one-by-one.
            // Each `register` runs `create_handle`, which opens the corpus's
            // SQLite and loads/rebuilds its HNSW index from disk — seconds per
            // corpus, and previously awaited strictly in series. The bound keeps
            // a fleet of large indexes from all loading at once; the
            // IngestionCoordinator still governs the actual re-index concurrency.
            const RESTORE_CONCURRENCY: usize = 8;
            let sem = Arc::new(tokio::sync::Semaphore::new(RESTORE_CONCURRENCY));
            let mut set = tokio::task::JoinSet::new();
            for entry in &live {
                let this = Arc::clone(self);
                let paths = entry.paths.clone();
                let id = entry.id.clone();
                // The semaphore is owned locally and never closed, so
                // acquire_owned cannot fail; bail out of the loop defensively
                // rather than panic if that invariant ever changes.
                let Ok(permit) = Arc::clone(&sem).acquire_owned().await else {
                    break;
                };
                set.spawn(async move {
                    let _permit = permit;
                    if let Err(e) = this.register(&paths).await {
                        warn!(corpus_id = %id, error = %e, "failed to restore corpus");
                    }
                });
            }
            while set.join_next().await.is_some() {}
        }

        // If two live manifest entries canonicalised to the same id,
        // rewrite the manifest from the deduped in-memory corpora map so
        // the dupe doesn't keep reappearing on every restart.
        if duplicate_canonical && let Err(e) = self.save_manifest().await {
            warn!(
                error = %e,
                "failed to persist deduped corpus manifest after canonicalisation merge"
            );
        }

        // Self-heal: drop dead entries from the manifest and best-effort
        // remove their orphaned corpus dirs, so a stale test/deleted
        // project stops reappearing after every restart / `just reinstall`.
        if !dead.is_empty() {
            for entry in &dead {
                info!(
                    corpus_id = %entry.id,
                    paths = ?entry.paths,
                    "pruning dead corpus entry (source paths gone)"
                );
            }
            // Mirror the prune into the durable repo when wired, so a
            // dead row doesn't keep reappearing on every pod restart in
            // cloud mode.
            for entry in &dead {
                self.notify_repo_remove(&entry.id).await;
            }
            let dead_dirs: Vec<PathBuf> = dead.iter().map(|e| corpora_dir.join(&e.id)).collect();
            tokio::spawn(async move {
                for dir in dead_dirs {
                    if dir.exists()
                        && let Err(e) = ministr_core::fs_util::remove_dir_all_robust(&dir).await
                    {
                        warn!(error = %e, path = %dir.display(), "failed to remove dead corpus dir");
                    }
                }
            });
            // Persisting the live set drops the dead entries even if no
            // live corpus is present to trigger a save via `register`.
            if live.is_empty()
                && let Err(e) = self.save_manifest().await
            {
                warn!(error = %e, "failed to persist pruned corpus manifest");
            }
        }
    }

    /// Register a corpus, initialize its resources, and spawn background indexing.
    ///
    /// Strictly idempotent on canonical identity: re-registering the same
    /// path set (in any equivalent form — case, separators, trailing slash)
    /// returns the existing `corpus_id` with `indexing_started = false` and
    /// touches no other corpus's state. To change the paths of an existing
    /// corpus without dropping its sessions, use
    /// [`Self::update_corpus_paths`].
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if storage or index initialization fails.
    pub async fn register(
        self: &Arc<Self>,
        paths: &[String],
    ) -> Result<(String, bool), RegistryError> {
        let canonical = canonical_corpus_paths(paths)?;
        let corpus_id = corpus_id_from_paths(&canonical)?;

        if self.corpora.read().await.contains_key(&corpus_id) {
            return Ok((corpus_id, false));
        }

        // Display name is derived from the *original* paths so we
        // preserve the user's casing — `canonical` is lowercased on
        // Windows for identity stability, but humans want to see
        // "Ministr", not "ministr".
        let display_name = display_name_from_paths(paths);
        let handle = self
            .create_handle(&corpus_id, &canonical, display_name)
            .await?;

        // Subscribe to coherence broadcasts for answer cache invalidation
        // BEFORE inserting the handle (the tx is on the handle).
        let coherence_rx = handle.coherence_tx.subscribe();
        let cache_storage = Arc::clone(&handle.storage);
        let cache_cid = corpus_id.clone();

        // If an observatory sink is wired, subscribe a second receiver
        // and push each event into the shared ring buffer.
        let sink_rx = handle.coherence_tx.subscribe();
        let sink_opt = self.coherence_sink.get().cloned();

        // Clone the corpus cancellation token before the handle is moved
        // into the map so `spawn_watcher` can stop cleanly on unregister.
        let watcher_cancel = handle.cancel.clone();
        // Clone the task-handle sink before the handle moves into the map
        // so the tasks spawned just below can be registered for awaited
        // teardown in `unregister`.
        let tasks = Arc::clone(&handle.tasks);

        // Snapshot storage + index for a post-insert integrity probe
        // (the loaded-from-disk state, before the background indexer
        // gets a chance to rebuild it).
        let integrity_storage = Arc::clone(&handle.storage);
        let integrity_index = Arc::clone(&handle.index);

        // Atomic check-and-insert. The early `contains_key` above is only
        // a fast path; the authoritative test happens here under the
        // write lock so two concurrent `register`s of the same id can't
        // both pass and have the second overwrite (and orphan) the
        // first's handle. The loser discards its freshly-created handle —
        // no background tasks have been spawned yet, so its `Drop` just
        // closes the (idempotently-opened) SQLite/index and returns the
        // idempotent `(id, false)`.
        {
            let mut map = self.corpora.write().await;
            if map.contains_key(&corpus_id) {
                return Ok((corpus_id, false));
            }
            map.insert(corpus_id.clone(), Arc::new(handle));
        }
        info!(corpus_id = %corpus_id, "corpus registered");

        check_index_integrity(&corpus_id, &integrity_storage, integrity_index.len()).await;

        // Manifest persistence failure is non-fatal for the in-memory
        // registration (the corpus is usable this session) but must not
        // be silent — surface it so a restart-loses-corpus is diagnosable.
        if let Err(e) = self.save_manifest().await {
            warn!(corpus_id = %corpus_id, error = %e, "failed to persist corpus manifest after register");
        }
        // Mirror to the durable repo when wired (cloud mode). Same
        // failure contract as save_manifest above.
        self.notify_repo_upsert(&corpus_id, &canonical).await;

        let mut spawned: Vec<tokio::task::JoinHandle<()>> = Vec::with_capacity(4);

        // Spawn answer cache invalidation on coherence events.
        spawned.push(tokio::spawn(async move {
            crate::ask::spawn_cache_invalidator(cache_storage, coherence_rx, cache_cid).await;
        }));

        // Spawn session invalidation on coherence events. Without this,
        // delivered items in every session appear fresh across file edits
        // and no `CoherenceAlert` is enqueued for the MCP client — the
        // documented `SessionRegistry::invalidate_all` path had no
        // production caller.
        spawned.push(spawn_session_invalidator(
            Arc::clone(self),
            corpus_id.clone(),
        ));

        if let Some(sink) = sink_opt {
            spawned.push(tokio::spawn(spawn_coherence_sink_pusher(sink, sink_rx)));
        }

        // Enqueue the initial index onto the coordinator's queue (cq-queue),
        // then start the file watcher (which enqueues reindexes on change). The
        // coordinator tracks the spawned indexing job in this corpus's `tasks`
        // for teardown; this outer task only does the enqueue + watcher launch.
        let registry = Arc::clone(self);
        let cid = corpus_id.clone();
        let owned_paths = canonical.clone();
        spawned.push(tokio::spawn(async move {
            registry
                .enqueue_index(cid.clone(), owned_paths.clone())
                .await;
            indexer::spawn_watcher(registry, cid, owned_paths, watcher_cancel);
        }));

        if let Ok(mut guard) = tasks.lock() {
            guard.extend(spawned);
        }

        Ok((corpus_id, true))
    }

    /// Update the registered paths for an existing corpus without dropping
    /// its [`CorpusHandle`] or in-memory session state.
    ///
    /// Use this when `.ministr.toml` paths change. Active MCP sessions, the
    /// running indexer, the file watcher, and the SQLite/HNSW state all
    /// survive — only `info.paths` is mutated, the manifest is re-saved,
    /// and ingestion is queued for any genuinely new paths.
    ///
    /// `new_paths` must canonicalise to the same `corpus_id` as the existing
    /// corpus. Identity is derived from canonical paths, so a different
    /// canonical id means the caller is changing identity, not just the
    /// path expression — they should `unregister` + `register` instead.
    ///
    /// # Errors
    ///
    /// - [`RegistryError::NotFound`] if `corpus_id` is not registered.
    /// - [`RegistryError::IdentityChanged`] if `new_paths` canonicalise to a
    ///   different id.
    pub async fn update_corpus_paths(
        self: &Arc<Self>,
        corpus_id: &str,
        new_paths: &[String],
    ) -> Result<(), RegistryError> {
        let canonical = canonical_corpus_paths(new_paths)?;
        let new_id = corpus_id_from_paths(&canonical)?;
        if new_id != corpus_id {
            return Err(RegistryError::IdentityChanged {
                expected: corpus_id.to_string(),
                actual: new_id,
            });
        }

        let new_display_name = display_name_from_paths(new_paths);

        // Extract a clone of the info Arc under the corpora-map read
        // guard, then drop the guard before awaiting the per-handle
        // info write. Holding `corpora.read()` across `info.write().await`
        // would block concurrent register/unregister writers for the
        // duration of the info write.
        let info_lock = {
            let corpora = self.corpora.read().await;
            let handle = corpora
                .get(corpus_id)
                .ok_or_else(|| RegistryError::NotFound {
                    id: corpus_id.to_string(),
                })?;
            Arc::clone(&handle.info)
        };

        let added: Vec<String> = {
            let mut info = info_lock.write().await;
            let prior: std::collections::HashSet<String> = info.paths.iter().cloned().collect();
            let added: Vec<String> = canonical
                .iter()
                .filter(|p| !prior.contains(p.as_str()))
                .cloned()
                .collect();
            info.paths = canonical;
            info.display_name = new_display_name;
            added
        };

        self.save_manifest().await?;
        // Mirror the new path set to the durable repo. Identity didn't
        // change (checked above), so this is an in-place row update.
        {
            let info = info_lock.read().await;
            self.notify_repo_upsert(corpus_id, &info.paths).await;
        }

        if !added.is_empty() {
            // Enqueue the re-ingest of the genuinely-new paths onto the
            // coordinator's queue (cq-queue); it tracks the spawned job in the
            // corpus's `tasks` so a later `unregister` awaits it before deleting
            // the corpus dir.
            self.enqueue_index(corpus_id.to_string(), added).await;
        }

        Ok(())
    }

    /// Unregister a corpus, cancelling background work and awaiting its
    /// teardown so the caller can safely delete the corpus directory.
    ///
    /// After signalling `cancel`, this awaits every spawned task (with a
    /// bounded timeout) so their `SQLite` connections and the directory
    /// watcher are actually closed before returning. Skipping this is the
    /// root cause of `remove_dir_all` failing on Windows with a sharing
    /// violation right after unregister.
    ///
    /// # Errors
    ///
    /// - [`RegistryError::NotFound`] if the corpus does not exist.
    /// - [`RegistryError::Storage`] if the manifest could not be persisted.
    pub async fn unregister(&self, corpus_id: &str) -> Result<(), RegistryError> {
        // Extract from the map first, releasing the write lock before
        // save_manifest (which needs a read lock — same RwLock, not reentrant).
        let removed = self.corpora.write().await.remove(corpus_id);
        let Some(handle) = removed else {
            return Err(RegistryError::NotFound {
                id: corpus_id.to_string(),
            });
        };

        // Signal all background tasks to stop, then await their actual
        // exit so file handles are released. Dropping the broadcast
        // sender (held by `handle`) also unblocks the receiver-driven
        // tasks; the cancellation token covers the indexer/watcher.
        handle.cancel.cancel();
        let pending: Vec<tokio::task::JoinHandle<()>> = handle
            .tasks
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();

        if !pending.is_empty() {
            // Bounded: a task wedged on a slow filesystem must not hang
            // unregister forever. The token + dropped sender make a
            // clean exit the overwhelmingly common case.
            const TEARDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
            let join_all = async {
                for h in pending {
                    let _ = h.await;
                }
            };
            if tokio::time::timeout(TEARDOWN_TIMEOUT, join_all)
                .await
                .is_err()
            {
                warn!(
                    corpus_id,
                    "background tasks did not exit within teardown timeout; \
                     directory deletion may transiently fail on Windows"
                );
            }
        }

        info!(corpus_id, "corpus unregistered");
        self.save_manifest().await?;
        self.notify_repo_remove(corpus_id).await;
        Ok(())
    }

    /// List all registered corpora with current status.
    ///
    /// Snapshots each corpus's `Arc` fields under the map read guard,
    /// then **drops the guard** before awaiting any per-corpus lock —
    /// so a concurrent `register`/`unregister` (map write lock) is never
    /// serialised behind N per-corpus `info`/`sessions` awaits.
    pub async fn list(&self) -> Vec<CorpusInfo> {
        type Snap = (
            Arc<RwLock<CorpusInfo>>,
            Arc<IngestionProgress>,
            Arc<dyn VectorIndex>,
            Arc<tokio::sync::Mutex<SessionRegistry>>,
        );
        let snap: Vec<Snap> = {
            let corpora = self.corpora.read().await;
            corpora
                .values()
                .map(|h| {
                    (
                        Arc::clone(&h.info),
                        Arc::clone(&h.progress),
                        Arc::clone(&h.index),
                        Arc::clone(&h.sessions),
                    )
                })
                .collect()
        };

        let mut result = Vec::with_capacity(snap.len());
        for (info, progress, index, sessions) in snap {
            let mut ci = merge_live_info(info.read().await.clone(), &progress, index.len());
            ci.active_sessions = sessions.lock().await.session_count();
            result.push(ci);
        }
        result
    }

    /// Resolve a corpus ID to its handle.
    ///
    /// Clones the `Arc<CorpusHandle>` out and drops the map read guard
    /// before returning — the caller holds only the handle, never a
    /// `RwLockReadGuard`, so its subsequent `.await`s can't serialise
    /// register / unregister or invert lock order.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::NotFound`] if the corpus does not exist.
    pub async fn get(&self, corpus_id: &str) -> Result<Arc<CorpusHandle>, RegistryError> {
        // Fast path: in-memory hit.
        if let Some(handle) = self.corpora.read().await.get(corpus_id).cloned() {
            return Ok(handle);
        }
        // PHASE3 chunk 5 — in cloud mode a `CorpusRestorer` is wired
        // so a miss can be served by downloading the bundle from blob
        // and inserting via `register_restored`. Without a restorer
        // (self-hosted serve) `ensure_present` immediately returns
        // `NotFound`, matching the historical contract.
        self.ensure_present(corpus_id).await
    }

    /// Extract a corpus's `info` handle without holding the corpora-map
    /// guard across the subsequent `.await`.
    ///
    /// The indexer calls `set_status`/`update_stats`/`update_symbols_count`
    /// repeatedly *while* a `register`/`unregister`/`restore` may be
    /// taking the map write lock. Holding `corpora.read()` across
    /// `info.write().await` serialises those writers behind every
    /// per-corpus info write (and risks lock-order inversion). `info` is
    /// an `Arc<RwLock<…>>` precisely so we can clone it out, drop the map
    /// guard, *then* await.
    async fn info_handle(&self, corpus_id: &str) -> Option<Arc<RwLock<CorpusInfo>>> {
        let guard = self.corpora.read().await;
        guard.get(corpus_id).map(|h| Arc::clone(&h.info))
    }

    /// Update indexing status for a corpus.
    pub async fn set_status(&self, corpus_id: &str, status: IndexingStatus) {
        if let Some(info) = self.info_handle(corpus_id).await {
            info.write().await.status = status;
        }
    }

    /// Update corpus statistics after indexing completes.
    pub async fn update_stats(
        &self,
        corpus_id: &str,
        files_indexed: usize,
        sections_count: usize,
        embeddings_count: usize,
    ) {
        if let Some(info) = self.info_handle(corpus_id).await {
            let mut info = info.write().await;
            info.status = IndexingStatus::Idle;
            info.files_indexed = files_indexed;
            info.sections_count = sections_count;
            info.embeddings_count = embeddings_count;
            #[allow(clippy::cast_possible_wrap)]
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            info.last_indexed = Some(ts);
            // Surface the persisted snapshot so a "files went back to 0
            // after re-index" report has a log line to grep for.
            info!(
                corpus_id,
                files_indexed,
                sections_count,
                embeddings_count,
                "corpus stats updated post-indexing"
            );
        } else {
            warn!(
                corpus_id,
                "update_stats: corpus not found in registry — \
                 stats discarded (likely concurrent unregister)"
            );
        }
    }

    /// Update the symbols count for a corpus (called after symbol extraction).
    pub async fn update_symbols_count(&self, corpus_id: &str, symbols_count: usize) {
        if let Some(info) = self.info_handle(corpus_id).await {
            info.write().await.symbols_count = symbols_count;
        }
    }

    // -- Private --

    fn manifest_path(&self) -> PathBuf {
        self.config.data_dir.join("corpora.json")
    }

    /// Persist the current corpus registrations to disk.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::Storage`] if the manifest could not be
    /// serialized or written — callers decide whether that is fatal
    /// (`unregister`/`update_corpus_paths` propagate; `register` logs and
    /// continues since the in-memory corpus is still usable).
    async fn save_manifest(&self) -> Result<(), RegistryError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        // Per-process counter for unique manifest tmp-file names.
        static SEQ: AtomicU64 = AtomicU64::new(0);

        let entries: Vec<ManifestEntry> = {
            let corpora = self.corpora.read().await;
            let mut entries = Vec::with_capacity(corpora.len());
            for (id, handle) in corpora.iter() {
                let info = handle.info.read().await;
                entries.push(ManifestEntry {
                    id: id.clone(),
                    paths: info.paths.clone(),
                });
            }
            entries
        };

        let path = self.manifest_path();
        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| RegistryError::Storage(format!("serialize manifest: {e}")))?;
        // Atomic write (unique tmp + rename) so concurrent saves — e.g. the
        // parallel corpus restore on startup — can never tear the manifest.
        let tmp = path.with_extension(format!(
            "tmp.{}.{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&tmp, json).map_err(|e| {
            RegistryError::Storage(format!("write manifest tmp {}: {e}", tmp.display()))
        })?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            RegistryError::Storage(format!("rename manifest {}: {e}", path.display()))
        })?;
        Ok(())
    }

    async fn create_handle(
        &self,
        corpus_id: &str,
        paths: &[String],
        display_name: String,
    ) -> Result<CorpusHandle, RegistryError> {
        let corpus_dir = self.config.data_dir.join("corpora").join(corpus_id);
        let db_path = corpus_dir.join("content.db");
        let index_dir = corpus_dir.join("index");

        std::fs::create_dir_all(&corpus_dir)
            .map_err(|e| RegistryError::Storage(format!("create dir: {e}")))?;

        let storage = Arc::new(
            SqliteStorage::open(&db_path)
                .map_err(|e| RegistryError::Storage(format!("open db: {e}")))?,
        );

        // parity-seam-registry-routing + parity-registry-knobs: resolve THIS
        // corpus's FULL effective config (its `.ministr.toml` `[corpus]` model
        // / dimension / rerank_depth, else the daemon defaults) via the shared
        // seam, and bind the corpus's index + QueryService to it. The default
        // model returns the daemon's seeded shared embedder (zero change); a
        // per-corpus model is built + cached once. A configured `dimension`
        // additionally wraps that embedder in a `MatryoshkaEmbedder` so the
        // HNSW index is built at the TRUNCATED dim and the QueryService gets
        // two-stage `rerank_depth` reranking — exactly as the CLI's
        // `init_infrastructure` does. `indexer::run` reads `handle.model` +
        // `handle.dimension` and applies the SAME `apply_dimension`, so ingest
        // and query share one (possibly truncated) vector space.
        let cfg = resolve_corpus_config(paths, &corpus_dir, &self.config);
        let model = cfg.model.clone();
        let pooled = self.embedder_for(&model)?;
        let (embedder, dual) = apply_dimension(&pooled.embedder, cfg.dimension)?;

        let dim = embedder.dimension();
        let index = load_or_create_index(&index_dir, dim, &model, &storage).await?;

        let query_storage = SqliteStorage::open(&db_path)
            .map_err(|e| RegistryError::Storage(format!("open query db: {e}")))?;
        let mut service =
            QueryService::new(query_storage, Arc::clone(&embedder), Arc::clone(&index));

        // parity-registry-knobs: two-stage Matryoshka reranking when a
        // per-corpus `dimension` is configured (mirrors the CLI's `build_server`
        // wiring). `rerank_depth` defaults to 100 — the same default the CLI's
        // `InfrastructureContext` applies.
        if let Some(dual) = dual {
            service = service.with_matryoshka_rerank(dual, cfg.rerank_depth.unwrap_or(100));
            info!(
                corpus_id,
                dimension = cfg.dimension,
                rerank_depth = cfg.rerank_depth.unwrap_or(100),
                "per-corpus Matryoshka truncation + rerank applied"
            );
        }

        // rq5b: attach a cross-encoder reranker to the production query path
        // when one is configured (default OFF — `reranker_model = None`). A
        // reranker is an optional relevance enhancement, so a bad model name
        // or a failed load must NOT break the corpus: warn and serve the
        // dense/hybrid path unchanged. Every corpus's `QueryService` (the one
        // the daemon REST API, the Tauri GUI, and the MCP server all answer
        // through) gets the same reranker, so the flag can't be half-applied.
        if let Some(model) = self.config.reranker_model.as_deref() {
            match FastReranker::new(model, self.config.data_dir.to_str()) {
                Ok(reranker) => {
                    service = service.with_reranker(Arc::new(reranker));
                    info!(
                        corpus_id,
                        reranker_model = model,
                        "cross-encoder reranker attached"
                    );
                }
                Err(e) => warn!(
                    corpus_id,
                    reranker_model = model,
                    error = %e,
                    "failed to build reranker — serving without it",
                ),
            }
        }

        // cq-priority cold-start size signal: seed files_indexed + sections_count
        // from the on-disk content.db so a corpus re-registered after a daemon
        // restart (or any re-register of a previously-indexed corpus) starts with
        // its TRUE size instead of a placeholder 0. This lets the
        // IngestionCoordinator's shortest-job-first dispatch order a cold-start
        // restore-storm by real size (not pure FIFO), and makes the
        // /api/v1/corpora readout correct before the first reindex. These are the
        // same counts `indexer::run`'s `update_stats` writes, so the seed matches
        // the post-reindex value (no visible jump). A brand-new corpus has an
        // empty db → 0 → treated as small (prompt first index).
        let files_indexed = storage.document_count().await.unwrap_or(0);
        let sections_count = storage.section_count().await.unwrap_or(0);

        Ok(CorpusHandle {
            info: Arc::new(RwLock::new(CorpusInfo {
                id: corpus_id.to_string(),
                display_name,
                paths: paths.to_vec(),
                status: IndexingStatus::Idle,
                files_indexed,
                sections_count,
                embeddings_count: index.len(),
                active_sessions: 0,
                last_indexed: None,
                symbols_count: 0,
                model: model.clone(),
            })),
            storage,
            index,
            service,
            model,
            dimension: cfg.dimension,
            rerank_depth: cfg.rerank_depth,
            parser: cfg.parser,
            min_section_tokens: cfg.min_section_tokens,
            sessions: Arc::new(tokio::sync::Mutex::new(SessionRegistry::new(
                UsageConfig::default(),
            ))),
            prefetch: Arc::new(tokio::sync::Mutex::new(
                PrefetchEngine::with_default_capacity(),
            )),
            progress: Arc::new(IngestionProgress::new()),
            cancel: CancellationToken::new(),
            data_dir: corpus_dir,
            tasks: Arc::new(std::sync::Mutex::new(Vec::new())),
            coherence_tx: tokio::sync::broadcast::channel(16).0,
        })
    }
}

/// Resolve a corpus's FULL effective per-corpus config from its first local
/// path's `.ministr.toml` (`[corpus]` `model` / `dimension` / `rerank_depth` /
/// …), falling back to the daemon defaults.
///
/// Best-effort by design (parity-seam-registry-routing): a missing or
/// unparseable `.ministr.toml`, or a non-local path (e.g. a restored cloud
/// corpus whose source files aren't on this pod), yields the default config —
/// so registration never fails just because a per-corpus override can't be
/// discovered. Routes through the same `resolve_effective_corpus_config` seam
/// the CLI uses, so the surfaces cannot drift.
fn resolve_corpus_config(
    paths: &[String],
    corpus_dir: &std::path::Path,
    config: &MinistrConfig,
) -> ministr_core::config::EffectiveCorpusConfig {
    let repo_cfg = paths.first().and_then(|p| {
        let path = std::path::Path::new(p);
        let dir = if path.is_dir() {
            Some(path)
        } else {
            path.parent()
        };
        dir.and_then(|d| ministr_core::config::RepoConfig::discover(d).ok().flatten())
    });
    // parity-meta-toml-load: load this corpus's per-corpus `meta.toml`
    // (`CorpusConfig`) from its data dir so parser + min_section_tokens are
    // honored — the SAME `corpus_config` arg the CLI passes to the seam.
    // Absent/unparseable meta.toml → `None` (defaults), so registration never
    // fails over a missing override.
    let meta = ministr_core::config::CorpusConfig::load(&corpus_dir.join("meta.toml")).ok();
    ministr_core::config::resolve_effective_corpus_config(
        repo_cfg.as_ref().map(|(_, rc)| rc),
        meta.as_ref(),
        config,
    )
}

/// The corpus embedder a configured `dimension` resolves to: the embedder used
/// for the HNSW index + ingest (truncated when Matryoshka is active), plus the
/// optional [`DualEmbedder`] handle the `QueryService` reranks at full
/// dimension with.
type DimensionedEmbedder = (Arc<dyn Embedder>, Option<Arc<dyn DualEmbedder>>);

/// Apply a corpus's effective Matryoshka truncation `dimension` to its pooled
/// embedder (parity-registry-knobs).
///
/// `None` → the pooled embedder unchanged (full model dimension, zero
/// behavioural change — the default-model path). `Some(target)` → wrap it in a
/// [`MatryoshkaEmbedder`] that returns `target`-dimension vectors for HNSW
/// indexing + ingest, plus the [`DualEmbedder`] handle the `QueryService` uses
/// for full-dimension two-stage reranking. This is the SAME truncation the
/// CLI's `init_infrastructure` builds, and it is the single seam BOTH the query
/// path (`create_handle`) and the ingest path (`indexer::run`) call so the two
/// can't drift into different vector spaces.
///
/// # Errors
///
/// Returns [`RegistryError::Embedder`] when `target` exceeds the model's native
/// dimension (an invalid Matryoshka truncation).
pub(crate) fn apply_dimension(
    pooled: &Arc<dyn Embedder>,
    dimension: Option<usize>,
) -> Result<DimensionedEmbedder, RegistryError> {
    match dimension {
        Some(target) => {
            let matryoshka = Arc::new(
                MatryoshkaEmbedder::new(Arc::clone(pooled), target).map_err(|e| {
                    RegistryError::Embedder(format!("matryoshka dimension {target}: {e}"))
                })?,
            );
            let embedder: Arc<dyn Embedder> = Arc::clone(&matryoshka) as _;
            let dual: Arc<dyn DualEmbedder> = matryoshka;
            Ok((embedder, Some(dual)))
        }
        None => Ok((Arc::clone(pooled), None)),
    }
}

/// Bridge a per-corpus coherence broadcast into the app-level ring buffer.
///
/// Runs until the broadcast sender is dropped (corpus unregistered).
/// Lag warnings are silent — the feed is informational, so a dropped
/// event just shortens the displayed history.
async fn spawn_coherence_sink_pusher(
    sink: Arc<RwLock<VecDeque<CoherenceEvent>>>,
    mut rx: tokio::sync::broadcast::Receiver<CoherenceEvent>,
) {
    const BUFFER_CAPACITY: usize = 500;
    loop {
        match rx.recv().await {
            Ok(event) => {
                let mut buf = sink.write().await;
                while buf.len() >= BUFFER_CAPACITY {
                    buf.pop_front();
                }
                buf.push_back(event);
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
        }
    }
}

/// Spawn a task that propagates coherence invalidations into the
/// corpus's session registry.
///
/// Subscribes to `handle.coherence_tx` for the given corpus and calls
/// `SessionRegistry::invalidate_all` on every event carrying a non-empty
/// `affected_sections`. Each affected session marks those content IDs
/// stale and enqueues a `CoherenceAlert` for its consumer.
///
/// Runs until the broadcast sender is dropped (corpus unregistered) or
/// the corpus is removed from the registry. Lag drops are ignored — a
/// missed alert is a bounded cost and the feed stays live.
/// Whether a manifest entry should be restored.
///
/// `true` unless **every** path is a local path that no longer exists.
/// Remote sources (`http`/`git`) can't be stat'd, so any remote path
/// keeps the entry alive. An empty path set is dead.
fn entry_is_live(paths: &[String]) -> bool {
    use ministr_core::config::{CorpusSource, classify_corpus_path};
    !paths.is_empty()
        && paths.iter().any(|p| match classify_corpus_path(p) {
            CorpusSource::Local(pb) => pb.exists(),
            CorpusSource::Web(_) | CorpusSource::Git(_) => true,
        })
}

#[must_use]
pub fn spawn_session_invalidator(
    registry: Arc<CorpusRegistry>,
    corpus_id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = {
            let corpora = registry.corpora.read().await;
            match corpora.get(&corpus_id) {
                Some(handle) => handle.coherence_tx.subscribe(),
                None => return,
            }
        };
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if event.affected_sections.is_empty() {
                        continue;
                    }
                    let corpora = registry.corpora.read().await;
                    let Some(handle) = corpora.get(&corpus_id) else {
                        break;
                    };
                    let mut sessions = handle.sessions.lock().await;
                    let n = sessions.invalidate_all(&event.affected_sections);
                    if n > 0 {
                        tracing::debug!(
                            corpus_id = %corpus_id,
                            invalidated = n,
                            "propagated coherence invalidation to sessions"
                        );
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            }
        }
    })
}

async fn load_or_create_index(
    index_dir: &std::path::Path,
    dim: usize,
    model_name: &str,
    storage: &SqliteStorage,
) -> Result<Arc<dyn VectorIndex>, RegistryError> {
    // ADR 0001 D4: prefer rebuilding the in-memory ANN index from the ACID
    // vector source of truth (the `indexed_vectors` table). There is no
    // separate on-disk graph to drift, and the degenerate-vector guard
    // re-runs on every insert during the rebuild — so the zero-vector-poison
    // and "fixed in code / stale on disk" bug classes are structurally
    // impossible. Falls back to the legacy on-disk HNSW dump for corpora and
    // cloud bundles indexed before the V24 flip (their `indexed_vectors` is
    // empty), and finally to a fresh index.
    match ministr_core::index::rebuild_hnsw_from_store(storage, dim, Some(model_name)).await {
        Ok(rebuilt) if !rebuilt.is_empty() => {
            info!(
                vectors = rebuilt.len(),
                "rebuilt vector index from the SQLite source of truth"
            );
            return Ok(Arc::new(rebuilt));
        }
        Ok(_) => {
            // No persisted indexed vectors — a legacy corpus. Fall through to
            // the on-disk dump so pre-V24 corpora keep loading unchanged.
        }
        Err(e) => {
            warn!(error = %e, "rebuild from SQLite source of truth failed; falling back to on-disk index");
        }
    }

    if index_dir.exists() {
        match HnswIndex::load(index_dir) {
            Ok(loaded) => match loaded.check_compatible(dim, model_name, index_dir) {
                Ok(()) => {
                    // Adopt a legacy index that predates model tracking
                    // so a later model change can actually be detected.
                    if loaded.model_name().is_none() {
                        loaded.set_model_name(model_name);
                    }
                    return Ok(Arc::new(loaded));
                }
                Err(e) => {
                    warn!(error = %e, "embedding model changed — rebuilding index");
                    discard_index_dir(index_dir);
                }
            },
            Err(e) => {
                warn!(error = %e, "corrupted index — rebuilding");
                discard_index_dir(index_dir);
            }
        }
    }
    let fresh = HnswIndex::new(dim, 100_000).map_err(|e| RegistryError::Index(e.to_string()))?;
    fresh.set_model_name(model_name);
    Ok(Arc::new(fresh))
}

/// Lightweight desync probe: warn (with an actionable repair path) when
/// the persisted `SQLite` content and the on-disk vector index are
/// grossly out of sync at registration time — i.e. one side is empty
/// while the other is not. This catches an index whose dump was lost or
/// failed to load (search silently returns nothing despite indexed
/// content) and an orphaned index left without backing content. It
/// never deletes anything: the background indexer reconciles a real
/// content drift; a stale-merkle short-circuit is the case that would
/// otherwise leave this broken until the user forces a re-index.
async fn check_index_integrity(corpus_id: &str, storage: &SqliteStorage, vector_count: usize) {
    let sections = match storage.section_count().await {
        Ok(n) => n,
        Err(e) => {
            // Can't probe — don't block registration over a stats query.
            tracing::debug!(corpus_id, error = %e, "integrity probe: section_count failed");
            return;
        }
    };

    let desynced = (sections > 0 && vector_count == 0) || (sections == 0 && vector_count > 0);
    if desynced {
        warn!(
            corpus_id,
            sections,
            vectors = vector_count,
            "index/content desync detected — semantic search will be \
             degraded for this corpus. Re-index to repair: \
             `ministr reindex` (CLI) or the Re-index button in the app."
        );
    }
}

/// Remove a corrupt/incompatible index dir with the Windows-robust
/// retrying remove, logging (not swallowing) a failure so a stale dir
/// that would be re-loaded next start is visible.
fn discard_index_dir(index_dir: &std::path::Path) {
    if let Err(e) = ministr_core::fs_util::remove_dir_all_robust_sync(index_dir) {
        warn!(
            error = %e,
            dir = %index_dir.display(),
            "failed to remove stale index directory; a rebuild will overwrite it"
        );
    }
}

/// Derive a human-readable label for a corpus from its path set.
///
/// Returns the basename of the longest common ancestor of all paths —
/// i.e. for a `.ministr.toml` whose `[corpus] paths` resolve to
/// `["…/foo/src", "…/foo/docs", "…/foo/README.md"]` this returns
/// `"foo"` regardless of how the paths get sorted later. For a
/// single-path corpus, returns the basename of that path.
///
/// Operates on the original (un-canonicalised) paths to preserve the
/// case the user typed — the canonical form is for identity, not
/// display.
#[must_use]
pub fn display_name_from_paths(paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }

    // Normalise separators only, preserving case.
    let normalised: Vec<String> = paths.iter().map(|p| p.replace('\\', "/")).collect();

    let root: String = if normalised.len() == 1 {
        normalised[0].trim_end_matches('/').to_string()
    } else {
        let segments: Vec<Vec<&str>> = normalised.iter().map(|p| p.split('/').collect()).collect();
        let first = &segments[0];
        let mut common = 0usize;
        for (i, s0) in first.iter().enumerate() {
            if segments[1..].iter().all(|seg| seg.get(i) == Some(s0)) {
                common = i + 1;
            } else {
                break;
            }
        }
        first[..common].join("/")
    };

    let basename = std::path::Path::new(&root)
        .file_name()
        .and_then(|s| s.to_str())
        .map(std::string::ToString::to_string);
    basename.unwrap_or(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Corpus-identity canonicalisation is now owned and unit-tested by
    // `ministr_core::corpus_id` (the single source of truth shared with
    // the CLI). The tests below cover daemon-local behaviour only.

    fn snapshot_with(status: IndexingStatus) -> CorpusInfo {
        CorpusInfo {
            id: "test".into(),
            display_name: "test".into(),
            paths: vec![],
            status,
            files_indexed: 1,
            sections_count: 7,
            embeddings_count: 0,
            active_sessions: 0,
            last_indexed: None,
            symbols_count: 0,
            model: "all-MiniLM-L6-v2".into(),
        }
    }

    /// Minimal full-dimension embedder for the `apply_dimension` parity tests
    /// (no model load). Returns a fixed-dimension vector per text.
    #[derive(Debug)]
    struct FixedDimEmbedder {
        dim: usize,
    }

    impl Embedder for FixedDimEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ministr_core::error::IndexError> {
            Ok(texts.iter().map(|_| vec![0.1f32; self.dim]).collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    // parity-registry-knobs: `apply_dimension` is the single seam BOTH the
    // registry query path (`create_handle`) and the ingest path (`indexer::run`)
    // call, so it MUST truncate the corpus's vector space when a per-corpus
    // `dimension` is configured — and leave the full-dim default untouched
    // otherwise. These prove the behavior the parity gate documents.

    #[test]
    fn apply_dimension_none_is_identity_full_dim() {
        let pooled: Arc<dyn Embedder> = Arc::new(FixedDimEmbedder { dim: 384 });
        let (embedder, dual) = apply_dimension(&pooled, None).expect("identity must not fail");
        assert_eq!(embedder.dimension(), 384, "no dimension → full model dim");
        assert!(
            dual.is_none(),
            "no dimension → no Matryoshka dual for rerank"
        );
    }

    #[test]
    fn apply_dimension_some_truncates_and_yields_dual() {
        let pooled: Arc<dyn Embedder> = Arc::new(FixedDimEmbedder { dim: 384 });
        let (embedder, dual) =
            apply_dimension(&pooled, Some(256)).expect("valid truncation must succeed");
        assert_eq!(
            embedder.dimension(),
            256,
            "the index + ingest must run at the truncated dim"
        );
        assert!(
            dual.is_some(),
            "a configured dimension must yield the dual embedder the QueryService reranks with"
        );
    }

    #[test]
    fn apply_dimension_rejects_target_above_native_dim() {
        let pooled: Arc<dyn Embedder> = Arc::new(FixedDimEmbedder { dim: 384 });
        // A truncation larger than the model's native dim is invalid — it must
        // surface as a clear error, not silently fall back (the parity epic's
        // "no surface silently ignores a configured knob" rule).
        assert!(apply_dimension(&pooled, Some(512)).is_err());
    }

    #[test]
    fn display_name_multi_path_uses_lca_basename() {
        // Mirrors a typical .ministr.toml resolved-paths set. After
        // canonicalisation paths get sorted lexicographically, so a
        // naive "first path's basename" gives "docs" or "README.md"
        // — we want the project root's basename instead.
        let paths = vec![
            "D:\\Code\\Ministr\\src".into(),
            "D:\\Code\\Ministr\\docs".into(),
            "D:\\Code\\Ministr\\README.md".into(),
        ];
        assert_eq!(display_name_from_paths(&paths), "Ministr");
    }

    #[test]
    fn display_name_preserves_case() {
        // Display name is derived from the *original* paths so the
        // user's casing survives — the canonical (lowercased on
        // Windows) form is for identity only.
        let paths = vec!["/Users/x/MyProject".into()];
        assert_eq!(display_name_from_paths(&paths), "MyProject");
    }

    #[test]
    fn display_name_single_path() {
        let paths = vec!["/Users/x/Code/foo".into()];
        assert_eq!(display_name_from_paths(&paths), "foo");
    }

    #[test]
    fn display_name_empty() {
        assert_eq!(display_name_from_paths(&[]), "");
    }

    #[test]
    fn display_name_handles_trailing_slash() {
        let paths = vec!["/Users/x/Code/foo/".into()];
        assert_eq!(display_name_from_paths(&paths), "foo");
    }

    #[test]
    fn merge_live_overrides_indexing_with_progress() {
        let progress = IngestionProgress::new();
        progress.start(10);
        progress.increment_done();
        progress.increment_done();
        progress.add_sections_done(5);

        let merged = merge_live_info(
            snapshot_with(IndexingStatus::Indexing {
                files_done: 0,
                files_total: 0,
            }),
            &progress,
            42,
        );

        assert!(matches!(
            merged.status,
            IndexingStatus::Indexing {
                files_done: 2,
                files_total: 10,
            }
        ));
        assert_eq!(merged.sections_count, 5);
        assert_eq!(merged.embeddings_count, 42);
    }

    #[test]
    fn merge_live_preserves_error_under_stuck_progress() {
        // Some pipeline error paths return early without calling
        // `progress.complete()`, so `is_running()` can stay true even
        // after the registry transitions to Error. The merge must not
        // mask that.
        let progress = IngestionProgress::new();
        progress.start(10);
        progress.increment_done();

        let merged = merge_live_info(
            snapshot_with(IndexingStatus::Error {
                message: "boom".into(),
            }),
            &progress,
            42,
        );

        assert!(matches!(merged.status, IndexingStatus::Error { .. }));
        assert_eq!(merged.sections_count, 7, "persisted snapshot preserved");
        assert_eq!(merged.embeddings_count, 42, "index always wins");
    }

    #[test]
    fn merge_live_preserves_idle_under_stuck_progress() {
        // Same precedence rule for cancellation: registry sets Idle,
        // progress flag may still read running.
        let progress = IngestionProgress::new();
        progress.start(10);
        progress.increment_done();

        let merged = merge_live_info(snapshot_with(IndexingStatus::Idle), &progress, 42);

        assert!(matches!(merged.status, IndexingStatus::Idle));
        assert_eq!(merged.sections_count, 7);
        assert_eq!(merged.embeddings_count, 42);
    }

    #[test]
    fn merge_live_skips_progress_when_not_running() {
        // Pre-start window: registry has marked Indexing { 0, 0 } but
        // the pipeline hasn't called `progress.start()` yet. Don't
        // synthesize a fake live snapshot — leave the persisted one.
        let progress = IngestionProgress::new();
        let info = snapshot_with(IndexingStatus::Indexing {
            files_done: 0,
            files_total: 0,
        });
        let merged = merge_live_info(info, &progress, 42);

        assert!(matches!(
            merged.status,
            IndexingStatus::Indexing {
                files_done: 0,
                files_total: 0,
            }
        ));
        assert_eq!(merged.sections_count, 7, "persisted snapshot preserved");
        assert_eq!(merged.embeddings_count, 42);
    }

    // -- Completion-channel wiring (PHASE2 chunk 3) --
    //
    // The cloud durability reactor consumes `(corpus_id, corpus_dir)`
    // tuples from this channel and uploads bundles to blob. On self-
    // hosted serve no sink is wired and `notify_complete` is a no-op.

    #[derive(Debug)]
    struct StubEmbedder {
        dim: usize,
    }

    impl ministr_core::embedding::Embedder for StubEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ministr_core::error::IndexError> {
            Ok(texts.iter().map(|_| vec![0.0_f32; self.dim]).collect())
        }
        fn dimension(&self) -> usize {
            self.dim
        }
    }

    fn build_test_registry() -> CorpusRegistry {
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder { dim: 4 });
        CorpusRegistry::new(embedder, MinistrConfig::default())
    }

    #[tokio::test]
    async fn notify_complete_with_sink_sends_corpus_id_and_dir() {
        let registry = build_test_registry();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        registry.set_completion_sink(tx);

        registry.notify_complete("c1", std::path::Path::new("/tmp/c1"));
        let got = rx.recv().await.expect("expected one completion event");

        assert_eq!(got.0, "c1");
        assert_eq!(got.1, std::path::PathBuf::from("/tmp/c1"));
    }

    #[tokio::test]
    async fn notify_complete_without_sink_is_a_no_op() {
        // No `set_completion_sink` — call must not panic and must not
        // block waiting for a phantom receiver.
        let registry = build_test_registry();
        registry.notify_complete("c1", std::path::Path::new("/tmp/c1"));
    }

    #[tokio::test]
    async fn second_set_completion_sink_is_a_no_op() {
        // OnceLock semantics — first sink wins, second call is silently
        // dropped (matches `set_coherence_sink`).
        let registry = build_test_registry();
        let (tx1, mut rx1) = tokio::sync::mpsc::unbounded_channel();
        let (tx2, mut rx2) = tokio::sync::mpsc::unbounded_channel();
        registry.set_completion_sink(tx1);
        registry.set_completion_sink(tx2);

        registry.notify_complete("c1", std::path::Path::new("/tmp/c1"));

        assert_eq!(
            rx1.recv().await,
            Some(("c1".to_string(), std::path::PathBuf::from("/tmp/c1")))
        );
        assert!(rx2.try_recv().is_err(), "second sink must not receive");
    }

    // -- cq-priority cold-start size signal --

    /// A minimal document with `sections` sections, for populating a test
    /// content.db so `document_count`/`section_count` return known values.
    fn cold_start_doc(id: &str, sections: usize) -> ministr_core::types::DocumentTree {
        use ministr_core::types::{ContentId, DocumentTree, Section, SectionId};
        DocumentTree {
            id: ContentId(id.into()),
            title: id.into(),
            source_path: id.into(),
            sections: (0..sections)
                .map(|i| Section {
                    id: SectionId(format!("{id}#s{i}")),
                    heading_path: vec![format!("s{i}")],
                    depth: 1,
                    text: format!("section {i} of {id}"),
                    structural_nodes: vec![],
                    children: vec![],
                    claims: vec![],
                    summary: None,
                })
                .collect(),
            summary: None,
        }
    }

    fn test_registry_in(data_dir: &std::path::Path) -> CorpusRegistry {
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder { dim: 4 });
        let config = MinistrConfig {
            data_dir: data_dir.to_path_buf(),
            ..MinistrConfig::default()
        };
        CorpusRegistry::new(embedder, config)
    }

    #[tokio::test]
    async fn create_handle_seeds_size_from_existing_content_db() {
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = test_registry_in(tmp.path());

        // Pre-stage an already-indexed content.db at the path `create_handle`
        // opens, as if this corpus had been indexed in a prior daemon session.
        let corpus_id = "cs-seed-test";
        let corpus_dir = tmp.path().join("corpora").join(corpus_id);
        std::fs::create_dir_all(&corpus_dir).unwrap();
        let db = corpus_dir.join("content.db");
        {
            let storage = SqliteStorage::open(&db).unwrap();
            storage
                .insert_document(&cold_start_doc("a.md", 2))
                .await
                .unwrap();
            storage
                .insert_document(&cold_start_doc("b.md", 1))
                .await
                .unwrap();
        }

        // The fresh handle (cold-restart / re-register path) reports the on-disk
        // size immediately — not the placeholder 0 — so cq-priority can order a
        // restore-storm by real size instead of FIFO.
        let corpus_path = tmp.path().to_str().unwrap().to_string();
        let handle = registry
            .create_handle(corpus_id, &[corpus_path], "cs".to_string())
            .await
            .unwrap();
        let info = handle.info.read().await;
        assert_eq!(info.files_indexed, 2, "two documents on disk");
        assert_eq!(info.sections_count, 3, "three sections total");
    }

    #[tokio::test]
    async fn create_handle_on_empty_db_seeds_zero() {
        // A brand-new corpus (no prior index) seeds 0 → treated as small →
        // prompt first-time indexing (unchanged behavior).
        let tmp = tempfile::TempDir::new().unwrap();
        let registry = test_registry_in(tmp.path());
        let corpus_path = tmp.path().to_str().unwrap().to_string();
        let handle = registry
            .create_handle("brand-new", &[corpus_path], "n".to_string())
            .await
            .unwrap();
        assert_eq!(handle.info.read().await.files_indexed, 0);
    }
}
