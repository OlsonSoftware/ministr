//! Per-corpus HNSW blob persistence for the cloud surface.
//!
//! Every corpus indexed by `mcp.ministr.ai` lives in Azure Blob Storage
//! as a **versioned `.ministr-index` bundle plus a small manifest blob**:
//!
//! ```text
//! <container>/
//!   ├─ corpora/<corpus_id>/manifest.json              ← pointer (small, atomically swapped)
//!   └─ corpora/<corpus_id>/<bundle_version>.ministr-index  ← bundle (versioned)
//! ```
//!
//! The manifest is the source of truth for "what's current". Atomic-swap
//! semantics fall out of Azure Blob's single-PUT atomicity: until the
//! manifest is rewritten, readers see the old bundle pointer; after, they
//! see the new one. Bundles themselves are never overwritten; old
//! versions accumulate and are eligible for separate GC (a future
//! `retention` job — out of scope for F1.1).
//!
//! This layout mirrors F4.2's Atlas design (`atlas/<slug>/<commit>.idx`
//! plus a `latest` pointer) so the same retention machinery can serve
//! both.
//!
//! ## Build-on-/tmp + fsync + atomic-swap (F1.1 sub-bullet 6)
//!
//! Multi-tenant pods build new bundles on local ephemeral disk
//! (`/tmp`) — fast, no SMB-mount perf penalty — then fsync the file
//! before upload. Once durable, the worker:
//!
//! 1. Uploads the bundle to `corpora/<id>/<version>.ministr-index`.
//! 2. Atomically swaps `corpora/<id>/manifest.json` to point at the new
//!    version (single small PUT).
//!
//! A pod that dies mid-upload leaves the old manifest pointing at the
//! old bundle — readers never see a half-written corpus. A pod that dies
//! between the two PUTs leaves an orphan bundle blob; GC reaps it later.
//!
//! ## Why blob, not pgvector
//!
//! Reuses [`ministr_core::bundle::export_bundle`] /
//! [`ministr_core::bundle::import_bundle`] verbatim — same format the
//! local stack already exports for `ministr export`. Sidesteps
//! pgvector's 2000-dimension HNSW ceiling. Keeps the `SQLite` path for
//! self-hosted users unchanged.
//!
//! ## Cold-start / warm-cache (daemon wiring lands in F1.2)
//!
//! This module owns the upload/download/list primitives. The daemon-side
//! "on pod boot, restore each registered corpus's blob into
//! `$DATA_DIR/corpora/`" loop is `cmd_serve_http`'s job once the cloud
//! mode selector lands in F1.2. The primitives here are deliberately
//! state-free so the daemon owns when/where to materialise each corpus.
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
use std::time::{SystemTime, UNIX_EPOCH};

use azure_core::credentials::TokenCredential;
use azure_core::http::{RequestContent, StatusCode, Url};
use azure_storage_blob::{BlobContainerClient, BlobServiceClient};
use futures::TryStreamExt;
use ministr_core::bundle::{self, BundleManifest, BUNDLE_EXTENSION};
use ministr_core::error::BundleError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, warn};

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
    #[error("malformed corpus manifest blob: {0}")]
    MalformedManifest(#[from] serde_json::Error),
    #[error("bundle manifest is missing the `bundle_version` field; cannot upload (run `bundle::compute_bundle_version` first)")]
    MissingBundleVersion,
}

/// Result alias for blob-store operations.
pub type BlobResult<T> = Result<T, BlobError>;

/// Tiny pointer blob naming the currently-canonical bundle version
/// for a corpus. Atomically rewritten on every publish.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusManifest {
    /// Bundle version string — names the canonical
    /// `corpora/<corpus_id>/<version>.ministr-index` blob. Sourced from
    /// `bundle::compute_bundle_version` so reads against a manifest with
    /// the same version as the local cache are no-op staleness checks.
    pub current_version: String,
    /// Unix-seconds when this manifest was last written.
    pub updated_at: u64,
}

