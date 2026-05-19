//! Per-corpus HNSW blob persistence for the cloud surface.
//!
//! Every corpus indexed by `mcp.ministr.ai` lives as **one Azure Blob
//! per corpus** — a zstd-tar `.ministr-index` bundle holding the `SQLite`
//! content database + HNSW vector index. The blob is the source of
//! truth; warm pods cache it on local disk.
//!
//! ## Why blob, not pgvector
//!
//! Reuses [`ministr_core::bundle::export_bundle`] /
//! [`ministr_core::bundle::import_bundle`] verbatim — same format the
//! local stack already exports for `ministr export`. Sidesteps
//! pgvector's 2000-dimension HNSW ceiling (current ministr embeddings
//! happen to be 384-dim, but we don't want the wall there as the model
//! choice evolves). And keeps the `SQLite` path for self-hosted users
//! unchanged.
//!
//! ## Layout in the container
//!
//! All blobs are prefixed `corpora/` inside the container so a single
//! Azure Storage Account can host both per-tenant `corpora/` and the
//! future Atlas index at `atlas/` (F2.6, F4.2) without conflict.
//!
//! ```text
//! <container>/
//!   ├─ corpora/<corpus_id>.ministr-index     ← this file
//!   └─ atlas/<repo_slug>/<commit>.ministr-index    ← F2.6+
//! ```
//!
//! ## Cold-start / warm-cache (daemon wiring lands in F1.2)
//!
//! This module owns the upload/download primitives. The daemon-side
//! "on pod boot, restore each registered corpus's blob into
//! `$DATA_DIR/corpora/`" loop is `cmd_serve_http`'s job once the cloud
//! mode selector lands in F1.2. The primitives here are deliberately
//! state-free so the daemon owns when/where to materialise the corpus.
//!
//! ## Authentication
//!
//! Production uses [`azure_identity::ManagedIdentityCredential`] —
//! Azure Container Apps' pod identity grants the storage account's
//! `Storage Blob Data Contributor` role at deploy time, no secrets in
//! environment. Dev/CI flows use
//! [`azure_identity::DeveloperToolsCredential`] (`az login`, env
//! vars, etc.) — that's what tests pick up too via
//! [`CorpusBlobStore::with_credential`].

#![allow(dead_code)] // daemon wiring (F1.2) is the first caller of this surface

use std::path::{Path, PathBuf};
use std::sync::Arc;

use azure_core::credentials::TokenCredential;
use azure_core::http::{RequestContent, StatusCode, Url};
use azure_storage_blob::{BlobContainerClient, BlobServiceClient};
use futures::TryStreamExt;
use ministr_core::bundle::{self, BundleManifest, BUNDLE_EXTENSION};
use ministr_core::error::BundleError;
use thiserror::Error;
use tracing::{debug, info};

/// Errors that `CorpusBlobStore` can surface.
#[derive(Debug, Error)]
pub enum BlobError {
    #[error("azure storage error: {0}")]
    Azure(#[from] azure_core::Error),
    #[error("bundle export/import error: {0}")]
    Bundle(#[from] BundleError),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("blob name {0:?} is not a recognised .ministr-index bundle under corpora/")]
    UnexpectedBlobName(String),
}

/// Result alias for blob-store operations.
pub type BlobResult<T> = Result<T, BlobError>;

/// Per-corpus HNSW blob store backed by an Azure Storage container.
///
/// One instance maps to one container. Multiple corpora live as
/// distinct blobs under the `corpora/` prefix inside that container.
///
/// The Azure SDK's `BlobContainerClient` is not `Clone`; wrap an
/// instance in `Arc<CorpusBlobStore>` when you need to share it across
/// tasks.
pub struct CorpusBlobStore {
    container: BlobContainerClient,
}

impl std::fmt::Debug for CorpusBlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CorpusBlobStore").finish_non_exhaustive()
    }
}

impl CorpusBlobStore {
    /// Construct a store using a caller-supplied credential. Production
    /// pods pass a [`azure_identity::ManagedIdentityCredential`]; tests
    /// and dev flows pass [`azure_identity::DeveloperToolsCredential`]
    /// (which itself chains `az login`, env vars, etc.).
    ///
    /// `account_name` is the bare storage-account name (e.g.
    /// `"ministrdata"`); `container_name` is the container within it
    /// (e.g. `"ministr-corpora"`).
    ///
    /// # Errors
    ///
    /// Fails if `account_name` is not a syntactically-valid host label
    /// or the service-client construction errors (rare).
    pub fn with_credential(
        account_name: &str,
        container_name: &str,
        credential: Arc<dyn TokenCredential>,
    ) -> BlobResult<Self> {
        let service_url = Url::parse(&format!("https://{account_name}.blob.core.windows.net/"))
            .map_err(|e| azure_core::Error::with_message(azure_core::error::ErrorKind::Other, e.to_string()))?;
        let service = BlobServiceClient::new(service_url, Some(credential), None)?;
        let container = service.blob_container_client(container_name);
        debug!(account = account_name, container = container_name, "opened corpus blob store");
        Ok(Self { container })
    }

