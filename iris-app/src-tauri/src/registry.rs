//! [`CorpusRegistry`] — central manager for all indexed corpora.
//!
//! Owns the shared embedding model (loaded once) and a map of
//! [`CorpusHandle`] instances, each with its own storage, vector index,
//! and query service. Thread-safe via interior `RwLock`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iris_api::corpus::{CorpusInfo, IndexingStatus};
use iris_core::config::IrisConfig;
use iris_core::embedding::Embedder;
use iris_core::index::{HnswIndex, VectorIndex, VectorIndexLoad};
use iris_core::ingestion::IngestionProgress;
use iris_core::service::QueryService;
use iris_core::storage::SqliteStorage;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Errors from corpus registry operations.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// A storage operation failed.
    #[error("storage: {0}")]
    Storage(String),
    /// An embedding model error.
    #[error("embedding: {0}")]
    Embedding(String),
    /// An index error.
    #[error("index: {0}")]
    Index(String),
    /// Corpus not found.
    #[error("corpus not found: {id}")]
    NotFound { id: String },
}

/// Central registry managing all indexed corpora.
///
/// The embedding model is loaded **once** and shared by all corpora,
/// saving ~200 MB of memory per additional corpus.
pub struct CorpusRegistry {
    /// Shared embedding model.
    embedder: Arc<dyn Embedder>,
    /// Active corpora, keyed by corpus ID.
    corpora: RwLock<HashMap<String, CorpusHandle>>,
    /// Global configuration.
    config: IrisConfig,
}

/// A single managed corpus with its resources.
pub struct CorpusHandle {
    /// Corpus metadata.
    pub info: CorpusInfo,
    /// Persistent storage (SQLite).
    pub storage: Arc<SqliteStorage>,
    /// Dense vector index (HNSW).
    pub index: Arc<dyn VectorIndex>,
    /// Query service facade.
    pub service: QueryService,
    /// Indexing progress tracker.
    pub progress: Arc<IngestionProgress>,
    /// Cancellation token for background indexing.
    pub cancel: CancellationToken,
    /// Filesystem paths that make up this corpus.
    pub paths: Vec<PathBuf>,
    /// Data directory for this corpus.
    pub data_dir: PathBuf,
}

impl CorpusRegistry {
    /// Create a new registry with the given embedding model and config.
    pub fn new(embedder: Arc<dyn Embedder>, config: IrisConfig) -> Self {
        Self {
            embedder,
            corpora: RwLock::new(HashMap::new()),
            config,
        }
    }

    /// The shared embedding model.
    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    /// The global configuration.
    pub fn config(&self) -> &IrisConfig {
        &self.config
    }

    /// Register a corpus and prepare it for indexing.
    ///
    /// If the corpus is already registered (same ID), returns the existing
    /// info without re-registering.
    pub async fn register(
        &self,
        paths: &[String],
    ) -> Result<(String, bool), RegistryError> {
        let corpus_id = corpus_id_from_paths(paths);

        // Check if already registered.
        {
            let corpora = self.corpora.read().await;
            if corpora.contains_key(&corpus_id) {
                return Ok((corpus_id, false));
            }
        }

        // Initialize storage and index for this corpus.
        let corpus_dir = self.config.data_dir.join("corpora").join(&corpus_id);
        let db_path = corpus_dir.join("content.db");
        let index_dir = corpus_dir.join("index");

        std::fs::create_dir_all(&corpus_dir).map_err(|e| {
            RegistryError::Storage(format!("failed to create corpus dir: {e}"))
        })?;

        let storage = SqliteStorage::open(&db_path)
            .map_err(|e| RegistryError::Storage(format!("failed to open database: {e}")))?;
        let storage = Arc::new(storage);

        let dim = self.embedder.dimension();
        let index: Arc<dyn VectorIndex> = if index_dir.exists() {
            match HnswIndex::load(&index_dir) {
                Ok(loaded) => Arc::new(loaded),
                Err(e) => {
                    warn!(error = %e, "corrupted vector index — rebuilding");
                    let _ = std::fs::remove_dir_all(&index_dir);
                    Arc::new(
                        HnswIndex::new(dim, 100_000)
                            .map_err(|e| RegistryError::Index(e.to_string()))?,
                    )
                }
            }
        } else {
            Arc::new(
                HnswIndex::new(dim, 100_000)
                    .map_err(|e| RegistryError::Index(e.to_string()))?,
            )
        };

        // Build query service.
        let query_storage = SqliteStorage::open(&db_path)
            .map_err(|e| RegistryError::Storage(format!("failed to open query storage: {e}")))?;
        let service = QueryService::new(
            query_storage,
            Arc::clone(&self.embedder),
            Arc::clone(&index),
        );

        let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let info = CorpusInfo {
            id: corpus_id.clone(),
            paths: paths.to_vec(),
            status: IndexingStatus::Idle,
            files_indexed: 0,
            sections_count: 0,
            embeddings_count: 0,
        };

        let handle = CorpusHandle {
            info,
            storage,
            index,
            service,
            progress: Arc::new(IngestionProgress::new()),
            cancel: CancellationToken::new(),
            paths: path_bufs,
            data_dir: corpus_dir,
        };

        let mut corpora = self.corpora.write().await;
        corpora.insert(corpus_id.clone(), handle);

        info!(corpus_id = %corpus_id, paths = ?paths, "corpus registered");
        Ok((corpus_id, true))
    }

    /// Unregister and remove a corpus.
    pub async fn unregister(&self, corpus_id: &str) -> Result<(), RegistryError> {
        let mut corpora = self.corpora.write().await;
        if let Some(handle) = corpora.remove(corpus_id) {
            handle.cancel.cancel();
            info!(corpus_id = %corpus_id, "corpus unregistered");
            Ok(())
        } else {
            Err(RegistryError::NotFound {
                id: corpus_id.to_string(),
            })
        }
    }

    /// List all registered corpora.
    pub async fn list(&self) -> Vec<CorpusInfo> {
        let corpora = self.corpora.read().await;
        corpora.values().map(|h| h.info.clone()).collect()
    }

    /// Get a read guard and look up a corpus by ID.
    ///
    /// Returns a reference to the `CorpusHandle` if found.
    /// The caller must hold the returned guard for the duration of use.
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
}

/// Compute a deterministic corpus ID from a set of paths.
fn corpus_id_from_paths(paths: &[String]) -> String {
    let mut sorted = paths.to_vec();
    sorted.sort();
    let joined = sorted.join("\n");
    let hash = Sha256::digest(joined.as_bytes());
    format!("multi-{}", &hex::encode(hash.as_slice())[..8])
}

/// Hex encoding without pulling in the `hex` crate.
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
