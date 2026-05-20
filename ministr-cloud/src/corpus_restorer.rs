//! `CorpusRestorer` impl backed by Azure Blob Storage.
//!
//! The serve pod's `CorpusRegistry::ensure_present` consults this when
//! a query references a `corpus_id` that isn't in the in-memory map
//! AND a `cloud_corpora` row exists. Calling `download` restores the
//! versioned `.ministr-index` bundle (manifest pointer → blob → tar+zstd
//! extract) into `<data_dir>/corpora/<id>/`, ready for `register_restored`.

use std::path::Path;
use std::sync::Arc;

use ministr_api::corpus_restorer::{CorpusRestoreError, CorpusRestorer, RestoreFuture};

use crate::blob_backend::BlobBackend;

/// Cloud `CorpusRestorer` that streams a bundle out of the `ministr-corpora`
/// container. Cheap to clone — wraps `Arc<BlobBackend>`.
#[derive(Debug, Clone)]
pub struct BlobCorpusRestorer {
    backend: Arc<BlobBackend>,
}

impl BlobCorpusRestorer {
    #[must_use]
    pub fn new(backend: Arc<BlobBackend>) -> Self {
        Self { backend }
    }
}

impl CorpusRestorer for BlobCorpusRestorer {
    fn download<'a>(
        &'a self,
        corpus_id: &'a str,
        target_corpus_dir: &'a Path,
    ) -> RestoreFuture<'a, ()> {
        Box::pin(async move {
            // Best-effort mkdir — download_corpus expects the dir to
            // exist; the registry's caller already does this too, but
            // a concurrent unregister might have removed it.
            if let Err(e) = tokio::fs::create_dir_all(target_corpus_dir).await {
                return Err(CorpusRestoreError::Backend(format!(
                    "mkdir {}: {e}",
                    target_corpus_dir.display()
                )));
            }
            match self.backend.download_corpus(corpus_id, target_corpus_dir).await {
                Ok(_) => Ok(()),
                // `BlobError` does not yet expose a typed NotFound
                // discriminator (the private `is_not_found` helper
                // in blob.rs detects HTTP 404 / `BlobNotFound`). For
                // chunk 5 a string-shape probe is sufficient — caller
                // treats either variant as "no bundle yet" and the
                // serve pod returns NotFound to the HTTP client. A
                // future refinement promotes the helper to public.
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("BlobNotFound") || msg.contains("404") {
                        Err(CorpusRestoreError::NotFound {
                            corpus_id: corpus_id.to_string(),
                        })
                    } else {
                        Err(CorpusRestoreError::Backend(msg))
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time proof — if the impl drops `dyn`-safety the line below
    /// fails to type-check.
    #[allow(dead_code)]
    fn assert_dyn_safe(restorer: BlobCorpusRestorer) {
        let _: Arc<dyn CorpusRestorer> = Arc::new(restorer);
    }
}
