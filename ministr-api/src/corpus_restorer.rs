//! on-demand blob restore hook.
//!
//! [`CorpusRestorer`] is the trait the daemon's [`CorpusRegistry::get`]
//! consults when the in-memory map misses. Cloud deployments wire
//! `ministr_cloud::corpus_restorer::BlobCorpusRestorer`, which calls
//! [`BlobBackend::download_corpus`] to fetch `corpora/<id>/<ver>.ministr-index`
//! into `<data_dir>/corpora/<id>/`. Self-hosted serve leaves the slot
//! `None` and a missing in-memory entry produces the usual `NotFound`.
//!
//! Replaces the boot-time bulk download (which downloaded every
//! corpus's bundle whether queried or not). On-demand is right-sized:
//! a cold pod only pays the download cost for corpora the session
//! actually touches.
//!
//! [`CorpusRegistry::get`]: ../../../ministr_daemon/registry/struct.CorpusRegistry.html#method.get
//! [`BlobBackend::download_corpus`]: ../../../ministr_cloud/blob_backend/struct.BlobBackend.html#method.download_corpus

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

/// Errors surfaced by [`CorpusRestorer`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum CorpusRestoreError {
    /// The corpus has no bundle in blob storage. The caller should
    /// propagate this as `NotFound` to the HTTP client.
    #[error("no bundle for {corpus_id} in blob")]
    NotFound { corpus_id: String },
    /// Underlying storage / IO failure.
    #[error("restore: {0}")]
    Backend(String),
}

/// Boxed future returned by every [`CorpusRestorer`] method. Lifetime
/// ties the future to the borrow of `&self` and the borrowed args.
pub type RestoreFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, CorpusRestoreError>> + Send + 'a>>;

/// Download a corpus bundle on demand.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn CorpusRestorer>`. The trait is `dyn`-safe via boxed
/// futures.
pub trait CorpusRestorer: Send + Sync + std::fmt::Debug {
    /// Download the corpus identified by `corpus_id` into
    /// `target_corpus_dir`. After this returns `Ok`, the directory
    /// is expected to contain `content.db` and `index/`, ready for
    /// the registry to open. Returns [`CorpusRestoreError::NotFound`]
    /// when the bundle is missing — distinct from a storage failure.
    fn download<'a>(
        &'a self,
        corpus_id: &'a str,
        target_corpus_dir: &'a Path,
    ) -> RestoreFuture<'a, ()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct MockRestorer {
        calls: Mutex<Vec<(String, PathBuf)>>,
    }

    impl CorpusRestorer for MockRestorer {
        fn download<'a>(
            &'a self,
            corpus_id: &'a str,
            target_corpus_dir: &'a Path,
        ) -> RestoreFuture<'a, ()> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push((corpus_id.to_string(), target_corpus_dir.to_path_buf()));
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn trait_is_dyn_compatible() {
        let r: std::sync::Arc<dyn CorpusRestorer> = std::sync::Arc::new(MockRestorer::default());
        r.download("c1", Path::new("/tmp/c1")).await.unwrap();
    }
}
