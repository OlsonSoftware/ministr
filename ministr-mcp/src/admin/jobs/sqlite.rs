//! SQLite-backed indexer job queue at `$DATA_DIR/jobs.db`.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use tokio::task;
use tracing::debug;

use super::super::ids::new_job_id;
use super::{Job, JobProgress, JobQueue, JobQueueError, JobResult, JobStatus, JobTrigger};
use crate::time::epoch_now;

#[derive(Debug, Clone)]
pub struct SqliteJobQueue {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteJobQueue {
    /// Open (or create) a `SQLite`-backed job queue at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`JobQueueError::Backend`] if the file cannot be opened
    /// or the schema cannot be applied.
    #[allow(dead_code)] // wired in PR1.4
    pub fn open(path: &Path) -> JobResult<Self> {
        let conn = Connection::open(path)
            .map_err(|e| JobQueueError::Backend(format!("open {}: {e}", path.display())))?;
        configure(&conn)?;
        ensure_schema(&conn)?;
        debug!(path = %path.display(), "opened sqlite job queue");
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn with_conn<T, F>(&self, op: F) -> impl Future<Output = JobResult<T>> + Send
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> JobResult<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        async move {
            task::spawn_blocking(move || {
                let mut guard = conn
                    .lock()
                    .map_err(|e| JobQueueError::Backend(format!("mutex poisoned: {e}")))?;
                op(&mut guard)
            })
            .await
            .map_err(|e| JobQueueError::Backend(format!("join: {e}")))?
        }
    }
}

fn configure(conn: &Connection) -> JobResult<()> {
    // Same WAL-with-DELETE-fallback pattern as the OAuth sqlite (see
    // `auth/storage/sqlite.rs`) and `ministr-core::configure_connection`.
    // `MINISTR_REQUIRE_WAL=0` permits both silent downgrade *and* hard
    // pragma failure (the SMB case).
    let require_wal = std::env::var("MINISTR_REQUIRE_WAL").map_or(true, |v| v != "0");

    match conn.pragma_update(None, "journal_mode", "WAL") {
        Ok(()) => {
            let actual: String = conn
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .map_err(|e| JobQueueError::Backend(format!("read journal_mode: {e}")))?;
            if !actual.eq_ignore_ascii_case("wal") {
                if require_wal {
                    return Err(JobQueueError::Backend(format!(
                        "journal_mode did not stick — got {actual:?}, wanted WAL \
                         (set MINISTR_REQUIRE_WAL=0 to allow DELETE fallback)"
                    )));
                }
                tracing::debug!(mode = %actual, "admin jobs sqlite: WAL silently downgraded");
            }
        }
        Err(e) => {
            if require_wal {
                return Err(JobQueueError::Backend(format!("journal_mode: {e}")));
            }
            tracing::debug!(error = %e, "admin jobs sqlite: WAL pragma failed; using DELETE");
            conn.pragma_update(None, "journal_mode", "DELETE")
                .map_err(|e| JobQueueError::Backend(format!("journal_mode DELETE: {e}")))?;
        }
    }
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| JobQueueError::Backend(format!("synchronous: {e}")))?;
    conn.pragma_update(None, "busy_timeout", 5_000)
        .map_err(|e| JobQueueError::Backend(format!("busy_timeout: {e}")))?;
    Ok(())
}

fn ensure_schema(conn: &Connection) -> JobResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS indexer_jobs (
            id          TEXT PRIMARY KEY,
            corpus_id   TEXT NOT NULL,
            status      TEXT NOT NULL,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL,
            data        TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_indexer_jobs_status_created
            ON indexer_jobs(status, created_at);",
    )
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

