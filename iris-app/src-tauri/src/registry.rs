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
use iris_core::ingestion::{IngestionPipeline, IngestionProgress};
use iris_core::service::QueryService;
use iris_core::storage::SqliteStorage;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Errors from corpus registry operations.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("storage: {0}")]
    Storage(String),
    #[error("embedding: {0}")]
    #[allow(dead_code)]
    Embedding(String),
    #[error("index: {0}")]
    Index(String),
    #[error("corpus not found: {id}")]
    NotFound { id: String },
}

/// Central registry managing all indexed corpora.
///
/// The embedding model is loaded **once** and shared by all corpora,
/// saving ~200 MB of memory per additional corpus.
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
    pub progress: Arc<IngestionProgress>,
    pub cancel: CancellationToken,
    #[allow(dead_code)]
    pub paths: Vec<PathBuf>,
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

    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    pub fn config(&self) -> &IrisConfig {
        &self.config
    }

    /// Register a corpus and spawn background indexing.
    pub async fn register(
        self: &Arc<Self>,
        paths: &[String],
    ) -> Result<(String, bool), RegistryError> {
        let corpus_id = corpus_id_from_paths(paths);

        // Already registered?
        {
            let corpora = self.corpora.read().await;
            if corpora.contains_key(&corpus_id) {
                return Ok((corpus_id, false));
            }
        }

        let corpus_dir = self.config.data_dir.join("corpora").join(&corpus_id);
        let db_path = corpus_dir.join("content.db");
        let index_dir = corpus_dir.join("index");

        std::fs::create_dir_all(&corpus_dir)
            .map_err(|e| RegistryError::Storage(format!("create dir: {e}")))?;

        let storage = Arc::new(
            SqliteStorage::open(&db_path)
                .map_err(|e| RegistryError::Storage(format!("open db: {e}")))?,
        );

        let dim = self.embedder.dimension();
        let index: Arc<dyn VectorIndex> = if index_dir.exists() {
            match HnswIndex::load(&index_dir) {
                Ok(loaded) => Arc::new(loaded),
                Err(e) => {
                    warn!(error = %e, "corrupted index — rebuilding");
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

        let query_storage = SqliteStorage::open(&db_path)
            .map_err(|e| RegistryError::Storage(format!("open query db: {e}")))?;
        let service =
            QueryService::new(query_storage, Arc::clone(&self.embedder), Arc::clone(&index));

        let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let info = CorpusInfo {
            id: corpus_id.clone(),
            paths: paths.to_vec(),
            status: IndexingStatus::Idle,
            files_indexed: 0,
            sections_count: 0,
            embeddings_count: index.len(),
        };

        let handle = CorpusHandle {
            info: RwLock::new(info),
            storage,
            index,
            service,
            progress: Arc::new(IngestionProgress::new()),
            cancel: CancellationToken::new(),
            paths: path_bufs,
            data_dir: corpus_dir,
        };

        {
            let mut corpora = self.corpora.write().await;
            corpora.insert(corpus_id.clone(), handle);
        }

        info!(corpus_id = %corpus_id, paths = ?paths, "corpus registered");

        // Spawn background indexing.
        let registry = Arc::clone(self);
        let cid = corpus_id.clone();
        let owned_paths: Vec<String> = paths.to_vec();
        tokio::spawn(async move {
            registry.run_indexing(&cid, &owned_paths).await;
        });

        Ok((corpus_id, true))
    }

    /// Run ingestion for a corpus. Updates status in the handle.
    async fn run_indexing(&self, corpus_id: &str, paths: &[String]) {
        let (storage, embedder, index, index_dir, progress, _cancel) = {
            let corpora = self.corpora.read().await;
            let Some(handle) = corpora.get(corpus_id) else {
                return;
            };
            (
                Arc::clone(&handle.storage),
                Arc::clone(&self.embedder),
                Arc::clone(&handle.index),
                handle.data_dir.join("index"),
                Arc::clone(&handle.progress),
                handle.cancel.clone(),
            )
        };

        // Set status to indexing.
        self.set_status(corpus_id, IndexingStatus::Indexing {
            files_done: 0,
            files_total: 0,
        })
        .await;

        let local_paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let pipeline = IngestionPipeline::new().with_progress(Arc::clone(&progress));

        let result = pipeline
            .ingest_paths_with_embeddings(&local_paths, &*storage, &*embedder, &*index)
            .await;

        match result {
            Ok(stats) => {
                info!(
                    corpus_id,
                    files_indexed = stats.files_indexed,
                    files_skipped = stats.files_skipped,
                    sections = stats.total_sections,
                    embeddings = stats.total_embeddings,
                    "indexing complete"
                );

                // Persist vector index to disk.
                if let Err(e) = index.persist(&index_dir) {
                    error!(corpus_id, error = %e, "failed to persist index");
                }

                // Update corpus info.
                let corpora = self.corpora.read().await;
                if let Some(handle) = corpora.get(corpus_id) {
                    let mut info = handle.info.write().await;
                    info.status = IndexingStatus::Idle;
                    info.files_indexed = stats.files_indexed;
                    info.sections_count = stats.total_sections;
                    info.embeddings_count = index.len();
                }
            }
            Err(e) => {
                // Cancelled is not an error.
                if matches!(e, iris_core::error::IngestionError::Cancelled) {
                    info!(corpus_id, "indexing cancelled");
                    self.set_status(corpus_id, IndexingStatus::Idle).await;
                } else {
                    error!(corpus_id, error = %e, "indexing failed");
                    self.set_status(
                        corpus_id,
                        IndexingStatus::Error {
                            message: e.to_string(),
                        },
                    )
                    .await;
                }
            }
        }
    }

    async fn set_status(&self, corpus_id: &str, status: IndexingStatus) {
        let corpora = self.corpora.read().await;
        if let Some(handle) = corpora.get(corpus_id) {
            handle.info.write().await.status = status;
        }
    }

    /// Unregister and remove a corpus.
    pub async fn unregister(&self, corpus_id: &str) -> Result<(), RegistryError> {
        let mut corpora = self.corpora.write().await;
        if let Some(handle) = corpora.remove(corpus_id) {
            handle.cancel.cancel();
            info!(corpus_id, "corpus unregistered");
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
        let mut result = Vec::with_capacity(corpora.len());
        for handle in corpora.values() {
            result.push(handle.info.read().await.clone());
        }
        result
    }

    /// Get a read guard to look up a corpus by ID.
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

fn corpus_id_from_paths(paths: &[String]) -> String {
    let mut sorted = paths.to_vec();
    sorted.sort();
    let joined = sorted.join("\n");
    let hash = Sha256::digest(joined.as_bytes());
    format!("multi-{}", &hex::encode(hash.as_slice())[..8])
}

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