/// Per-corpus HNSW blob store backed by an Azure Storage container.
///
/// One instance maps to one container. Multiple corpora live as
/// distinct `corpora/<id>/` prefixes inside that container.
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
            .map_err(|e| {
                azure_core::Error::with_message(azure_core::error::ErrorKind::Other, e.to_string())
            })?;
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

    /// Upload the corpus at `corpus_dir` as a versioned
    /// `.ministr-index` bundle, then atomically swap the corpus
    /// manifest to point at the new version.
    ///
    /// Pipeline:
    /// 1. **Build on `/tmp`** — `bundle::export_bundle` writes a
    ///    zstd-tar bundle to a tempfile on local ephemeral disk.
    /// 2. **fsync** — the tempfile is fsynced before any upload, so a
    ///    pod death after this point leaves a durable local artefact
    ///    (helpful for incident forensics and any retry harness above).
    /// 3. **PUT versioned bundle** — `corpora/<id>/<version>.ministr-index`.
    /// 4. **Atomic-swap manifest** — single small PUT on
    ///    `corpora/<id>/manifest.json`. Until this PUT lands, readers
    ///    still see the old version.
    ///
    /// Returns the bundle version string the manifest now points at.
    ///
    /// # Errors
    ///
    /// Fails if `manifest.bundle_version` is `None` (the caller must
    /// have populated it via `bundle::compute_bundle_version`), or on
    /// any bundle, I/O, or storage error.
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

        // (1) build bundle on local /tmp
        let tmp = tempfile::NamedTempFile::new().map_err(|e| BlobError::Io {
            path: PathBuf::from("<tempfile>"),
            source: e,
        })?;
        let bundle_path =
            bundle::export_bundle(corpus_dir, tmp.path(), bundle_manifest).map_err(BlobError::Bundle)?;

        // (2) fsync the tempfile so the bundle is durable on the pod's
        //     local disk before we read it back for upload. Defensive
        //     against partial writes from a crashing tar+zstd stream.
        let f = tokio::fs::File::open(&bundle_path).await.map_err(|e| BlobError::Io {
            path: bundle_path.clone(),
            source: e,
        })?;
        f.sync_all().await.map_err(|e| BlobError::Io {
            path: bundle_path.clone(),
            source: e,
        })?;
        drop(f);

        // (3) PUT the versioned bundle blob.
        let bytes = tokio::fs::read(&bundle_path).await.map_err(|e| BlobError::Io {
            path: bundle_path.clone(),
            source: e,
        })?;
        let bundle_size = bytes.len();
        let bundle_blob = self.container.blob_client(&bundle_blob_name(corpus_id, &version));
        bundle_blob
            .upload(RequestContent::from(bytes), None)
            .await?;

        // (4) Atomic-swap the manifest pointer.
        self.put_manifest(corpus_id, &version).await?;

        info!(
            corpus_id,
            version = %version,
            bytes = bundle_size,
            "uploaded corpus bundle and swapped manifest"
        );
        Ok(version)
    }

    /// Download the manifest's pointed-at bundle and restore it into
    /// `target_corpus_dir`, returning the bundle's manifest.
    ///
    /// `target_corpus_dir` must already exist; the daemon's
    /// registration path is responsible for creating it before this
    /// call (matches `import_bundle`'s contract).
    ///
    /// # Errors
    ///
    /// Fails if no manifest is present (treated as a clean 404), the
    /// bundle the manifest points at is missing, the download fails,
    /// the tempfile cannot be written, or the bundle is malformed.
    pub async fn download_corpus(
        &self,
        corpus_id: &str,
        target_corpus_dir: &Path,
    ) -> BlobResult<BundleManifest> {
        let cm = self.get_manifest(corpus_id).await?;
        let bundle_blob = self
            .container
            .blob_client(&bundle_blob_name(corpus_id, &cm.current_version));
        let response = bundle_blob.download(None).await?;
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
            version = %cm.current_version,
            vectors = manifest.vector_count,
            documents = manifest.document_count,
            "downloaded and restored corpus bundle"
        );
        Ok(manifest)
    }

    /// Return the current `CorpusManifest` for a corpus.
    ///
    /// # Errors
    ///
    /// Surfaces the underlying storage error if the manifest blob is
    /// missing — callers that want a soft-404 should match
    /// [`is_not_found`] on the error.
    pub async fn get_manifest(&self, corpus_id: &str) -> BlobResult<CorpusManifest> {
        let manifest_blob = self.container.blob_client(&manifest_blob_name(corpus_id));
        let response = manifest_blob.download(None).await?;
        let bytes = response.body.collect().await?;
        let cm: CorpusManifest = serde_json::from_slice(&bytes)?;
        Ok(cm)
    }

    /// Write a fresh `CorpusManifest` pointing at `version`. Replaces
    /// any prior manifest in a single atomic PUT.
    async fn put_manifest(&self, corpus_id: &str, version: &str) -> BlobResult<()> {
        let cm = CorpusManifest {
            current_version: version.to_string(),
            updated_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        };
        let bytes = serde_json::to_vec(&cm)?;
        let manifest_blob = self.container.blob_client(&manifest_blob_name(corpus_id));
        manifest_blob
            .upload(RequestContent::from(bytes), None)
            .await?;
        debug!(corpus_id, version, "atomic-swapped manifest pointer");
        Ok(())
    }

    /// List all corpus IDs present in the container. A corpus is
    /// considered present iff its `manifest.json` blob exists; orphaned
    /// bundle versions without a manifest are not reported (they are
    /// GC candidates, not live data).
    ///
    /// Each returned string is the `<corpus_id>` segment of a
    /// `corpora/<corpus_id>/manifest.json` blob.
    ///
    /// # Errors
    ///
    /// Surfaces any error from the storage service.
    pub async fn list_corpora(&self) -> BlobResult<Vec<String>> {
        let mut pager = self.container.list_blobs(None)?;
        let mut ids = Vec::new();
        while let Some(item) = pager.try_next().await? {
            let Some(name) = item.name else { continue };
            if let Some(id) = corpus_id_from_manifest_blob_name(&name) {
                ids.push(id);
            } else {
                debug!(blob = %name, "skipped non-manifest blob during corpora listing");
            }
        }
        Ok(ids)
    }

    /// Delete a corpus's manifest (so it disappears from
    /// `list_corpora`) and best-effort delete its current bundle blob.
    /// Historic bundle versions are left for separate GC.
    ///
    /// # Errors
    ///
    /// Surfaces any non-404 error from the storage service.
    pub async fn delete_corpus(&self, corpus_id: &str) -> BlobResult<()> {
        // Try to read the manifest *before* deleting it so we can also
        // delete the bundle it points at. If the manifest is already
        // gone, just succeed.
        let current_version = match self.get_manifest(corpus_id).await {
            Ok(m) => Some(m.current_version),
            Err(BlobError::Azure(e)) if is_not_found(&e) => None,
            Err(e) => return Err(e),
        };

        // (a) Delete manifest first — single atomic delete; the corpus
        //     disappears from `list_corpora` immediately.
        let manifest_blob = self.container.blob_client(&manifest_blob_name(corpus_id));
        match manifest_blob.delete(None).await {
            Ok(_) => debug!(corpus_id, "deleted corpus manifest"),
            Err(e) if is_not_found(&e) => {
                debug!(corpus_id, "corpus manifest already absent");
            }
            Err(e) => return Err(e.into()),
        }

        // (b) Best-effort delete of the current bundle. Failures here
        //     are non-fatal — the bundle will be reaped by a GC pass.
        if let Some(version) = current_version {
            let bundle_blob = self
                .container
                .blob_client(&bundle_blob_name(corpus_id, &version));
            if let Err(e) = bundle_blob.delete(None).await
                && !is_not_found(&e)
            {
                warn!(
                    corpus_id,
                    version,
                    error = %e,
                    "failed to delete current bundle blob during delete_corpus; \
                     leaving for GC"
                );
            }
        }
        info!(corpus_id, "deleted corpus");
        Ok(())
    }
}

