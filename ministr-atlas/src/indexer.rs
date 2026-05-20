//! Weekly cron worker entrypoint.
//!
//! `reindex_once()` orchestrates ONE pass over the seed list. Each
//! per-repo step is a separate trait so unit tests can stub them
//! independently (DIP):
//!
//! - [`Cloner::clone_to_tmp`] — git fetch into a working dir
//! - [`IndexerStep::index_dir`] — run the ministr-core indexing
//!   pipeline against the cloned tree
//! - [`BlobWriter::write_blob`] — upload the resulting HNSW + symbol
//!   blob to the Atlas storage account
//!
//! The cron job (Azure Container Apps Job, F4.2) invokes
//! `reindex_once` once per scheduled tick. F2.6 v0 ships the trait
//! surface + a single in-memory test impl; the production impls
//! (real `git clone`, real index pipeline, real Azure Blob upload)
//! land in F4.2 because they require infrastructure provisioning.

use std::pin::Pin;
use std::sync::Arc;

use crate::license::LicenseFilter;
use crate::optout::OptOutRegistry;
use crate::repos::ATLAS_SEED_REPOS;

/// Errors surfaced by the indexer.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ReindexError {
    /// Per-repo step failed. The aggregated outcome counts how many
    /// of these landed; the cron continues past individual failures
    /// so one broken repo doesn't block the other 49.
    #[error("step '{step}' failed for {slug}: {reason}")]
    Step {
        step: &'static str,
        slug: String,
        reason: String,
    },
}

/// Async-trait-style return: boxed `Future` so the trait stays
/// `dyn`-safe and tests can return varying futures per call.
type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Step 1: clone the repo into a working directory the indexer can
/// walk. F4.2 wires this to `ministr_core::git::GitFetcher`.
pub trait Cloner: Send + Sync + std::fmt::Debug {
    /// Clone `clone_url` and return the local path of the clone. The
    /// returned path is the indexer's input.
    fn clone_to_tmp<'a>(
        &'a self,
        clone_url: &'a str,
    ) -> BoxFut<'a, Result<std::path::PathBuf, ReindexError>>;
}

/// Step 2: run the indexer over a cloned directory. F4.2 wires this
/// to the ministr-core ingestion pipeline.
pub trait IndexerStep: Send + Sync + std::fmt::Debug {
    /// Index `path` and return an opaque handle the [`BlobWriter`]
    /// consumes. F2.6 v0 uses a stringly-typed placeholder; F4.2
    /// returns the actual `BundleHandle` from ministr-core.
    fn index_dir<'a>(
        &'a self,
        path: &'a std::path::Path,
    ) -> BoxFut<'a, Result<String, ReindexError>>;
}

/// Step 3: upload the indexed blob to durable storage. F4.2 wires
/// this to `ministr_cloud::CorpusBlobStore`.
pub trait BlobWriter: Send + Sync + std::fmt::Debug {
    /// Persist `bundle_handle` at the Atlas storage location for
    /// `slug`. Returns the resolved blob URL or path.
    fn write_blob<'a>(
        &'a self,
        slug: &'a str,
        bundle_handle: &'a str,
    ) -> BoxFut<'a, Result<String, ReindexError>>;
}