impl JobQueue for SqliteJobQueue {
    fn enqueue(
        &self,
        corpus_id: String,
        trigger: JobTrigger,
        priority: i16,
    ) -> impl Future<Output = JobResult<Job>> + Send {
        self.with_conn(move |conn| {
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
                // Self-hosted single-worker SQLite path doesn't honour
                // priority — captured on the JSON blob for future
                // observability but never used by `claim_next`.
                priority,
            };
            let blob = serialise(&job)?;
            conn.execute(
                "INSERT INTO indexer_jobs (id, corpus_id, status, created_at, updated_at, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    job.id,
                    job.corpus_id,
                    status_str(job.status),
                    job.created_at.cast_signed(),
                    job.updated_at.cast_signed(),
                    blob
                ],
            )
            .map_err(|e| JobQueueError::Backend(format!("enqueue: {e}")))?;
            Ok(job)
        })
    }

    fn get(&self, job_id: &str) -> impl Future<Output = JobResult<Option<Job>>> + Send {
        let job_id = job_id.to_owned();
        self.with_conn(move |conn| {
            let blob: Option<String> = conn
                .query_row(
                    "SELECT data FROM indexer_jobs WHERE id = ?1",
                    params![job_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| JobQueueError::Backend(format!("get: {e}")))?;
            match blob {
                Some(s) => Ok(Some(deserialise(&s)?)),
                None => Ok(None),
            }
        })
    }

    fn claim_next(&self) -> impl Future<Output = JobResult<Option<Job>>> + Send {
        self.with_conn(|conn| {
            let tx = conn
                .transaction()
                .map_err(|e| JobQueueError::Backend(format!("begin: {e}")))?;
            let row: Option<(String, String)> = tx
                .query_row(
                    "SELECT id, data FROM indexer_jobs
                     WHERE status = 'pending'
                     ORDER BY created_at
                     LIMIT 1",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|e| JobQueueError::Backend(format!("claim select: {e}")))?;
            let Some((id, blob)) = row else {
                tx.commit().ok();
                return Ok(None);
            };
            let mut job = deserialise(&blob)?;
            job.status = JobStatus::Running;
            job.updated_at = epoch_now();
            let updated = serialise(&job)?;
            tx.execute(
                "UPDATE indexer_jobs SET status = ?1, updated_at = ?2, data = ?3 WHERE id = ?4",
                params![
                    status_str(job.status),
                    job.updated_at.cast_signed(),
                    updated,
                    id
                ],
            )
            .map_err(|e| JobQueueError::Backend(format!("claim update: {e}")))?;
            tx.commit()
                .map_err(|e| JobQueueError::Backend(format!("claim commit: {e}")))?;
            Ok(Some(job))
        })
    }

    fn update_progress(
        &self,
        job_id: &str,
        progress: JobProgress,
    ) -> impl Future<Output = JobResult<()>> + Send {
        let job_id = job_id.to_owned();
        self.with_conn(move |conn| {
            let mut job = match conn
                .query_row(
                    "SELECT data FROM indexer_jobs WHERE id = ?1",
                    params![job_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| JobQueueError::Backend(format!("update_progress select: {e}")))?
            {
                Some(blob) => deserialise(&blob)?,
                None => return Err(JobQueueError::NotFound(job_id)),
            };
            job.progress = progress;
            job.updated_at = epoch_now();
            let blob = serialise(&job)?;
            conn.execute(
                "UPDATE indexer_jobs SET updated_at = ?1, data = ?2 WHERE id = ?3",
                params![job.updated_at.cast_signed(), blob, job_id],
            )
            .map_err(|e| JobQueueError::Backend(format!("update_progress: {e}")))?;
            Ok(())
        })
    }

    fn finish(
        &self,
        job_id: &str,
        status: JobStatus,
        error: Option<String>,
    ) -> impl Future<Output = JobResult<()>> + Send {
        let job_id = job_id.to_owned();
        self.with_conn(move |conn| {
            let mut job = match conn
                .query_row(
                    "SELECT data FROM indexer_jobs WHERE id = ?1",
                    params![job_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| JobQueueError::Backend(format!("finish select: {e}")))?
            {
                Some(blob) => deserialise(&blob)?,
                None => return Err(JobQueueError::NotFound(job_id)),
            };
            job.status = status;
            job.error = error;
            job.updated_at = epoch_now();
            let blob = serialise(&job)?;
            conn.execute(
                "UPDATE indexer_jobs SET status = ?1, updated_at = ?2, data = ?3 WHERE id = ?4",
                params![
                    status_str(job.status),
                    job.updated_at.cast_signed(),
                    blob,
                    job_id
                ],
            )
            .map_err(|e| JobQueueError::Backend(format!("finish: {e}")))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn open() -> (tempfile::TempDir, SqliteJobQueue) {
        let dir = tempdir().unwrap();
        let q = SqliteJobQueue::open(&dir.path().join("jobs.db")).unwrap();
        (dir, q)
    }

    #[tokio::test]
    async fn enqueue_and_round_trip_through_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("jobs.db");

        let id;
        {
            let q = SqliteJobQueue::open(&path).unwrap();
            let job = q.enqueue("c1".into(), JobTrigger::Manual, 0).await.unwrap();
            id = job.id;
        }
        // Re-open simulates a restart.
        let q = SqliteJobQueue::open(&path).unwrap();
        let got = q.get(&id).await.unwrap().unwrap();
        assert_eq!(got.corpus_id, "c1");
        assert_eq!(got.status, JobStatus::Pending);
    }

    #[tokio::test]
    async fn claim_next_transitions_status_atomically() {
        let (_dir, q) = open();
        let job = q.enqueue("c1".into(), JobTrigger::Manual, 0).await.unwrap();
        let claimed = q.claim_next().await.unwrap().unwrap();
        assert_eq!(claimed.id, job.id);
        assert_eq!(claimed.status, JobStatus::Running);
        assert!(q.claim_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn progress_and_finish() {
        let (_dir, q) = open();
        let job = q.enqueue("c1".into(), JobTrigger::Manual, 0).await.unwrap();
        q.update_progress(
            &job.id,
            JobProgress {
                stage: "embedding".into(),
                total_files: 100,
                processed_files: 42,
                current_file: Some("foo.rs".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        q.finish(&job.id, JobStatus::Completed, None).await.unwrap();
        let got = q.get(&job.id).await.unwrap().unwrap();
        assert_eq!(got.status, JobStatus::Completed);
        assert_eq!(got.progress.processed_files, 42);
    }
}
