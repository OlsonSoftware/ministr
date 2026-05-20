//! Postgres-backed indexer job queue.
//!
//! Mirrors `sqlite.rs` for multi-pod cloud deployments where the query
//! App and one or more Indexer Workers run as separate ACA containers
//! that all need a single coherent queue. Schema and operation shapes
//! match the `SQLite` store one-for-one (`TEXT` pk, `BIGINT` timestamps,
//! `TEXT` JSON blob payload), so an in-process swap from `SQLite` to
//! Postgres is a one-line backend-selector change in `cmd_serve_http`.
//!
//! # Concurrency advantage over `SQLite`
//!
//! `claim_next` uses `SELECT … FOR UPDATE SKIP LOCKED` — a Postgres
//! idiom that lets N workers race for the head of the queue and each
//! receive a *different* row (or `None`). The `SQLite` version can't
//! express this; it relies on a serialised transaction that scales to
//! one writer. F2.2's priority-queue cross-pool draining and F5.5's
//! Enterprise dedicated lane both depend on this primitive.

use std::str::FromStr;

use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use rustls::ClientConfig;
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::debug;

use super::super::ids::new_job_id;
use super::{Job, JobProgress, JobQueue, JobQueueError, JobResult, JobStatus, JobTrigger};
use crate::time::epoch_now;

/// Persistent indexer queue, deadpool-pooled.
#[derive(Debug, Clone)]
#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector in F1.2
pub(crate) struct PostgresJobQueue {
    pool: Pool,
}

#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector in F1.2
impl PostgresJobQueue {
    /// Open (or attach to) the `indexer_jobs` table in the database
    /// referenced by `url`. Schema creation is idempotent so every pod
    /// can run this on boot without coordination.
    pub(crate) async fn open(url: &str) -> JobResult<Self> {
        let mut cfg = Config::new();
        cfg.url = Some(url.to_string());
        cfg.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });

        let tls = make_rustls_connector();
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), tls)
            .map_err(|e| JobQueueError::Backend(format!("create_pool: {e}")))?;

        let host_hint = redact_url_host(url);
        debug!(host = %host_hint, "opening postgres job queue");

        ensure_schema(&pool).await?;
        Ok(Self { pool })
    }

    /// Bare-pool constructor for tests that supply a pre-configured
    /// `Pool` (custom TLS, local container, etc.).
    #[cfg(test)]
    pub(crate) async fn from_pool(pool: Pool) -> JobResult<Self> {
        ensure_schema(&pool).await?;
        Ok(Self { pool })
    }
}

#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector in F1.2
fn make_rustls_connector() -> MakeRustlsConnect {
    // Mozilla root bundle. See `auth/storage/postgres.rs` for the
    // rationale on always-on TLS — Azure Postgres Flex requires it.
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    MakeRustlsConnect::new(config)
}

#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector in F1.2
fn redact_url_host(url: &str) -> String {
    tokio_postgres::Config::from_str(url)
        .ok()
        .and_then(|cfg| cfg.get_hosts().first().cloned())
        .map_or_else(|| "<unknown>".to_owned(), |h| format!("{h:?}"))
}

#[allow(dead_code)] // wired into `cmd_serve_http` cloud-mode selector in F1.2
async fn ensure_schema(pool: &Pool) -> JobResult<()> {
    let client = pool
        .get()
        .await
        .map_err(|e| JobQueueError::Backend(format!("schema get conn: {e}")))?;
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS indexer_jobs (
                 id          TEXT PRIMARY KEY,
                 corpus_id   TEXT NOT NULL,
                 status      TEXT NOT NULL,
                 created_at  BIGINT NOT NULL,
                 updated_at  BIGINT NOT NULL,
                 data        TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_indexer_jobs_status_created
                 ON indexer_jobs (status, created_at);
             -- F2.2: priority lane. `ALTER TABLE ADD COLUMN IF NOT EXISTS`
             -- is idempotent in Postgres 9.6+; safe to run on every pod
             -- boot. The matching index covers the `(status, priority,
             -- created_at)` drain order used by claim_next so the
             -- planner picks an index scan over a seq scan on busy
             -- queues.
             ALTER TABLE indexer_jobs
                 ADD COLUMN IF NOT EXISTS priority SMALLINT NOT NULL DEFAULT 0;
             CREATE INDEX IF NOT EXISTS idx_indexer_jobs_status_priority_created
                 ON indexer_jobs (status, priority DESC, created_at);",
        )
        .await
        .map_err(|e| JobQueueError::Backend(format!("schema: {e}")))?;
    Ok(())
}

fn serialise(job: &Job) -> JobResult<String> {
    Ok(serde_json::to_string(job)?)
}

fn deserialise(blob: &str) -> JobResult<Job> {
    Ok(serde_json::from_str(blob)?)
}

fn status_str(s: JobStatus) -> &'static str {
    match s {
        JobStatus::Pending => "pending",
        JobStatus::Running => "running",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
    }
}

