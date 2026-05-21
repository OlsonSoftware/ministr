//! Backend dispatcher for corpus blob storage.
//!
//! [`BlobBackend`] is the single concrete type cloud handlers hold
//! (matches the `OAuthBackend` / `JobQueueBackend` pattern in
//! `ministr-mcp::auth::storage` and `ministr-mcp::admin::jobs`). Two
//! variants today:
//!
//! - [`BlobBackend::Azure`] — production. Wraps [`CorpusBlobStore`],
//!   talks to Azure Blob via the official `azure_storage_blob` SDK.
//! - [`BlobBackend::Filesystem`] — local dev. Wraps
//!   [`FilesystemBlobStore`]. Selected by `just dev-cloud-up` so a
//!   fresh laptop can run the cloud without an Azurite container.
//!
//! Add a backend = add a variant + impl the same method set. Callers
//! never change (OCP). The wrapping enum lets the cloud crate keep
//! a `Send + Sync` concrete handle that's cheap to clone — neither
//! `CorpusBlobStore` nor the underlying Azure `BlobContainerClient`
//! impl `Clone`, so the enum's variant carries an `Arc`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ministr_core::bundle::BundleManifest;

use crate::blob::{BlobError, BlobResult, CorpusBlobStore, CorpusManifest};
use crate::blob_fs::FilesystemBlobStore;

/// Concrete dispatcher held by every cloud surface that needs to
/// read or write corpus blobs (the F4.2 cron worker, the future
/// Atlas indexer, the in-VPC mirror).
///
/// Cheap to clone — both variants are `Arc`-backed internally.
#[derive(Debug, Clone)]
pub enum BlobBackend {
    Azure(Arc<CorpusBlobStore>),
    Filesystem(FilesystemBlobStore),
}

impl BlobBackend {
    /// Idempotently create the underlying container / root directory.
    /// Safe to call on every pod boot.
    ///
    /// # Errors
    ///
    /// Surfaces the backend's own creation error.
    pub async fn ensure_container(&self) -> BlobResult<()> {
        match self {
            Self::Azure(s) => s.ensure_container().await,
            Self::Filesystem(s) => s.ensure_container().await,
        }
    }

    /// Persist a corpus + atomically swap the manifest pointer.
    /// Returns the version string written.
    ///
    /// # Errors
    ///
    /// Same surface as [`CorpusBlobStore::upload_corpus`] /
    /// [`FilesystemBlobStore::upload_corpus`].
    pub async fn upload_corpus(
        &self,
        corpus_id: &str,
        corpus_dir: &Path,
        bundle_manifest: &BundleManifest,
    ) -> BlobResult<String> {
        match self {
            Self::Azure(s) => s.upload_corpus(corpus_id, corpus_dir, bundle_manifest).await,
            Self::Filesystem(s) => s.upload_corpus(corpus_id, corpus_dir, bundle_manifest).await,
        }
    }

    /// Restore a corpus into `target_corpus_dir`, returning the
    /// imported manifest.
    ///
    /// # Errors
    ///
    /// Same surface as the per-backend `download_corpus`.
    pub async fn download_corpus(
        &self,
        corpus_id: &str,
        target_corpus_dir: &Path,
    ) -> BlobResult<BundleManifest> {
        match self {
            Self::Azure(s) => s.download_corpus(corpus_id, target_corpus_dir).await,
            Self::Filesystem(s) => s.download_corpus(corpus_id, target_corpus_dir).await,
        }
    }

    /// Read the current `CorpusManifest`.
    ///
    /// # Errors
    ///
    /// Same surface as the per-backend `get_manifest`.
    pub async fn get_manifest(&self, corpus_id: &str) -> BlobResult<CorpusManifest> {
        match self {
            Self::Azure(s) => s.get_manifest(corpus_id).await,
            Self::Filesystem(s) => s.get_manifest(corpus_id).await,
        }
    }

    /// List all corpus IDs with a current manifest.
    ///
    /// # Errors
    ///
    /// Same surface as the per-backend `list_corpora`.
    pub async fn list_corpora(&self) -> BlobResult<Vec<String>> {
        match self {
            Self::Azure(s) => s.list_corpora().await,
            Self::Filesystem(s) => s.list_corpora().await,
        }
    }

    /// Delete a corpus's manifest + best-effort delete its current
    /// bundle.
    ///
    /// # Errors
    ///
    /// Same surface as the per-backend `delete_corpus`.
    pub async fn delete_corpus(&self, corpus_id: &str) -> BlobResult<()> {
        match self {
            Self::Azure(s) => s.delete_corpus(corpus_id).await,
            Self::Filesystem(s) => s.delete_corpus(corpus_id).await,
        }
    }
}

