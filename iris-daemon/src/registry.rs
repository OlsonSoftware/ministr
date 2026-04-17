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

use iris_api::coherence::CoherenceEvent;
use iris_api::corpus::{CorpusInfo, IndexingStatus};
use iris_core::config::IrisConfig;
use iris_core::embedding::Embedder;
use iris_core::index::{HnswIndex, VectorIndex, VectorIndexLoad};
use iris_core::ingestion::IngestionProgress;
use iris_core::service::QueryService;
use iris_core::session::prefetch::PrefetchEngine;
use iris_core::session::{BudgetConfig, SessionRegistry};
use iris_core::storage::SqliteStorage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
}

/// Central registry managing all indexed corpora.
pub struct CorpusRegistry {
    embedder: Arc<dyn Embedder>,
    corpora: RwLock<HashMap<String, CorpusHandle>>,
    config: IrisConfig,
    /// Optional sink for coherence events — wired in by [`AppState::new`]
    /// after construction so `register` can spawn a pusher task that
    /// feeds the app-level ring buffer without the registry needing a
    /// direct handle to [`AppState`].
    coherence_sink: std::sync::OnceLock<Arc<RwLock<VecDeque<CoherenceEvent>>>>,
}

/// A single managed corpus with its resources.
pub struct CorpusHandle {
    pub info: RwLock<CorpusInfo>,
    pub storage: Arc<SqliteStorage>,
    pub index: Arc<dyn VectorIndex>,
    pub service: QueryService,
    pub sessions: tokio::sync::Mutex<SessionRegistry>,
    pub prefetch: Arc<tokio::sync::Mutex<PrefetchEngine>>,
    pub progress: Arc<IngestionProgress>,
    pub cancel: CancellationToken,
    pub data_dir: PathBuf,
    /// Broadcast channel for coherence notifications — one event per
    /// file-system change the watcher observed, carrying path, kind, and
    /// the list of affected section IDs.
    pub coherence_tx: tokio::sync::broadcast::Sender<CoherenceEvent>,
}