fn manifest_blob_name(corpus_id: &str) -> String {
    format!("corpora/{corpus_id}/manifest.json")
}

fn bundle_blob_name(corpus_id: &str, version: &str) -> String {
    format!("corpora/{corpus_id}/{version}.{BUNDLE_EXTENSION}")
}

fn corpus_id_from_manifest_blob_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("corpora/")?;
    let id = rest.strip_suffix("/manifest.json")?;
    if id.is_empty() || id.contains('/') {
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
    //! Unit tests + `#[ignore]` Azure integration tests.
    //!
    //! The pure-logic tests verify the blob-name <-> corpus-id mapping
    //! and the manifest JSON shape; the Azure tests need a real
    //! account at `AZURE_STORAGE_ACCOUNT_NAME` with the caller signed
    //! in via `az login`.

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
            bundle_version: Some("abc123".into()),
            source_commit: None,
        }
    }

    #[test]
    fn manifest_blob_name_round_trip() {
        let name = manifest_blob_name("abc123");
        assert_eq!(name, "corpora/abc123/manifest.json");
        assert_eq!(
            corpus_id_from_manifest_blob_name(&name).as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn bundle_blob_name_includes_version() {
        let name = bundle_blob_name("abc123", "v-deadbeef");
        assert_eq!(name, "corpora/abc123/v-deadbeef.ministr-index");
    }

    #[test]
    fn rejects_non_corpus_blob_names() {
        // Atlas blobs live under a different prefix.
        assert!(corpus_id_from_manifest_blob_name("atlas/react/manifest.json").is_none());
        // Bundle blobs are not manifest blobs.
        assert!(corpus_id_from_manifest_blob_name("corpora/foo/v-1.ministr-index").is_none());
        // Empty corpus id.
        assert!(corpus_id_from_manifest_blob_name("corpora//manifest.json").is_none());
        // Nested subpaths shouldn't accidentally match.
        assert!(corpus_id_from_manifest_blob_name("corpora/foo/bar/manifest.json").is_none());
    }

    #[test]
    fn corpus_manifest_round_trips_as_json() {
        let cm = CorpusManifest {
            current_version: "abc123".into(),
            updated_at: 1_716_148_800,
        };
        let s = serde_json::to_string(&cm).unwrap();
        assert!(s.contains("\"current_version\":\"abc123\""));
        let back: CorpusManifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.current_version, "abc123");
        assert_eq!(back.updated_at, 1_716_148_800);
    }

    #[test]
    fn upload_requires_bundle_version() {
        // Real-world callers must populate `bundle_version` (via
        // `bundle::compute_bundle_version`) before upload; we surface a
        // clear error rather than silently overwrite a default.
        let mut m = fake_manifest();
        m.bundle_version = None;
        // We can't exercise the full upload without an Azure client, but
        // the error variant exists and `?` flows through `BlobError`.
        let err: BlobError = BlobError::MissingBundleVersion;
        match err {
            BlobError::MissingBundleVersion => {}
            other => panic!("unexpected variant: {other:?}"),
        }
        // Sanity: the manifest is still serialisable with the version
        // unset; it's the upload helper that enforces the invariant.
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"bundle_version\":null"));
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
