//! serve-side enqueue hook for the indexer worker.
//!
//! When the cloud daemon receives `POST /api/v1/corpora` (or the
//! clone-url route), it no longer runs `indexer::run` inline. Instead
//! it routes through [`IndexJobSink::create_pending`], which performs
//! both the `cloud_corpora` upsert and the `indexer_jobs` enqueue in
//! one call. The worker drains the queue and uploads the
//! bundle; the serve pod's [`IndexJobSink::latest_for_corpus`] reads
//! `JobProgress` from Postgres to power the per-corpus SSE.
//!
//! Self-hosted serve leaves the field `None` on `AppState` and keeps
//! the existing inline-register / inline-clone path — fine because
//! the user's disk is durable and there's no separate worker.
//!
//! # Why a new trait instead of re-exporting `JobQueue`
//!
//! `ministr-mcp::admin::jobs::JobQueue` lives in the MIT crate but the
//! daemon (`ministr-daemon`) does not depend on `ministr-mcp` (the
//! reverse arrow is established: `ministr-mcp` depends on
//! `ministr-daemon`). Putting the consumer trait in `ministr-api`
//! keeps the daemon's dep tree minimal and matches the existing
//! `BlobSink` / `UsageSink` convention.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

/// Lifecycle status of a queued indexing job. Mirrors the
/// `ministr-mcp::admin::jobs::JobStatus` enum on the wire (`snake_case`
/// serde) so callers can `Deserialize` either through this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexJobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// Snapshot of a queued indexing job — surface the daemon's progress
/// SSE reads from Postgres in cloud mode.
///
/// added the `sections_done` / `embeddings_*` fields so
/// the streaming consumer's per-batch progress reaches the SSE
/// instead of being clipped at the wire boundary. `serde(default)`
/// keeps the snapshot deserialisable from -era data blobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexJobSnapshot {
    pub job_id: String,
    pub corpus_id: String,
    pub status: IndexJobStatus,
    pub stage: String,
    pub total_files: u64,
    pub processed_files: u64,
    pub current_file: Option<String>,
    pub error: Option<String>,
    /// see [`JobProgress::sections_done`].
    #[serde(default)]
    pub sections_done: u64,
    /// see [`JobProgress::embeddings_total`].
    #[serde(default)]
    pub embeddings_total: u64,
    /// see [`JobProgress::embeddings_done`]. The
    /// primary signal SSE consumers render as the live progress bar.
    #[serde(default)]
    pub embeddings_done: u64,
}

/// Errors surfaced by [`IndexJobSink`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum IndexJobError {
    #[error("storage: {0}")]
    Storage(String),
}

/// Boxed future returned by every [`IndexJobSink`] method. Lifetime
/// ties the future to the borrow of `&self` (and any borrowed args)
/// so impls can capture references.
pub type IndexJobFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, IndexJobError>> + Send + 'a>>;

/// Cloud-mode enqueue hook. The daemon's `POST /api/v1/corpora` and
/// clone handler call this when wired; otherwise they fall back to
/// the inline-register path.
pub trait IndexJobSink: Send + Sync + std::fmt::Debug {
    /// Create (or refresh) a pending corpus registration and enqueue
    /// an indexing job for it. Idempotent on `corpus_id`: a second
    /// call with the same id may either return the existing `job_id`
    /// or enqueue a fresh re-index — implementations decide. Returns
    /// the `job_id` so the caller can correlate progress.
    fn create_pending<'a>(
        &'a self,
        corpus_id: &'a str,
        paths: &'a [String],
        display_name: Option<&'a str>,
        clone_url: Option<&'a str>,
    ) -> IndexJobFuture<'a, String>;

    /// Upsert the corpus row WITHOUT enqueueing an indexer job. Used by
    /// `POST /api/v1/corpora` in cloud mode when the paths are local
    /// (the serve pod has no source files, so dispatching an indexer
    /// job would just discover 0 files and pollute the queue). The
    /// corpus appears in `list_corpora` as a logical container that
    /// later `POST .../clone` calls can attach indexed content to.
    fn register_corpus_only<'a>(
        &'a self,
        corpus_id: &'a str,
        paths: &'a [String],
        display_name: Option<&'a str>,
    ) -> IndexJobFuture<'a, ()>;

    /// Look up the most-recent job for a corpus. The daemon's progress
    /// SSE polls this every ~500ms and yields the snapshot. Returns
    /// `None` when the corpus exists but has never been queued.
    fn latest_for_corpus<'a>(
        &'a self,
        corpus_id: &'a str,
    ) -> IndexJobFuture<'a, Option<IndexJobSnapshot>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct MockSink {
        jobs: Mutex<Vec<IndexJobSnapshot>>,
    }

    impl IndexJobSink for MockSink {
        fn create_pending<'a>(
            &'a self,
            corpus_id: &'a str,
            _paths: &'a [String],
            _display_name: Option<&'a str>,
            _clone_url: Option<&'a str>,
        ) -> IndexJobFuture<'a, String> {
            Box::pin(async move {
                let job_id = format!("job-{corpus_id}");
                let mut jobs = self.jobs.lock().unwrap();
                jobs.push(IndexJobSnapshot {
                    job_id: job_id.clone(),
                    corpus_id: corpus_id.to_string(),
                    status: IndexJobStatus::Pending,
                    stage: "pending".into(),
                    total_files: 0,
                    processed_files: 0,
                    current_file: None,
                    error: None,
                    sections_done: 0,
                    embeddings_total: 0,
                    embeddings_done: 0,
                });
                Ok(job_id)
            })
        }

        fn register_corpus_only<'a>(
            &'a self,
            _corpus_id: &'a str,
            _paths: &'a [String],
            _display_name: Option<&'a str>,
        ) -> IndexJobFuture<'a, ()> {
            Box::pin(async move { Ok(()) })
        }

        fn latest_for_corpus<'a>(
            &'a self,
            corpus_id: &'a str,
        ) -> IndexJobFuture<'a, Option<IndexJobSnapshot>> {
            Box::pin(async move {
                Ok(self
                    .jobs
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|j| j.corpus_id == corpus_id)
                    .cloned())
            })
        }
    }

    #[tokio::test]
    async fn trait_is_dyn_compatible_and_round_trips() {
        let sink: std::sync::Arc<dyn IndexJobSink> = std::sync::Arc::new(MockSink::default());
        let job_id = sink
            .create_pending("c1", &["/tmp/x".into()], Some("X"), None)
            .await
            .unwrap();
        assert_eq!(job_id, "job-c1");
        let snap = sink.latest_for_corpus("c1").await.unwrap().unwrap();
        assert_eq!(snap.status, IndexJobStatus::Pending);
        assert_eq!(snap.corpus_id, "c1");
    }
}