impl CorpusRegistry {
    pub fn new(embedder: Arc<dyn Embedder>, config: IrisConfig) -> Self {
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

    pub fn config(&self) -> &IrisConfig {
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
        for entry in &entries {
            if let Err(e) = self.register(&entry.paths).await {
                warn!(corpus_id = %entry.id, error = %e, "failed to restore corpus");
            }
        }
    }

    /// Register a corpus, initialize its resources, and spawn background indexing.
    ///
    /// If an existing corpus shares the same project root (common ancestor
    /// directory) but has a different path set, the stale entry is replaced.
    /// This prevents duplicate registrations when `.iris.toml` paths change.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if storage or index initialization fails.
    pub async fn register(
        self: &Arc<Self>,
        paths: &[String],
    ) -> Result<(String, bool), RegistryError> {
        let corpus_id = corpus_id_from_paths(paths);

        if self.corpora.read().await.contains_key(&corpus_id) {
            return Ok((corpus_id, false));
        }

        // Check for an existing corpus with the same project root but different
        // paths (e.g. user added a path in .iris.toml). Replace it.
        let new_root = project_root_from_paths(paths);
        let stale_id = {
            let corpora = self.corpora.read().await;
            let mut found = None;
            for (id, handle) in corpora.iter() {
                let info = handle.info.read().await;
                if project_root_from_paths(&info.paths) == new_root {
                    found = Some(id.clone());
                    break;
                }
            }
            found
        };
        if let Some(old_id) = stale_id {
            info!(old_id = %old_id, new_id = %corpus_id, "replacing stale corpus (paths changed)");
            // Best-effort unregister — ignore NotFound race.
            let _ = self.unregister(&old_id).await;
        }

        let handle = self.create_handle(&corpus_id, paths)?;

        // Subscribe to coherence broadcasts for answer cache invalidation
        // BEFORE inserting the handle (the tx is on the handle).
        let coherence_rx = handle.coherence_tx.subscribe();
        let cache_storage = Arc::clone(&handle.storage);
        let cache_cid = corpus_id.clone();

        // If an observatory sink is wired, subscribe a second receiver
        // and push each event into the shared ring buffer.
        let sink_rx = handle.coherence_tx.subscribe();
        let sink_opt = self.coherence_sink.get().cloned();

        self.corpora.write().await.insert(corpus_id.clone(), handle);
        info!(corpus_id = %corpus_id, "corpus registered");

        self.save_manifest().await;

        // Spawn answer cache invalidation on coherence events.
        tokio::spawn(async move {
            crate::ask::spawn_cache_invalidator(cache_storage, coherence_rx, cache_cid).await;
        });

        if let Some(sink) = sink_opt {
            tokio::spawn(spawn_coherence_sink_pusher(sink, sink_rx));
        }

        // Spawn background indexing (delegated to indexer module).
        let registry = Arc::clone(self);
        let cid = corpus_id.clone();
        let owned_paths = paths.to_vec();
        tokio::spawn(async move {
            indexer::run(&registry, &cid, &owned_paths).await;
            // After initial indexing, start watching for file changes.
            indexer::spawn_watcher(registry, cid, owned_paths);
        });

        Ok((corpus_id, true))
    }

    /// Unregister a corpus, cancelling any background work.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::NotFound`] if the corpus does not exist.
    pub async fn unregister(&self, corpus_id: &str) -> Result<(), RegistryError> {
        // Extract from the map first, releasing the write lock before
        // save_manifest (which needs a read lock — same RwLock, not reentrant).
        let removed = self.corpora.write().await.remove(corpus_id);
        match removed {
            Some(handle) => {
                handle.cancel.cancel();
                info!(corpus_id, "corpus unregistered");
                self.save_manifest().await;
                Ok(())
            }
            None => Err(RegistryError::NotFound {
                id: corpus_id.to_string(),
            }),
        }
    }

    /// List all registered corpora with current status.
    pub async fn list(&self) -> Vec<CorpusInfo> {
        let corpora = self.corpora.read().await;
        let mut result = Vec::with_capacity(corpora.len());
        for handle in corpora.values() {
            let mut info = handle.info.read().await.clone();
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

    /// Update indexing status for a corpus.
    pub async fn set_status(&self, corpus_id: &str, status: IndexingStatus) {
        if let Some(handle) = self.corpora.read().await.get(corpus_id) {
            handle.info.write().await.status = status;
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
        if let Some(handle) = self.corpora.read().await.get(corpus_id) {
            let mut info = handle.info.write().await;
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
        if let Some(handle) = self.corpora.read().await.get(corpus_id) {
            handle.info.write().await.symbols_count = symbols_count;
        }
    }

    // -- Private --

    fn manifest_path(&self) -> PathBuf {
        self.config.data_dir.join("corpora.json")
    }

    /// Persist the current corpus registrations to disk.
    async fn save_manifest(&self) {
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
        match serde_json::to_string_pretty(&entries) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    warn!(error = %e, "failed to write corpus manifest");
                }
            }
            Err(e) => warn!(error = %e, "failed to serialize corpus manifest"),
        }
    }

    fn create_handle(
        &self,
        corpus_id: &str,
        paths: &[String],
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
            info: RwLock::new(CorpusInfo {
                id: corpus_id.to_string(),
                paths: paths.to_vec(),
                status: IndexingStatus::Idle,
                files_indexed: 0,
                sections_count: 0,
                embeddings_count: index.len(),
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
            data_dir: corpus_dir,
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

/// Derive a deterministic corpus ID from sorted paths.
#[must_use]
pub fn corpus_id_from_paths(paths: &[String]) -> String {
    use std::fmt::Write;
    let mut sorted = paths.to_vec();
    sorted.sort();
    let hash = Sha256::digest(sorted.join("\n").as_bytes());
    let hex = hash.iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    });
    format!("multi-{}", &hex[..8])
}

/// Derive the project root directory from corpus paths.
///
/// Computes the longest common ancestor of all paths. For single-path
/// corpora like `["/Users/x/project/src"]`, returns the parent
/// (`/Users/x/project`). For multi-path corpora, returns the deepest
/// shared directory.
#[must_use]
pub fn project_root_from_paths(paths: &[String]) -> std::path::PathBuf {
    if paths.is_empty() {
        return std::path::PathBuf::new();
    }
    if paths.len() == 1 {
        // Single path: go up one level (src → project root).
        let p = std::path::Path::new(&paths[0]);
        return p.parent().unwrap_or(p).to_path_buf();
    }
    // Multi-path: find common ancestor.
    let segments: Vec<Vec<&str>> = paths
        .iter()
        .map(|p| p.split('/').collect::<Vec<_>>())
        .collect();
    let mut common = 0;
    'outer: for i in 0..segments[0].len() {
        for seg in &segments[1..] {
            if i >= seg.len() || seg[i] != segments[0][i] {
                break 'outer;
            }
        }
        common = i + 1;
    }
    std::path::PathBuf::from(segments[0][..common].join("/"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn project_root_single_path() {
        let paths = vec!["/Users/x/Code/pretext/src".to_string()];
        assert_eq!(
            project_root_from_paths(&paths),
            Path::new("/Users/x/Code/pretext")
        );
    }

    #[test]
    fn project_root_multi_path() {
        let paths = vec![
            "/Users/x/Code/pretext/src".to_string(),
            "/Users/x/Code/pretext/docs".to_string(),
        ];
        assert_eq!(
            project_root_from_paths(&paths),
            Path::new("/Users/x/Code/pretext")
        );
    }

    #[test]
    fn project_root_empty() {
        let paths: Vec<String> = vec![];
        assert_eq!(project_root_from_paths(&paths), Path::new(""));
    }

    #[test]
    fn project_root_deeply_nested() {
        let paths = vec!["/a/b/c/src/lib".to_string(), "/a/b/c/src/bin".to_string()];
        assert_eq!(project_root_from_paths(&paths), Path::new("/a/b/c/src"));
    }

    #[test]
    fn corpus_id_deterministic() {
        let a = corpus_id_from_paths(&["b".into(), "a".into()]);
        let b = corpus_id_from_paths(&["a".into(), "b".into()]);
        assert_eq!(a, b, "order should not matter");
    }

    #[test]
    fn corpus_id_changes_with_paths() {
        let a = corpus_id_from_paths(&["src".into()]);
        let b = corpus_id_from_paths(&["src".into(), "docs".into()]);
        assert_ne!(a, b, "different path sets should produce different IDs");
    }

    #[test]
    fn project_root_mixed_depth_paths() {
        // iris-rs case: mix of /*/src dirs and top-level files like README.md
        let paths = vec![
            "/Users/x/Code/iris-rs/iris-api/src".to_string(),
            "/Users/x/Code/iris-rs/iris-core/src".to_string(),
            "/Users/x/Code/iris-rs/README.md".to_string(),
            "/Users/x/Code/iris-rs/docs".to_string(),
        ];
        assert_eq!(
            project_root_from_paths(&paths),
            Path::new("/Users/x/Code/iris-rs")
        );
    }

    #[test]
    fn project_roots_distinct_projects() {
        let a = project_root_from_paths(&["/Users/x/Code/shader-art/src".into()]);
        let b = project_root_from_paths(&[
            "/Users/x/Code/pretext/src".into(),
            "/Users/x/Code/pretext/README.md".into(),
        ]);
        let c = project_root_from_paths(&[
            "/Users/x/Code/iris-rs/iris-api/src".into(),
            "/Users/x/Code/iris-rs/README.md".into(),
        ]);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }
}
