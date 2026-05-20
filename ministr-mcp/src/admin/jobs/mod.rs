//! Indexer job queue: trait, record types, and backend dispatcher.
//!
//! The queue bridges the **query app** (which receives `/reindex` requests)
//! and the **indexer worker** (which dequeues and runs them). Two backends:
//!
//! - `InMemoryJobQueue` — single-process, ephemeral; used by local dev and
//!   any deployment where the indexer runs in the same container.
//! - `SqliteJobQueue` — persisted at `$DATA_DIR/jobs.db` on the Azure Files
//!   mount; survives pod restarts and works when the indexer is a
//!   separate ACA Job that mounts the same share.

mod in_memory;
pub(crate) mod postgres;
mod sqlite;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub(crate) use in_memory::InMemoryJobQueue;
pub(crate) use postgres::PostgresJobQueue;
pub(crate) use sqlite::SqliteJobQueue;

/// Result alias for queue operations.
pub(crate) type JobResult<T> = Result<T, JobQueueError>;

/// Errors that a queue backend can surface.
#[derive(Debug, Error)]
#[allow(dead_code)] // variants surface once a persistent backend is selected
pub(crate) enum JobQueueError {
    #[error("job queue backend error: {0}")]
    Backend(String),
    #[error("job queue serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("job not found: {0}")]
    NotFound(String),
}

/// What triggered an indexing job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum JobTrigger {
    /// Direct `/reindex` POST.
    Manual,
    /// GitHub webhook (push event).
    #[allow(dead_code)] // wired in PR5 when the webhook fires
    Github {
        #[serde(rename = "ref")]
        reference: String,
        commit: String,
    },
}

/// Lifecycle status of an indexing job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl JobStatus {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// Indexer progress snapshot. Updated by the worker; streamed by SSE.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct JobProgress {
    pub(crate) stage: String,
    pub(crate) total_files: u64,
    pub(crate) processed_files: u64,
    pub(crate) current_file: Option<String>,
}

/// A reindex job record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Job {
    pub(crate) id: String,
    pub(crate) corpus_id: String,
    pub(crate) trigger: JobTrigger,
    pub(crate) status: JobStatus,
    pub(crate) progress: JobProgress,
    pub(crate) created_at: u64,
    pub(crate) updated_at: u64,
    pub(crate) error: Option<String>,
    /// F2.2 — tier-derived scheduling priority. Higher wins. The
    /// Postgres backend drains `ORDER BY priority DESC, created_at ASC`
    /// so Team jumps Pro; in-memory + `SQLite` back-ends ignore the
    /// value (single-worker self-hosted has no notion of priority).
    /// Defaults to `0` to keep self-hosted enqueue calls source-stable
    /// — they emit a single bucket and queue order remains FIFO.
    #[serde(default)]
    pub(crate) priority: i16,
}

/// Contract every queue backend implements.
///
/// Semantics:
/// - `enqueue` returns a job with status `Pending` and a fresh id.
/// - `claim_next` is atomic: it picks the oldest `Pending` job and
///   transitions it to `Running` in one transaction. Multiple workers
///   calling concurrently must each see *different* jobs (or `None`).
/// - `update_*` is upsert-by-id. Concurrent updates from the worker and
///   `claim_next` are safe under WAL.
pub(crate) trait JobQueue: Send + Sync {
    /// Enqueue a new pending job. `priority` is the tier-derived
    /// scheduling weight (see [`Job::priority`]); pass `0` from
    /// self-hosted call sites where every job sits in a single bucket.
    fn enqueue(
        &self,
        corpus_id: String,
        trigger: JobTrigger,
        priority: i16,
    ) -> impl Future<Output = JobResult<Job>> + Send;

    fn get(&self, job_id: &str) -> impl Future<Output = JobResult<Option<Job>>> + Send;

    #[allow(dead_code)] // consumed by the indexer worker in PR2
    fn claim_next(&self) -> impl Future<Output = JobResult<Option<Job>>> + Send;

    #[allow(dead_code)] // consumed by the indexer worker in PR2
    fn update_progress(
        &self,
        job_id: &str,
        progress: JobProgress,
    ) -> impl Future<Output = JobResult<()>> + Send;

    #[allow(dead_code)] // consumed by the indexer worker in PR2
    fn finish(
        &self,
        job_id: &str,
        status: JobStatus,
        error: Option<String>,
    ) -> impl Future<Output = JobResult<()>> + Send;
}

/// Concrete dispatcher. Add a variant to support a new backend.
#[derive(Debug, Clone)]
#[allow(dead_code)] // selected by admin router builder once wired in PR1.4
pub(crate) enum JobQueueBackend {
    InMemory(InMemoryJobQueue),
    Sqlite(SqliteJobQueue),
    Postgres(PostgresJobQueue),
}

impl JobQueueBackend {
    pub(crate) async fn enqueue(
        &self,
        corpus_id: String,
        trigger: JobTrigger,
        priority: i16,
    ) -> JobResult<Job> {
        match self {
            Self::InMemory(q) => q.enqueue(corpus_id, trigger, priority).await,
            Self::Sqlite(q) => q.enqueue(corpus_id, trigger, priority).await,
            Self::Postgres(q) => q.enqueue(corpus_id, trigger, priority).await,
        }
    }

    pub(crate) async fn get(&self, job_id: &str) -> JobResult<Option<Job>> {
        match self {
            Self::InMemory(q) => q.get(job_id).await,
            Self::Sqlite(q) => q.get(job_id).await,
            Self::Postgres(q) => q.get(job_id).await,
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn claim_next(&self) -> JobResult<Option<Job>> {
        match self {
            Self::InMemory(q) => q.claim_next().await,
            Self::Sqlite(q) => q.claim_next().await,
            Self::Postgres(q) => q.claim_next().await,
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn update_progress(
        &self,
        job_id: &str,
        progress: JobProgress,
    ) -> JobResult<()> {
        match self {
            Self::InMemory(q) => q.update_progress(job_id, progress).await,
            Self::Sqlite(q) => q.update_progress(job_id, progress).await,
            Self::Postgres(q) => q.update_progress(job_id, progress).await,
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn finish(
        &self,
        job_id: &str,
        status: JobStatus,
        error: Option<String>,
    ) -> JobResult<()> {
        match self {
            Self::InMemory(q) => q.finish(job_id, status, error).await,
            Self::Sqlite(q) => q.finish(job_id, status, error).await,
            Self::Postgres(q) => q.finish(job_id, status, error).await,
        }
    }
}
