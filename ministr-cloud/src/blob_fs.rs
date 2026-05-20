//! Filesystem-backed corpus blob store for local development.
//!
//! Drop-in alternative to [`crate::blob::CorpusBlobStore`] that
//! stores corpus bundles + manifests on the local filesystem under a
//! single root directory. Used by [`crate::blob_backend::BlobBackend`]
//! when the operator selects the filesystem variant via
//! `MINISTR_BLOB_STORE_KIND=filesystem` (the default for `just
//! dev-cloud-up`).
//!
//! # Layout
//!
//! ```text
//!   <root>/
//!     <corpus_id>/
//!       manifest.json         — points at the current bundle version
//!       bundles/<version>.tar.zst
//! ```
//!
//! The shape mirrors the Azure container hierarchy
//! (`corpora/<id>/manifest.json`, `corpora/<id>/bundles/<v>.tar.zst`)
//! one-for-one so behaviour stays identical across backends. The only
//! difference is that "atomic swap" of the manifest pointer is a
//! `rename(2)` on POSIX / `MoveFileExW` on Windows instead of a PUT.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ministr_core::bundle::{self, BundleManifest};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::blob::{BlobError, BlobResult, CorpusManifest};

/// Filesystem-backed `BlobStore`.
///
/// Cheap to clone — the inner state is `Arc`-wrapped. The mutex guards
/// per-corpus manifest writes so two concurrent `upload_corpus` calls
/// for the same `corpus_id` produce a coherent final manifest. Reads
/// don't take the lock.
#[derive(Debug, Clone)]
pub struct FilesystemBlobStore {
    root: Arc<PathBuf>,
    /// One mutex per process; coarse but sufficient — uploads are
    /// rare and the contended path is just a `serde_json` + `rename`.
    write_lock: Arc<Mutex<()>>,
}

impl FilesystemBlobStore {
    /// Construct a store rooted at `root`. The directory is created
    /// (recursively) on first use; `ensure_container` is a no-op other
    /// than that.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: Arc::new(root),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Idempotently create the root directory.
    ///
    /// # Errors
    ///
    /// Surfaces filesystem errors creating the root directory.
    pub async fn ensure_container(&self) -> BlobResult<()> {
        tokio::fs::create_dir_all(self.root.as_ref())
            .await
            .map_err(|e| BlobError::Io {
                path: self.root.as_ref().clone(),
                source: e,
            })?;
        Ok(())
    }

    fn corpus_dir(&self, corpus_id: &str) -> PathBuf {
        self.root.join(corpus_id)
    }

    fn manifest_path(&self, corpus_id: &str) -> PathBuf {
        self.corpus_dir(corpus_id).join("manifest.json")
    }

    fn bundle_path(&self, corpus_id: &str, version: &str) -> PathBuf {
        self.corpus_dir(corpus_id)
            .join("bundles")
            .join(format!("{version}.tar.zst"))
    }

    /// Same contract as `CorpusBlobStore::upload_corpus`. Writes the
    /// bundle to disk, atomically swaps the manifest via `rename(2)`,
    /// and returns the persisted version string.
    ///
    /// # Errors
    ///
    /// - [`BlobError::MissingBundleVersion`] when the manifest hasn't
    ///   been populated by `bundle::compute_bundle_version`.
    /// - [`BlobError::Bundle`] from the export step.
    /// - [`BlobError::Io`] for filesystem failures.
    pub async fn upload_corpus(
        &self,
        corpus_id: &str,
        corpus_dir: &Path,
        bundle_manifest: &BundleManifest,
    ) -> BlobResult<String> {
        let version = bundle_manifest
            .bundle_version
            .clone()
            .ok_or(BlobError::MissingBundleVersion)?;

        let _guard = self.write_lock.lock().await;

        let bundles_dir = self.corpus_dir(corpus_id).join("bundles");
        tokio::fs::create_dir_all(&bundles_dir)
            .await
            .map_err(|e| BlobError::Io {
                path: bundles_dir.clone(),
                source: e,
            })?;

        let target = self.bundle_path(corpus_id, &version);
        let bundle_path = bundle::export_bundle(corpus_dir, &target, bundle_manifest)
            .map_err(BlobError::Bundle)?;
        debug!(
            corpus_id,
            version = %version,
            path = %bundle_path.display(),
            "wrote bundle to filesystem"
        );

        // Atomic swap of manifest pointer — write to a sibling and rename.
        let cm = CorpusManifest {
            current_version: version.clone(),
            updated_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        };
        let manifest_tmp = self.corpus_dir(corpus_id).join("manifest.json.tmp");
        let manifest_final = self.manifest_path(corpus_id);
        let bytes = serde_json::to_vec(&cm)?;
        tokio::fs::write(&manifest_tmp, bytes)
            .await
            .map_err(|e| BlobError::Io {
                path: manifest_tmp.clone(),
                source: e,
            })?;
        tokio::fs::rename(&manifest_tmp, &manifest_final)
            .await
            .map_err(|e| BlobError::Io {
                path: manifest_final.clone(),
                source: e,
            })?;
        info!(
            corpus_id,
            version = %version,
            "uploaded corpus bundle and swapped manifest (filesystem)"
        );
        Ok(version)
    }

