//! `BlobSink` impl that exports a corpus to a bundle and uploads to blob.
//!
//! `BlobBackendSink` is the cloud-side concrete implementation of the
//! [`ministr_api::BlobSink`] trait. The daemon's registry completion
//! reactor fires `enqueue_upload` after every successful ingestion;
//! the sink spawns a tokio task that:
//!
//! 1. Opens the corpus's `SQLite` storage at `<corpus_dir>/content.db`
//!    to enumerate corpus roots and count documents.
//! 2. Loads the `HNSW` index at `<corpus_dir>/index/` to populate
//!    `vector_count` + `dimension`.
//! 3. Computes the deterministic `bundle_version` from the corpus
//!    roots' commit SHAs.
//! 4. Calls [`BlobBackend::upload_corpus`], which builds the bundle on
//!    local `/tmp`, fsyncs it, `PUT`s the versioned blob, then
//!    atomic-swaps the `manifest.json` pointer.
//!
//! Errors are logged at `warn` level and dropped — the caller (registry
//! completion reactor) is fire-and-forget by contract and never observes
//! storage hiccups.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ministr_api::BlobSink;
use ministr_core::bundle::{
    compute_bundle_version, BundleCorpusRoot, BundleManifest, BUNDLE_FORMAT_VERSION,
};
use ministr_core::index::{HnswIndex, VectorIndex as _, VectorIndexLoad as _};
use ministr_core::storage::{SqliteStorage, Storage as _};

use crate::blob_backend::BlobBackend;

/// `BlobSink` impl that wraps a [`BlobBackend`] (Azure or filesystem).
///
/// Cheap to clone — the backend handle is `Arc`-backed internally.
#[derive(Debug, Clone)]
pub struct BlobBackendSink {
    backend: Arc<BlobBackend>,
    model_name: String,
}

impl BlobBackendSink {
    /// Construct a sink that exports corpora through `backend`.
    ///
    /// `model_name` is the embedding-model identifier recorded in the
    /// bundle manifest. Dimension is *not* a constructor parameter — it
    /// is read from the actual HNSW index on disk at upload time, so a
    /// stale config value can't desync the manifest from the index.
    #[must_use]
    pub fn new(backend: Arc<BlobBackend>, model_name: String) -> Self {
        Self {
            backend,
            model_name,
        }
    }
}

impl BlobSink for BlobBackendSink {
    fn enqueue_upload(&self, corpus_id: String, corpus_dir: PathBuf) {
        let backend = Arc::clone(&self.backend);
        let model_name = self.model_name.clone();
        tokio::spawn(async move {
            let manifest = match build_manifest_from_corpus_dir(&corpus_dir, &model_name).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        corpus_id = %corpus_id,
                        corpus_dir = %corpus_dir.display(),
                        error = %e,
                        "blob upload skipped — manifest construction failed"
                    );
                    return;
                }
            };
            match backend
                .upload_corpus(&corpus_id, &corpus_dir, &manifest)
                .await
            {
                Ok(version) => tracing::info!(
                    corpus_id = %corpus_id,
                    version = %version,
                    "uploaded corpus bundle to blob"
                ),
                Err(e) => tracing::warn!(
                    corpus_id = %corpus_id,
                    error = %e,
                    "blob upload failed — corpus state will be ephemeral until next ingest"
                ),
            }
        });
    }
}

/// Errors surfaced while assembling a [`BundleManifest`] from a
/// corpus data directory.
#[derive(Debug, thiserror::Error)]
pub enum ManifestBuildError {
    #[error("storage open at {path}: {source}")]
    StorageOpen {
        path: PathBuf,
        source: ministr_core::error::StorageError,
    },
    #[error("storage read: {0}")]
    StorageRead(ministr_core::error::StorageError),
}

/// Build a complete [`BundleManifest`] (with `bundle_version` populated)
/// from a corpus data directory by reading its `SQLite` storage + HNSW
/// index.
///
/// Mirrors `ministr-mcp::bundle_routes::build_manifest`. Kept here so
/// cloud-only code does not depend on `ministr-mcp` (which is MIT and
/// must remain importable without this proprietary crate).
///
/// # Errors
///
/// Returns [`ManifestBuildError`] when the storage cannot be opened or
/// the document/root counts cannot be read.
pub async fn build_manifest_from_corpus_dir(
    corpus_dir: &Path,
    model_name: &str,
) -> Result<BundleManifest, ManifestBuildError> {
    let storage_path = corpus_dir.join("content.db");
    let storage = SqliteStorage::open(&storage_path).map_err(|source| {
        ManifestBuildError::StorageOpen {
            path: storage_path.clone(),
            source,
        }
    })?;

    let doc_count = storage
        .document_count()
        .await
        .map_err(ManifestBuildError::StorageRead)?;
    let roots = storage
        .list_corpus_roots()
        .await
        .map_err(ManifestBuildError::StorageRead)?;

    let bundle_roots: Vec<BundleCorpusRoot> = roots
        .iter()
        .map(|r| BundleCorpusRoot {
            id: r.id.clone(),
            display_name: r.display_name.clone(),
            kind: r.kind.as_str().to_string(),
            commit_sha: r.commit_sha.clone(),
            branch: r.branch.clone(),
            repo_url: r.repo_url.clone(),
        })
        .collect();

    let source_commit = bundle_roots.iter().find_map(|r| r.commit_sha.clone());
    let bundle_version = Some(compute_bundle_version(&bundle_roots));

    let index_dir = corpus_dir.join("index");
    let (vector_count, dimension) = if index_dir.exists() {
        match HnswIndex::load(&index_dir) {
            Ok(loaded) => (loaded.len(), loaded.dimension()),
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };

    Ok(BundleManifest {
        format_version: BUNDLE_FORMAT_VERSION,
        model_name: model_name.to_string(),
        dimension,
        vector_count,
        document_count: doc_count,
        symbol_count: 0,
        corpus_roots: bundle_roots,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        bundle_version,
        source_commit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob_fs::FilesystemBlobStore;

    /// Feeding a non-existent `corpus_dir` must warn-log and silently
    /// drop the upload — no panics, no partial blob state.
    #[tokio::test]
    async fn enqueue_upload_with_missing_corpus_dir_is_a_no_op() {
        let blob_root = tempfile::tempdir().unwrap();
        let store = FilesystemBlobStore::new(blob_root.path().to_path_buf());
        let backend = Arc::new(BlobBackend::Filesystem(store));
        let sink = BlobBackendSink::new(Arc::clone(&backend), "test-model".to_string());

        sink.enqueue_upload(
            "nonexistent-corpus".to_string(),
            PathBuf::from("/tmp/ministr-blob-sink-test-does-not-exist"),
        );

        // Give the spawned task a chance to run + log its warn. We don't
        // wait on the JoinHandle because the trait is intentionally
        // fire-and-forget (no handle returned).
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }

        // No corpus was uploaded — the FS store's `corpora/` root must
        // be empty (or contain nothing for this corpus_id).
        let corpora = backend.list_corpora().await.unwrap_or_default();
        assert!(
            corpora.is_empty(),
            "expected no corpora to land in blob store, got {corpora:?}"
        );
    }

    #[test]
    fn sink_is_dyn_compatible() {
        let blob_root = tempfile::tempdir().unwrap();
        let store = FilesystemBlobStore::new(blob_root.path().to_path_buf());
        let backend = Arc::new(BlobBackend::Filesystem(store));
        let _sink: Arc<dyn BlobSink> = Arc::new(BlobBackendSink::new(
            backend,
            "test-model".to_string(),
        ));
    }
}