impl JobQueue for PostgresJobQueue {
    fn enqueue(
        &self,
        corpus_id: String,
        trigger: JobTrigger,
        priority: i16,
    ) -> impl Future<Output = JobResult<Job>> + Send {
        let pool = self.pool.clone();
        async move {
            let now = epoch_now();
            let job = Job {
                id: new_job_id(),
                corpus_id,
                trigger,
                status: JobStatus::Pending,
                progress: JobProgress::default(),
                created_at: now,
                updated_at: now,
                error: None,
                priority,
            };
            let blob = serialise(&job)?;
            let created = job.created_at.cast_signed();
            let updated = job.updated_at.cast_signed();
            let conn = pool
                .get()
                .await
                .map_err(|e| JobQueueError::Backend(format!("enqueue conn: {e}")))?;
            conn.execute(
                "INSERT INTO indexer_jobs
                     (id, corpus_id, status, created_at, updated_at, data, priority)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[
                    &job.id,
                    &job.corpus_id,
                    &status_str(job.status),
                    &created,
                    &updated,
                    &blob,
                    &job.priority,
                ],
            )
            .await
            .map_err(|e| JobQueueError::Backend(format!("enqueue: {e}")))?;
            Ok(job)
        }
    }

    fn get(&self, job_id: &str) -> impl Future<Output = JobResult<Option<Job>>> + Send {
        let pool = self.pool.clone();
        let job_id = job_id.to_owned();
        async move {
            let conn = pool
                .get()
                .await
                .map_err(|e| JobQueueError::Backend(format!("get conn: {e}")))?;
            let row = conn
                .query_opt(
                    "SELECT data FROM indexer_jobs WHERE id = $1",
                    &[&job_id],
                )
                .await
                .map_err(|e| JobQueueError::Backend(format!("get: {e}")))?;
            match row {
                Some(r) => {
                    let blob: String = r
                        .try_get("data")
                        .map_err(|e| JobQueueError::Backend(format!("get row.data: {e}")))?;
                    Ok(Some(deserialise(&blob)?))
                }
                None => Ok(None),
            }
        }
    }

    fn claim_next(&self) -> impl Future<Output = JobResult<Option<Job>>> + Send {
        let pool = self.pool.clone();
        async move {
            let mut conn = pool
                .get()
                .await
                .map_err(|e| JobQueueError::Backend(format!("claim conn: {e}")))?;
            let tx = conn
                .transaction()
                .await
                .map_err(|e| JobQueueError::Backend(format!("claim tx: {e}")))?;
            // SELECT … FOR UPDATE SKIP LOCKED — the Postgres-idiomatic
            // way to let N workers race the head of the queue and each
            // get a different row. SKIP LOCKED returns rows other
            // workers haven't acquired in *their* in-flight tx; the
            // FOR UPDATE keeps our row locked until commit.
            //
            // F2.2: ORDER BY priority DESC, created_at ASC — Team jumps
            // Pro; Enterprise jumps both. Ties on priority fall back to
            // FIFO submission order. The composite index
            // `idx_indexer_jobs_status_priority_created` covers this
            // ordering for an index-only scan.
            let row = tx
                .query_opt(
                    "SELECT id, data FROM indexer_jobs
                       WHERE status = 'pending'
                       ORDER BY priority DESC, created_at ASC
                       FOR UPDATE SKIP LOCKED
                       LIMIT 1",
                    &[],
                )
                .await
                .map_err(|e| JobQueueError::Backend(format!("claim select: {e}")))?;
            let Some(r) = row else {
                tx.commit()
                    .await
                    .map_err(|e| JobQueueError::Backend(format!("claim commit: {e}")))?;
                return Ok(None);
            };
            let id: String = r
                .try_get("id")
                .map_err(|e| JobQueueError::Backend(format!("claim row.id: {e}")))?;
            let blob: String = r
                .try_get("data")
                .map_err(|e| JobQueueError::Backend(format!("claim row.data: {e}")))?;
            let mut job = deserialise(&blob)?;
            job.status = JobStatus::Running;
            job.updated_at = epoch_now();
            let updated = serialise(&job)?;
            tx.execute(
                "UPDATE indexer_jobs
                    SET status = $1, updated_at = $2, data = $3
                  WHERE id = $4",
                &[
                    &status_str(job.status),
                    &job.updated_at.cast_signed(),
                    &updated,
                    &id,
                ],
            )
            .await
            .map_err(|e| JobQueueError::Backend(format!("claim update: {e}")))?;
            tx.commit()
                .await
                .map_err(|e| JobQueueError::Backend(format!("claim commit: {e}")))?;
            Ok(Some(job))
        }
    }