    /// Same contract as `CorpusBlobStore::download_corpus`.
    ///
    /// # Errors
    ///
    /// Filesystem read failures or malformed bundle.
    pub async fn download_corpus(
        &self,
        corpus_id: &str,
        target_corpus_dir: &Path,
    ) -> BlobResult<BundleManifest> {
        let cm = self.get_manifest(corpus_id).await?;
        let bundle_path = self.bundle_path(corpus_id, &cm.current_version);
        let manifest = bundle::import_bundle(&bundle_path, target_corpus_dir)
            .map_err(BlobError::Bundle)?;
        info!(
            corpus_id,
            version = %cm.current_version,
            "downloaded and restored corpus bundle (filesystem)"
        );
        Ok(manifest)
    }

    /// Read the current `CorpusManifest`.
    ///
    /// # Errors
    ///
    /// Filesystem read failure or malformed JSON.
    pub async fn get_manifest(&self, corpus_id: &str) -> BlobResult<CorpusManifest> {
        let path = self.manifest_path(corpus_id);
        let bytes = tokio::fs::read(&path).await.map_err(|e| BlobError::Io {
            path: path.clone(),
            source: e,
        })?;
        let cm: CorpusManifest = serde_json::from_slice(&bytes)?;
        Ok(cm)
    }

    /// List corpus IDs by scanning for child directories that contain
    /// a `manifest.json`.
    ///
    /// # Errors
    ///
    /// Filesystem read failure.
    pub async fn list_corpora(&self) -> BlobResult<Vec<String>> {
        let mut ids = Vec::new();
        let mut entries = match tokio::fs::read_dir(self.root.as_ref()).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
            Err(e) => {
                return Err(BlobError::Io {
                    path: self.root.as_ref().clone(),
                    source: e,
                });
            }
        };
        while let Some(entry) = entries.next_entry().await.map_err(|e| BlobError::Io {
            path: self.root.as_ref().clone(),
            source: e,
        })? {
            let manifest = entry.path().join("manifest.json");
            if tokio::fs::try_exists(&manifest).await.unwrap_or(false)
                && let Some(name) = entry.file_name().to_str()
            {
                ids.push(name.to_owned());
            }
        }
        Ok(ids)
    }

    /// Delete a corpus's manifest + best-effort delete its current
    /// bundle.
    ///
    /// # Errors
    ///
    /// Filesystem failures other than "not found".
    pub async fn delete_corpus(&self, corpus_id: &str) -> BlobResult<()> {
        let _guard = self.write_lock.lock().await;
        let current_version = match self.get_manifest(corpus_id).await {
            Ok(m) => Some(m.current_version),
            Err(BlobError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                None
            }
            Err(e) => return Err(e),
        };
        let manifest = self.manifest_path(corpus_id);
        match tokio::fs::remove_file(&manifest).await {
            Ok(()) => debug!(corpus_id, "deleted corpus manifest (filesystem)"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(corpus_id, "corpus manifest already absent");
            }
            Err(e) => {
                return Err(BlobError::Io {
                    path: manifest,
                    source: e,
                });
            }
        }
        if let Some(version) = current_version {
            let bundle = self.bundle_path(corpus_id, &version);
            if let Err(e) = tokio::fs::remove_file(&bundle).await
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    corpus_id,
                    version,
                    error = %e,
                    "failed to delete bundle during delete_corpus; leaving for GC"
                );
            }
        }
        info!(corpus_id, "deleted corpus (filesystem)");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn list_returns_empty_when_root_missing() {
        let dir = tempdir().unwrap();
        // Use a subdir that doesn't exist yet.
        let store = FilesystemBlobStore::new(dir.path().join("nope"));
        assert!(store.list_corpora().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn ensure_container_creates_root_directory() {
        let dir = tempdir().unwrap();
        let store = FilesystemBlobStore::new(dir.path().join("created"));
        store.ensure_container().await.unwrap();
        assert!(tokio::fs::try_exists(dir.path().join("created")).await.unwrap());
    }

    #[tokio::test]
    async fn get_manifest_surfaces_not_found_as_io_error() {
        let dir = tempdir().unwrap();
        let store = FilesystemBlobStore::new(dir.path().to_path_buf());
        let err = store.get_manifest("nope").await.expect_err("missing");
        match err {
            BlobError::Io { source, .. } => {
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected Io NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_corpora_picks_up_directories_with_manifests() {
        let dir = tempdir().unwrap();
        let store = FilesystemBlobStore::new(dir.path().to_path_buf());
        // Seed the layout directly without running upload_corpus (would
        // need a real BundleManifest + corpus dir).
        let corpus_dir = dir.path().join("corp-1");
        tokio::fs::create_dir_all(&corpus_dir).await.unwrap();
        tokio::fs::write(
            corpus_dir.join("manifest.json"),
            r#"{"current_version":"v1","updated_at":1}"#,
        )
        .await
        .unwrap();
        // Sibling dir without a manifest — should NOT be returned.
        tokio::fs::create_dir_all(dir.path().join("not-a-corpus")).await.unwrap();

        let ids = store.list_corpora().await.unwrap();
        assert_eq!(ids, vec!["corp-1".to_string()]);
    }

    #[tokio::test]
    async fn delete_corpus_is_idempotent() {
        let dir = tempdir().unwrap();
        let store = FilesystemBlobStore::new(dir.path().to_path_buf());
        // No manifest to start — delete_corpus should succeed.
        store.delete_corpus("nope").await.unwrap();
    }
}
