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
use ministr_api::corpus::{CorpusInfo, IndexingStatus};
use ministr_core::config::MinistrConfig;
use ministr_core::corpus_id::{CorpusIdError, canonical_corpus_paths, corpus_id_from_paths};
use ministr_core::embedding::Embedder;
use ministr_core::index::{HnswIndex, VectorIndex, VectorIndexLoad};
use ministr_core::ingestion::IngestionProgress;
use ministr_core::service::QueryService;
use ministr_core::session::prefetch::PrefetchEngine;
use ministr_core::session::{BudgetConfig, SessionRegistry};
use ministr_core::storage::SqliteStorage;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::indexer;

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
    corpora: RwLock<HashMap<String, CorpusHandle>>,
    config: MinistrConfig,
    /// Optional sink for coherence events — wired in by [`AppState::new`]
    /// after construction so `register` can spawn a pusher task that
    /// feeds the app-level ring buffer without the registry needing a
    /// direct handle to [`AppState`].
    coherence_sink: std::sync::OnceLock<Arc<RwLock<VecDeque<CoherenceEvent>>>>,
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
    pub sessions: tokio::sync::Mutex<SessionRegistry>,
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
        Self {
            embedder,
            corpora: RwLock::new(HashMap::new()),
            config,
            coherence_sink: std::sync::OnceLock::new(),
        }
    }

    /// Wire a coherence sink so `register` will spawn a per-corpus
    /// pusher task that copies events into the supplied buffer.
    ///
    /// Intended to be called exactly once, immediately after construction.
    /// A second call is a no-op (the first wins).
    pub fn set_coherence_sink(&self, sink: Arc<RwLock<VecDeque<CoherenceEvent>>>) {
        let _ = self.coherence_sink.set(sink);
    }

    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    pub fn config(&self) -> &MinistrConfig {
        &self.config
    }

    /// Internal access to the corpora map (for the indexer and daemon).
    pub fn corpora(&self) -> &RwLock<HashMap<String, CorpusHandle>> {
        &self.corpora
    }

    /// Restore previously registered corpora from the on-disk manifest.
    ///
    /// Reads `{data_dir}/corpora.json` and re-registers each entry.
    /// Skips entries whose source paths no longer exist on disk.
    /// Safe to call on an empty registry — idempotent with `register`.
    pub async fn restore(self: &Arc<Self>) {
        let manifest_path = self.manifest_path();
        let entries = match std::fs::read_to_string(&manifest_path) {
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
        };

        info!(count = entries.len(), "restoring corpora from manifest");

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

        for entry in &live {
            if let Err(e) = self.register(&entry.paths).await {
                warn!(corpus_id = %entry.id, error = %e, "failed to restore corpus");
            }
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
        let handle = self.create_handle(&corpus_id, &canonical, display_name)?;

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
            map.insert(corpus_id.clone(), handle);
        }
        info!(corpus_id = %corpus_id, "corpus registered");

        // Manifest persistence failure is non-fatal for the in-memory
        // registration (the corpus is usable this session) but must not
        // be silent — surface it so a restart-loses-corpus is diagnosable.
        if let Err(e) = self.save_manifest().await {
            warn!(corpus_id = %corpus_id, error = %e, "failed to persist corpus manifest after register");
        }

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

        // Spawn background indexing (delegated to indexer module).
        let registry = Arc::clone(self);
        let cid = corpus_id.clone();
        let owned_paths = canonical.clone();
        spawned.push(tokio::spawn(async move {
            indexer::run(&registry, &cid, &owned_paths).await;
            // After initial indexing, start watching for file changes.
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
        let (info_lock, tasks) = {
            let corpora = self.corpora.read().await;
            let handle = corpora
                .get(corpus_id)
                .ok_or_else(|| RegistryError::NotFound {
                    id: corpus_id.to_string(),
                })?;
            (Arc::clone(&handle.info), Arc::clone(&handle.tasks))
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

        if !added.is_empty() {
            let registry = Arc::clone(self);
            let cid = corpus_id.to_string();
            let h = tokio::spawn(async move {
                indexer::run(&registry, &cid, &added).await;
            });
            // Track the re-ingest task so a later `unregister` awaits it
            // before deleting the corpus dir.
            if let Ok(mut guard) = tasks.lock() {
                guard.push(h);
            }
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
        Ok(())
    }

    /// List all registered corpora with current status.
    pub async fn list(&self) -> Vec<CorpusInfo> {
        let corpora = self.corpora.read().await;
        let mut result = Vec::with_capacity(corpora.len());
        for handle in corpora.values() {
            let mut info = handle.current_info().await;
            info.active_sessions = handle.sessions.lock().await.session_count();
            result.push(info);
        }
        result
    }

    /// Get a read guard to access a corpus by ID.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::NotFound`] if the corpus does not exist.
    pub async fn get(
        &self,
        corpus_id: &str,
    ) -> Result<tokio::sync::RwLockReadGuard<'_, HashMap<String, CorpusHandle>>, RegistryError>
    {
        let corpora = self.corpora.read().await;
        if corpora.contains_key(corpus_id) {
            Ok(corpora)
        } else {
            Err(RegistryError::NotFound {
                id: corpus_id.to_string(),
            })
        }
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
        std::fs::write(&path, json).map_err(|e| {
            RegistryError::Storage(format!("write manifest {}: {e}", path.display()))
        })?;
        Ok(())
    }

    fn create_handle(
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

        let dim = self.embedder.dimension();
        let index = load_or_create_index(&index_dir, dim)?;

        let query_storage = SqliteStorage::open(&db_path)
            .map_err(|e| RegistryError::Storage(format!("open query db: {e}")))?;
        let service = QueryService::new(
            query_storage,
            Arc::clone(&self.embedder),
            Arc::clone(&index),
        );

        Ok(CorpusHandle {
            info: Arc::new(RwLock::new(CorpusInfo {
                id: corpus_id.to_string(),
                display_name,
                paths: paths.to_vec(),
                status: IndexingStatus::Idle,
                files_indexed: 0,
                sections_count: 0,
                embeddings_count: index.len(),
                active_sessions: 0,
                last_indexed: None,
                symbols_count: 0,
            })),
            storage,
            index,
            service,
            sessions: tokio::sync::Mutex::new(SessionRegistry::new(BudgetConfig::default())),
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

fn load_or_create_index(
    index_dir: &std::path::Path,
    dim: usize,
) -> Result<Arc<dyn VectorIndex>, RegistryError> {
    if index_dir.exists() {
        match HnswIndex::load(index_dir) {
            Ok(loaded) => return Ok(Arc::new(loaded)),
            Err(e) => {
                warn!(error = %e, "corrupted index — rebuilding");
                let _ = std::fs::remove_dir_all(index_dir);
            }
        }
    }
    Ok(Arc::new(
        HnswIndex::new(dim, 100_000).map_err(|e| RegistryError::Index(e.to_string()))?,
    ))
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
        }
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
}