/// Result of one `reindex_once` pass — never `Err` for the call as a
/// whole; individual failures are counted so the cron logs the
/// aggregate.
#[derive(Debug, Clone, Default)]
pub struct ReindexOutcome {
    /// Repos that completed all three steps.
    pub indexed: Vec<String>,
    /// Repos that were skipped (license rejected OR opted out) plus
    /// the reason. Skipped repos are NOT failures.
    pub skipped: Vec<(String, &'static str)>,
    /// Per-step failures. The cron emits these as alerts when the
    /// count breaches the dead-letter threshold (F4.2 — 3 weeks in a
    /// row flags manual review).
    pub failed: Vec<ReindexError>,
}

/// Run one full pass over [`ATLAS_SEED_REPOS`]. The cron invokes
/// this on its schedule; the result drives the dashboard + alerts.
///
/// # Errors
///
/// This function never returns a top-level `Err`; per-repo failures
/// land in [`ReindexOutcome::failed`] so one broken repo doesn't kill
/// the run. The trait collaborators' own errors are caught and
/// recorded.
pub async fn reindex_once(
    cloner: &Arc<dyn Cloner>,
    indexer: &Arc<dyn IndexerStep>,
    writer: &Arc<dyn BlobWriter>,
    license: &Arc<dyn LicenseFilter>,
    optout: &Arc<dyn OptOutRegistry>,
) -> ReindexOutcome {
    let mut outcome = ReindexOutcome::default();
    for repo in ATLAS_SEED_REPOS {
        if !license.admits(repo.spdx) {
            outcome.skipped.push((repo.slug.into(), "license_filter"));
            continue;
        }
        if optout.is_opted_out(repo.clone_url) {
            outcome.skipped.push((repo.slug.into(), "opted_out"));
            continue;
        }
        match cloner.clone_to_tmp(repo.clone_url).await {
            Ok(dir) => match indexer.index_dir(&dir).await {
                Ok(handle) => match writer.write_blob(repo.slug, &handle).await {
                    Ok(_url) => outcome.indexed.push(repo.slug.into()),
                    Err(e) => outcome.failed.push(e),
                },
                Err(e) => outcome.failed.push(e),
            },
            Err(e) => outcome.failed.push(e),
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::license::SpdxFilter;
    use crate::optout::InMemoryRegistry;
    use std::path::{Path, PathBuf};

    #[derive(Debug)]
    struct OkCloner;
    impl Cloner for OkCloner {
        fn clone_to_tmp<'a>(
            &'a self,
            clone_url: &'a str,
        ) -> BoxFut<'a, Result<PathBuf, ReindexError>> {
            Box::pin(async move { Ok(PathBuf::from(format!("/tmp/clone-{}", clone_url.len()))) })
        }
    }

    #[derive(Debug)]
    struct OkIndexer;
    impl IndexerStep for OkIndexer {
        fn index_dir<'a>(
            &'a self,
            _path: &'a Path,
        ) -> BoxFut<'a, Result<String, ReindexError>> {
            Box::pin(async move { Ok("bundle-handle".into()) })
        }
    }

    #[derive(Debug)]
    struct OkWriter;
    impl BlobWriter for OkWriter {
        fn write_blob<'a>(
            &'a self,
            slug: &'a str,
            _handle: &'a str,
        ) -> BoxFut<'a, Result<String, ReindexError>> {
            Box::pin(async move { Ok(format!("atlas/{slug}/latest.idx")) })
        }
    }

    #[derive(Debug)]
    struct AlwaysFailIndexer;
    impl IndexerStep for AlwaysFailIndexer {
        fn index_dir<'a>(
            &'a self,
            _path: &'a Path,
        ) -> BoxFut<'a, Result<String, ReindexError>> {
            Box::pin(async {
                Err(ReindexError::Step {
                    step: "index_dir",
                    slug: "test".into(),
                    reason: "synthetic failure".into(),
                })
            })
        }
    }

    #[tokio::test]
    async fn full_pass_indexes_every_admitted_repo() {
        let cloner: Arc<dyn Cloner> = Arc::new(OkCloner);
        let indexer: Arc<dyn IndexerStep> = Arc::new(OkIndexer);
        let writer: Arc<dyn BlobWriter> = Arc::new(OkWriter);
        let license: Arc<dyn LicenseFilter> = Arc::new(SpdxFilter);
        let optout: Arc<dyn OptOutRegistry> = Arc::new(InMemoryRegistry::new());

        let outcome = reindex_once(&cloner, &indexer, &writer, &license, &optout).await;
        // Grafana is AGPL — gets skipped on the v0 permissive filter.
        // Everything else admits.
        assert!(outcome.indexed.len() >= 49);
        assert!(
            outcome
                .skipped
                .iter()
                .any(|(slug, reason)| slug == "grafana" && *reason == "license_filter"),
            "grafana should be license-filtered on the v0 pilot"
        );
        assert!(outcome.failed.is_empty(), "no step failures expected");
    }

    #[tokio::test]
    async fn opt_out_skips_repos() {
        let cloner: Arc<dyn Cloner> = Arc::new(OkCloner);
        let indexer: Arc<dyn IndexerStep> = Arc::new(OkIndexer);
        let writer: Arc<dyn BlobWriter> = Arc::new(OkWriter);
        let license: Arc<dyn LicenseFilter> = Arc::new(SpdxFilter);
        let optout: Arc<dyn OptOutRegistry> = Arc::new(InMemoryRegistry::from_urls(["https://github.com/facebook/react"]));

        let outcome = reindex_once(&cloner, &indexer, &writer, &license, &optout).await;
        assert!(
            outcome
                .skipped
                .iter()
                .any(|(slug, reason)| slug == "react" && *reason == "opted_out"),
            "react should be opted_out-skipped"
        );
        assert!(!outcome.indexed.contains(&"react".to_string()));
    }

    #[tokio::test]
    async fn indexer_step_failures_are_recorded_not_fatal() {
        let cloner: Arc<dyn Cloner> = Arc::new(OkCloner);
        let indexer: Arc<dyn IndexerStep> = Arc::new(AlwaysFailIndexer);
        let writer: Arc<dyn BlobWriter> = Arc::new(OkWriter);
        let license: Arc<dyn LicenseFilter> = Arc::new(SpdxFilter);
        let optout: Arc<dyn OptOutRegistry> = Arc::new(InMemoryRegistry::new());

        let outcome = reindex_once(&cloner, &indexer, &writer, &license, &optout).await;
        // Every non-skipped repo fails on the synthetic indexer.
        assert!(outcome.failed.len() >= 49);
        assert!(outcome.indexed.is_empty());
    }
}
