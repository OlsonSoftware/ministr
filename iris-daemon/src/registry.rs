//! [`CorpusRegistry`] — lifecycle manager for indexed corpora.
//!
//! Owns the shared embedding model (loaded once) and a map of
//! [`CorpusHandle`] instances. Thread-safe via interior `RwLock`.
//!
//! Ingestion is delegated to the [`indexer`](crate::indexer) module (SRP).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iris_api::corpus::{CorpusInfo, IndexingStatus};
use iris_core::config::IrisConfig;
use iris_core::embedding::Embedder;
use iris_core::index::{HnswIndex, VectorIndex, VectorIndexLoad};
use iris_core::ingestion::IngestionProgress;
use iris_core::service::QueryService;
use iris_core::session::prefetch::PrefetchEngine;
use iris_core::session::{BudgetConfig, SessionRegistry};
use iris_core::storage::SqliteStorage;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::indexer;

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
    /// Broadcast channel for coherence notifications (stale section IDs).
    pub coherence_tx: tokio::sync::broadcast::Sender<Vec<String>>,
}

impl CorpusRegistry {
    pub fn new(embedder: Arc<dyn Embedder>, config: IrisConfig) -> Self {
        Self {
            embedder,
            corpora: RwLock::new(HashMap::new()),
            config,
        }
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

    /// Register a corpus, initialize its resources, and spawn background indexing.
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

        let handle = self.create_handle(&corpus_id, paths)?;

        self.corpora.write().await.insert(corpus_id.clone(), handle);
        info!(corpus_id = %corpus_id, "corpus registered");

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
        match self.corpora.write().await.remove(corpus_id) {
            Some(handle) => {
                handle.cancel.cancel();
                info!(corpus_id, "corpus unregistered");
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
            result.push(handle.info.read().await.clone());
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
        }
    }

    // -- Private --

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
