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
pub mod postgres;
mod sqlite;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use in_memory::InMemoryJobQueue;
pub use postgres::PostgresJobQueue;
pub use sqlite::SqliteJobQueue;

/// Result alias for queue operations.
pub type JobResult<T> = Result<T, JobQueueError>;

/// Errors that a queue backend can surface.
#[derive(Debug, Error)]
#[allow(dead_code)] // variants surface once a persistent backend is selected
pub enum JobQueueError {
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
pub enum JobTrigger {
    /// Direct `/reindex` POST.
    Manual,
    /// GitHub webhook (push event).
    #[allow(dead_code)] // wired in PR5 when the webhook fires
    Github {
        #[serde(rename = "ref")]
        reference: String,
        commit: String,
    },
    /// PHASE3 — tenant-initiated cloud ingestion. Emitted by the serve
    /// pod's `POST /api/v1/corpora` handler in chunk 4 so the indexer
    /// worker (chunk 3) can pop the job, clone `clone_url` if set, run
    /// `indexer::run` against `paths`, upload the bundle, and mark the
    /// job done. The Job's `corpus_id` carries the deterministic id
    /// computed from the canonical paths.
    #[allow(dead_code)] // wired in chunk 4 when serve enqueues replace inline register
    Tenant {
        paths: Vec<String>,
        clone_url: Option<String>,
    },
}

/// Lifecycle status of an indexing job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl JobStatus {
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// Indexer progress snapshot. Updated by the worker; streamed by SSE.
///
/// `total_files` / `processed_files` are the parser-side counts —
/// `processed_files` is bumped per file as soon as parsing completes,
/// which can race ahead of the embedder by minutes on a large corpus.
/// `sections_done` / `embeddings_*` are the embedding-side counters
/// added in PHASE5 chunk 3: the streaming consumer updates the
/// in-memory `IngestionProgress` per batch and the 500ms reporter
/// snapshots all five into this struct. SSE clients render embedding
/// progress as the primary signal during the long embedder phase, with
/// `processed_files` as a secondary "parse phase done" indicator.
///
/// `serde(default)` on the new fields keeps in-flight Postgres rows
/// from PHASE4 deserialisable — old rows simply report zero for the
/// new fields until the next worker writes a fresh snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobProgress {
    pub stage: String,
    pub total_files: u64,
    pub processed_files: u64,
    pub current_file: Option<String>,
    /// PHASE5 chunk 3 — section count bumped by the producer per file.
    #[serde(default)]
    pub sections_done: u64,
    /// PHASE5 chunk 3 — embedding-pairs *expected* across the run.
    /// Producer increments as it discovers them; reflects total work
    /// the embedder needs to do.
    #[serde(default)]
    pub embeddings_total: u64,
    /// PHASE5 chunk 3 — embedding-pairs *flushed to HNSW*. Streaming
    /// consumer bumps per `batch_embed_and_insert`. SSE renders this as
    /// the embedding-progress bar.
    #[serde(default)]
    pub embeddings_done: u64,
}

/// A reindex job record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub corpus_id: String,
    pub trigger: JobTrigger,
    pub status: JobStatus,
    pub progress: JobProgress,
    pub created_at: u64,
    pub updated_at: u64,
    pub error: Option<String>,
    /// F2.2 — tier-derived scheduling priority. Higher wins. The
    /// Postgres backend drains `ORDER BY priority DESC, created_at ASC`
    /// so Team jumps Pro; in-memory + `SQLite` back-ends ignore the
    /// value (single-worker self-hosted has no notion of priority).
    /// Defaults to `0` to keep self-hosted enqueue calls source-stable
    /// — they emit a single bucket and queue order remains FIFO.
    #[serde(default)]
    pub priority: i16,
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
pub trait JobQueue: Send + Sync {
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
pub enum JobQueueBackend {
    InMemory(InMemoryJobQueue),
    Sqlite(SqliteJobQueue),
    Postgres(PostgresJobQueue),
}

impl JobQueueBackend {
    /// Enqueue a new pending job through the active backend.
    ///
    /// # Errors
    ///
    /// Surfaces the backend's [`JobQueueError`] on connection or
    /// serialization failure.
    pub async fn enqueue(
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

    /// Fetch a job by id. Returns `None` when the id is unknown.
    ///
    /// # Errors
    ///
    /// Surfaces the backend's [`JobQueueError`] on storage failure.
    pub async fn get(&self, job_id: &str) -> JobResult<Option<Job>> {
        match self {
            Self::InMemory(q) => q.get(job_id).await,
            Self::Sqlite(q) => q.get(job_id).await,
            Self::Postgres(q) => q.get(job_id).await,
        }
    }

    /// Atomically claim the next pending job (transitioning it to
    /// `Running`) so a worker can run it.
    ///
    /// # Errors
    ///
    /// Surfaces the backend's [`JobQueueError`] on storage failure.
    #[allow(dead_code)]
    pub async fn claim_next(&self) -> JobResult<Option<Job>> {
        match self {
            Self::InMemory(q) => q.claim_next().await,
            Self::Sqlite(q) => q.claim_next().await,
            Self::Postgres(q) => q.claim_next().await,
        }
    }

    /// Persist a `JobProgress` snapshot for an in-flight job.
    ///
    /// # Errors
    ///
    /// Returns [`JobQueueError::NotFound`] if `job_id` does not exist;
    /// other variants surface backend failures.
    #[allow(dead_code)]
    pub async fn update_progress(
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

    /// Mark a job terminal (`Completed` or `Failed`) and record an
    /// optional error message.
    ///
    /// # Errors
    ///
    /// Returns [`JobQueueError::NotFound`] if `job_id` does not exist;
    /// other variants surface backend failures.
    #[allow(dead_code)]
    pub async fn finish(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_trigger_tenant_round_trips_through_json() {
        // The PostgresJobQueue stores the whole Job (with its embedded
        // JobTrigger) as a JSON blob in the `data` column. Round-trip
        // the new variant to confirm the snake_case tag wins under
        // serde and the optional `clone_url` survives a None.
        let trigger = JobTrigger::Tenant {
            paths: vec!["/tmp/x".into(), "/tmp/y".into()],
            clone_url: Some("https://github.com/dtolnay/anyhow".into()),
        };
        let json = serde_json::to_string(&trigger).unwrap();
        assert!(json.contains(r#""kind":"tenant""#), "got {json}");
        assert!(json.contains(r#""clone_url":"https://github.com/dtolnay/anyhow""#));
        let parsed: JobTrigger = serde_json::from_str(&json).unwrap();
        match parsed {
            JobTrigger::Tenant { paths, clone_url } => {
                assert_eq!(paths, vec!["/tmp/x".to_string(), "/tmp/y".to_string()]);
                assert_eq!(clone_url.as_deref(), Some("https://github.com/dtolnay/anyhow"));
            }
            other => panic!("expected Tenant, got {other:?}"),
        }
    }

    #[test]
    fn job_trigger_tenant_round_trips_without_clone_url() {
        let trigger = JobTrigger::Tenant {
            paths: vec!["/tmp/x".into()],
            clone_url: None,
        };
        let json = serde_json::to_string(&trigger).unwrap();
        let parsed: JobTrigger = serde_json::from_str(&json).unwrap();
        match parsed {
            JobTrigger::Tenant { clone_url, .. } => assert!(clone_url.is_none()),
            other => panic!("expected Tenant, got {other:?}"),
        }
    }
}