    fn update_progress(
        &self,
        job_id: &str,
        progress: JobProgress,
    ) -> impl Future<Output = JobResult<()>> + Send {
        let pool = self.pool.clone();
        let job_id = job_id.to_owned();
        async move {
            let conn = pool
                .get()
                .await
                .map_err(|e| JobQueueError::Backend(format!("update_progress conn: {e}")))?;
            let row = conn
                .query_opt(
                    "SELECT data FROM indexer_jobs WHERE id = $1",
                    &[&job_id],
                )
                .await
                .map_err(|e| JobQueueError::Backend(format!("update_progress select: {e}")))?;
            let blob: String = match row {
                Some(r) => r
                    .try_get("data")
                    .map_err(|e| JobQueueError::Backend(format!("update_progress row.data: {e}")))?,
                None => return Err(JobQueueError::NotFound(job_id)),
            };
            let mut job = deserialise(&blob)?;
            job.progress = progress;
            job.updated_at = epoch_now();
            let updated = serialise(&job)?;
            conn.execute(
                "UPDATE indexer_jobs SET updated_at = $1, data = $2 WHERE id = $3",
                &[&job.updated_at.cast_signed(), &updated, &job_id],
            )
            .await
            .map_err(|e| JobQueueError::Backend(format!("update_progress: {e}")))?;
            Ok(())
        }
    }

    fn finish(
        &self,
        job_id: &str,
        status: JobStatus,
        error: Option<String>,
    ) -> impl Future<Output = JobResult<()>> + Send {
        let pool = self.pool.clone();
        let job_id = job_id.to_owned();
        async move {
            let conn = pool
                .get()
                .await
                .map_err(|e| JobQueueError::Backend(format!("finish conn: {e}")))?;
            let row = conn
                .query_opt(
                    "SELECT data FROM indexer_jobs WHERE id = $1",
                    &[&job_id],
                )
                .await
                .map_err(|e| JobQueueError::Backend(format!("finish select: {e}")))?;
            let blob: String = match row {
                Some(r) => r
                    .try_get("data")
                    .map_err(|e| JobQueueError::Backend(format!("finish row.data: {e}")))?,
                None => return Err(JobQueueError::NotFound(job_id)),
            };
            let mut job = deserialise(&blob)?;
            job.status = status;
            job.error = error;
            job.updated_at = epoch_now();
            let updated = serialise(&job)?;
            conn.execute(
                "UPDATE indexer_jobs
                    SET status = $1, updated_at = $2, data = $3
                  WHERE id = $4",
                &[
                    &status_str(job.status),
                    &job.updated_at.cast_signed(),
                    &updated,
                    &job_id,
                ],
            )
            .await
            .map_err(|e| JobQueueError::Backend(format!("finish: {e}")))?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests. Require a real Postgres at
    //! `MINISTR_TEST_PG_URL`. Marked `#[ignore]` so the default
    //! `cargo test` run stays dependency-free; CI flips the env var
    //! and reruns with `cargo test -- --ignored`.

    use super::*;
    use std::sync::Arc;

    fn test_url() -> Option<String> {
        std::env::var("MINISTR_TEST_PG_URL").ok()
    }

    async fn open() -> Option<PostgresJobQueue> {
        let url = test_url()?;
        Some(PostgresJobQueue::open(&url).await.expect("open postgres"))
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn enqueue_and_round_trip() {
        let Some(q) = open().await else { return };
        let job = q
            .enqueue(format!("c-{}", epoch_now()), JobTrigger::Manual, 0)
            .await
            .unwrap();
        let got = q.get(&job.id).await.unwrap().unwrap();
        assert_eq!(got.id, job.id);
        assert_eq!(got.status, JobStatus::Pending);
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn claim_next_transitions_status_atomically() {
        let Some(q) = open().await else { return };
        let job = q
            .enqueue(format!("c-{}", epoch_now()), JobTrigger::Manual, 0)
            .await
            .unwrap();
        let claimed = q.claim_next().await.unwrap().unwrap();
        assert_eq!(claimed.id, job.id);
        assert_eq!(claimed.status, JobStatus::Running);
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn progress_and_finish() {
        let Some(q) = open().await else { return };
        let job = q
            .enqueue(format!("c-{}", epoch_now()), JobTrigger::Manual, 0)
            .await
            .unwrap();
        q.update_progress(
            &job.id,
            JobProgress {
                stage: "embedding".into(),
                total_files: 100,
                processed_files: 42,
                current_file: Some("foo.rs".into()),
            },
        )
        .await
        .unwrap();
        q.finish(&job.id, JobStatus::Completed, None).await.unwrap();
        let got = q.get(&job.id).await.unwrap().unwrap();
        assert_eq!(got.status, JobStatus::Completed);
        assert_eq!(got.progress.processed_files, 42);
    }

    /// Two workers racing `claim_next` on a single pending row must
    /// each see a *different* outcome — exactly one gets the job.
    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn concurrent_claim_skip_locked() {
        let Some(q) = open().await else { return };
        let q = Arc::new(q);
        q.enqueue(format!("race-{}", epoch_now()), JobTrigger::Manual, 0)
            .await
            .unwrap();

        let mut handles = Vec::new();
        for _ in 0..4 {
            let qc = q.clone();
            handles.push(tokio::spawn(async move { qc.claim_next().await.unwrap() }));
        }
        let mut wins = 0;
        for h in handles {
            if h.await.unwrap().is_some() {
                wins += 1;
            }
        }
        assert_eq!(wins, 1, "exactly one worker should claim the row");
    }
}