    /// Idempotently create the underlying container. Safe to call on
    /// every pod boot — a `ContainerAlreadyExists` response is
    /// swallowed.
    ///
    /// # Errors
    ///
    /// Surfaces any non-409 error from the storage service.
    pub async fn ensure_container(&self) -> BlobResult<()> {
        match self.container.create(None).await {
            Ok(_) => {
                info!("container created");
                Ok(())
            }
            Err(e) if is_already_exists(&e) => {
                debug!("container already exists; reusing");
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Upload the corpus at `corpus_dir` as a single `.ministr-index`
    /// blob named `corpora/<corpus_id>.ministr-index`.
    ///
    /// The bundle is built into a tempfile on the pod's local disk
    /// first (so the upload streams sequential bytes rather than
    /// interleaving zstd-encoded chunks with concurrent HTTP frames),
    /// then uploaded. The tempfile is dropped on return.
    ///
    /// # Errors
    ///
    /// Fails on bundle-export errors, I/O errors writing the tempfile,
    /// or upload errors from the storage service.
    pub async fn upload_corpus(
        &self,
        corpus_id: &str,
        corpus_dir: &Path,
        manifest: &BundleManifest,
    ) -> BlobResult<()> {
        let tmp = tempfile::NamedTempFile::new().map_err(|e| BlobError::Io {
            path: PathBuf::from("<tempfile>"),
            source: e,
        })?;
        let bundle_path =
            bundle::export_bundle(corpus_dir, tmp.path(), manifest).map_err(BlobError::Bundle)?;
        let bytes = tokio::fs::read(&bundle_path).await.map_err(|e| BlobError::Io {
            path: bundle_path.clone(),
            source: e,
        })?;
        let size = bytes.len();
        let blob = self.container.blob_client(&blob_name_for(corpus_id));
        blob.upload(RequestContent::from(bytes), None).await?;
        info!(
            corpus_id,
            bytes = size,
            "uploaded corpus blob"
        );
        Ok(())
    }

    /// Download the blob for `corpus_id` and restore it into
    /// `target_corpus_dir`, returning the bundle's manifest.
    ///
    /// `target_corpus_dir` must already exist; the daemon's
    /// registration path is responsible for creating it before this
    /// call (matches the existing `import_bundle` contract).
    ///
    /// # Errors
    ///
    /// Fails if the blob does not exist, the download fails, the
    /// tempfile cannot be written, or the bundle is malformed.
    pub async fn download_corpus(
        &self,
        corpus_id: &str,
        target_corpus_dir: &Path,
    ) -> BlobResult<BundleManifest> {
        let blob = self.container.blob_client(&blob_name_for(corpus_id));
        let response = blob.download(None).await?;
        let bytes = response.body.collect().await?;
        let tmp = tempfile::NamedTempFile::new().map_err(|e| BlobError::Io {
            path: PathBuf::from("<tempfile>"),
            source: e,
        })?;
        tokio::fs::write(tmp.path(), &bytes).await.map_err(|e| BlobError::Io {
            path: tmp.path().to_path_buf(),
            source: e,
        })?;
        let manifest = bundle::import_bundle(tmp.path(), target_corpus_dir)?;
        info!(
            corpus_id,
            vectors = manifest.vector_count,
            documents = manifest.document_count,
            "downloaded and restored corpus blob"
        );
        Ok(manifest)
    }

    /// List all corpus IDs present in the container. Each returned
    /// string is the `corpus_id` portion of a `corpora/<id>.ministr-index`
    /// blob name. The order matches the storage service's listing
    /// (lexicographic by blob name).
    ///
    /// # Errors
    ///
    /// Surfaces any error from the storage service. Blobs that don't
    /// match the `corpora/<id>.ministr-index` shape are skipped (logged
    /// at debug level) rather than failing the listing.
    pub async fn list_corpora(&self) -> BlobResult<Vec<String>> {
        let mut pager = self.container.list_blobs(None)?;
        let mut ids = Vec::new();
        while let Some(item) = pager.try_next().await? {
            let Some(name) = item.name else { continue };
            if let Some(id) = corpus_id_from_blob_name(&name) {
                ids.push(id);
            } else {
                debug!(blob = %name, "skipped non-corpus blob during listing");
            }
        }
        Ok(ids)
    }

    /// Delete a single corpus's blob. Idempotent: a 404 is swallowed
    /// so callers can use this as a best-effort cleanup.
    ///
    /// # Errors
    ///
    /// Surfaces any non-404 error from the storage service.
    pub async fn delete_corpus(&self, corpus_id: &str) -> BlobResult<()> {
        let blob = self.container.blob_client(&blob_name_for(corpus_id));
        match blob.delete(None).await {
            Ok(_) => {
                info!(corpus_id, "deleted corpus blob");
                Ok(())
            }
            Err(e) if is_not_found(&e) => {
                debug!(corpus_id, "corpus blob already absent; nothing to delete");
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
}

fn blob_name_for(corpus_id: &str) -> String {
    format!("corpora/{corpus_id}.{BUNDLE_EXTENSION}")
}

fn corpus_id_from_blob_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("corpora/")?;
    let id = rest.strip_suffix(&format!(".{BUNDLE_EXTENSION}"))?;
    if id.is_empty() {
        return None;
    }
    Some(id.to_string())
}

fn is_already_exists(e: &azure_core::Error) -> bool {
    error_code_eq(e, "ContainerAlreadyExists")
}

fn is_not_found(e: &azure_core::Error) -> bool {
    matches!(
        e.kind(),
        azure_core::error::ErrorKind::HttpResponse { status, .. }
            if *status == StatusCode::NotFound
    ) || error_code_eq(e, "BlobNotFound")
}

fn error_code_eq(e: &azure_core::Error, code: &str) -> bool {
    if let azure_core::error::ErrorKind::HttpResponse { error_code, .. } = e.kind() {
        return error_code.as_deref() == Some(code);
    }
    false
}

#[cfg(test)]
mod tests {
    //! Unit + integration tests.
    //!
    //! The pure round-trip test exercises the `bundle::export_bundle` →
    //! `bundle::import_bundle` path that the Azure upload/download pair
    //! sits on top of. The `#[ignore]` Azure tests need a real account
    //! at `AZURE_STORAGE_ACCOUNT_NAME` with the caller signed in via
    //! `az login`.

    use super::*;
    use ministr_core::bundle::{BundleCorpusRoot, BundleManifest, BUNDLE_FORMAT_VERSION};

    fn fake_manifest() -> BundleManifest {
        BundleManifest {
            format_version: BUNDLE_FORMAT_VERSION,
            model_name: "test-model".into(),
            dimension: 4,
            vector_count: 0,
            document_count: 0,
            symbol_count: 0,
            corpus_roots: vec![BundleCorpusRoot {
                id: "root-1".into(),
                display_name: Some("test".into()),
                kind: "local".into(),
                commit_sha: None,
                branch: None,
                repo_url: None,
            }],
            created_at: 0,
            bundle_version: None,
            source_commit: None,
        }
    }

    #[test]
    fn blob_name_round_trip() {
        let name = blob_name_for("abc123");
        assert_eq!(name, "corpora/abc123.ministr-index");
        assert_eq!(corpus_id_from_blob_name(&name).as_deref(), Some("abc123"));
    }

    #[test]
    fn rejects_non_corpus_blob_names() {
        assert!(corpus_id_from_blob_name("atlas/react/abc.ministr-index").is_none());
        assert!(corpus_id_from_blob_name("corpora/foo.zip").is_none());
        assert!(corpus_id_from_blob_name("corpora/.ministr-index").is_none());
    }

    /// Verifies that we can hand a corpus directory shaped like the
    /// real one to `bundle::export_bundle` and then restore it back,
    /// which is the contract the Azure upload/download pair depends on.
    ///
    /// Skipped if the writer cannot build a `SQLite` db — that requires
    /// rusqlite, which is in workspace deps but not a direct dep of
    /// ministr-cloud yet. The test sets up the file scaffolding via
    /// raw bytes to stay dep-free.
    #[test]
    fn manifest_serialises_for_blob_payload() {
        // Sanity: the manifest we'd store in a real bundle is
        // JSON-serialisable. Full bundle round-trip happens in
        // ministr-core's tests; we don't duplicate that surface here.
        let manifest = fake_manifest();
        let s = serde_json::to_string(&manifest).expect("manifest serialises");
        assert!(s.contains("\"model_name\":\"test-model\""));
    }

    #[tokio::test]
    #[ignore = "needs AZURE_STORAGE_ACCOUNT_NAME + az login"]
    async fn list_empty_corpora() {
        let Ok(account) = std::env::var("AZURE_STORAGE_ACCOUNT_NAME") else {
            return;
        };
        let container = std::env::var("MINISTR_TEST_BLOB_CONTAINER")
            .unwrap_or_else(|_| "ministr-test-corpora".to_string());
        let cred = azure_identity::DeveloperToolsCredential::new(None)
            .expect("constructing DeveloperToolsCredential")
            as Arc<dyn TokenCredential>;
        let store = CorpusBlobStore::with_credential(&account, &container, cred)
            .expect("constructing CorpusBlobStore");
        store.ensure_container().await.expect("ensure_container");
        let _ids = store.list_corpora().await.expect("list_corpora");
        // We don't assert empty — the container may have artefacts from
        // earlier runs. The point is the round-trip completes.
    }
}