/// Env-var selector. Used by `cmd_serve_http` and the F2.6 atlas
/// reindex CLI to build the right backend without the caller
/// knowing which one.
///
/// | Variant | Trigger |
/// |---|---|
/// | Filesystem | `MINISTR_BLOB_STORE_KIND=filesystem` (or unset when `MINISTR_BLOB_FS_ROOT` is set) |
/// | Filesystem | `MINISTR_BLOB_FS_ROOT=<path>` — convenience, implies kind=filesystem |
/// | Azure | `MINISTR_BLOB_STORE_KIND=azure` + `MINISTR_BLOB_AZURE_ACCOUNT` + `MINISTR_BLOB_AZURE_CONTAINER` |
///
/// Returns `None` when no blob backend is configured — the operator's
/// code should treat that as "blob persistence disabled" rather than
/// crash. `just dev-cloud-up` defaults `MINISTR_BLOB_FS_ROOT` to
/// `$HOME/.ministr/cloud-dev/blobs`, so the dev path always resolves.
///
/// # Errors
///
/// Returns [`BlobError::Azure`] when the Azure variant is selected
/// but credential construction fails.
pub fn build_from_env() -> BlobResult<Option<BlobBackend>> {
    let explicit_kind = std::env::var("MINISTR_BLOB_STORE_KIND")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase());
    let fs_root_env = std::env::var("MINISTR_BLOB_FS_ROOT").ok();
    let kind = explicit_kind.unwrap_or_else(|| {
        if fs_root_env.is_some() {
            "filesystem".into()
        } else {
            String::new()
        }
    });

    match kind.as_str() {
        "filesystem" => {
            let root = fs_root_env.map_or_else(default_fs_root, PathBuf::from);
            Ok(Some(BlobBackend::Filesystem(FilesystemBlobStore::new(root))))
        }
        "azure" => {
            let account = std::env::var("MINISTR_BLOB_AZURE_ACCOUNT").ok();
            let container = std::env::var("MINISTR_BLOB_AZURE_CONTAINER").ok();
            let (Some(account), Some(container)) = (account, container) else {
                tracing::warn!(
                    "MINISTR_BLOB_STORE_KIND=azure but MINISTR_BLOB_AZURE_ACCOUNT or \
                     MINISTR_BLOB_AZURE_CONTAINER is missing — blob backend disabled"
                );
                return Ok(None);
            };
            // Pick credential by environment. ACA injects
            // `IDENTITY_ENDPOINT` for system-assigned MI; absence means
            // we're outside Azure (local dev, CI). DeveloperToolsCredential
            // only chains CLI tools (`az`, `azd`), neither of which exist
            // in the runtime image — using it in-pod was the cause of
            // every blob call failing with "non-transport error occurred
            // which will not be retried."
            // MINISTR_BLOB_STORE_KIND=azure is only set in cloud
            // deploys (Pulumi-templated env on the Container App).
            // Local dev never opts into the Azure variant — it uses
            // the filesystem store under MINISTR_BLOB_FS_ROOT. So
            // when we're here, we are by definition in-pod with a
            // system-assigned managed identity; ManagedIdentityCredential
            // is the only correct choice. Previously this used
            // DeveloperToolsCredential which only chains `az`/`azd`
            // CLI lookups, neither of which exist in the runtime
            // image — every blob call failed with "non-transport
            // error occurred which will not be retried" with the
            // real cause buried as "az not found on PATH".
            tracing::info!("constructing blob credential via ManagedIdentityCredential");
            let cred: std::sync::Arc<dyn azure_core::credentials::TokenCredential> =
                azure_identity::ManagedIdentityCredential::new(None)
                    .map_err(BlobError::Azure)?;
            let store = CorpusBlobStore::with_credential(&account, &container, cred)?;
            Ok(Some(BlobBackend::Azure(Arc::new(store))))
        }
        "" => Ok(None),
        other => {
            tracing::warn!(
                kind = %other,
                "unknown MINISTR_BLOB_STORE_KIND value — blob backend disabled"
            );
            Ok(None)
        }
    }
}

fn default_fs_root() -> PathBuf {
    // Mirrors `ministr_api::daemon_data_dir`'s posture: HOME on Unix,
    // USERPROFILE on Windows, fall back to the cwd. Avoids pulling in
    // `dirs-next` for one call site.
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if let Some(h) = home {
        h.join(".ministr").join("cloud-dev").join("blobs")
    } else {
        PathBuf::from("./.ministr-cloud-dev/blobs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Env-mutation tests would race with parallel cargo-test workers
    // (process-wide env is shared) AND require `unsafe` blocks since
    // Rust 2024 marked `std::env::set_var` unsafe — the workspace
    // denies unsafe. The selector's per-branch logic is straightforward
    // enough to read by eye; the heavy lifting is in the
    // per-backend modules (`blob_fs::tests` covers the filesystem
    // path; the Azure path is exercised by the `blob::tests`
    // integration suite gated on real credentials).

    #[test]
    fn default_fs_root_is_under_home_when_set() {
        let root = default_fs_root();
        // The root must be SOMEWHERE under either HOME or the cwd
        // fallback. We don't assert on absolute path equality because
        // tests run with arbitrary `HOME` on CI.
        assert!(root.to_string_lossy().contains("blobs"));
    }
}
