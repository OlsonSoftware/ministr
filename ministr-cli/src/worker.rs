//! PHASE6 chunk 2 — long-lived in-process indexer worker.
//!
//! Replaces the ACA-Job-driven `cmd_indexer_worker` (deleted in chunk
//! 3). The serve pod itself runs this loop on a background tokio task:
//! it polls `indexer_jobs` via [`JobQueueBackend::claim_next`] every
//! ~5 seconds, runs ingestion in-process, updates progress, marks the
//! job terminal, and goes back to polling.
//!
//! # Why
//!
//! The ACA-Job model paid an image-pull + replica-startup + model-load
//! cost on every job. With the embedder switched to Azure `OpenAI`
//! (chunk 1) the model load is gone, so the only remaining startup
//! cost is the replica boot itself — and replica boots are expensive
//! enough that amortising one replica over many jobs is the right
//! shape. See `deploy/azure/PHASE6.md` for the full diagnosis.
//!
//! # Concurrency
//!
//! One job in-flight per replica. The loop is strictly serial: claim →
//! run → finish → claim. Backlog accumulates in `indexer_jobs.pending`
//! and is drained when an existing job completes. Scale throughput by
//! adding Container App replicas; each runs its own `WorkerLoop` and
//! Postgres's `FOR UPDATE SKIP LOCKED` guarantees no two replicas claim
//! the same row.
//!
//! # Open-core posture
//!
//! Lives in `ministr-cli` (MIT) because the entire loop is
//! self-hosted-compatible — a local `ministr serve` with
//! `MINISTR_PG_URL` set behaves identically. The cloud crate
//! contributes specific concrete impls (`PostgresJobQueue`,
//! `OpenAiEmbedder`, blob backend) that the loop composes against.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::Future;
use ministr_core::config::MinistrConfig;
use ministr_mcp::admin::jobs::{Job, JobQueueBackend, JobStatus, JobTrigger};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Default polling interval — matches the PHASE3 cron cadence so
/// queued-then-immediate workloads see the same worst-case latency
/// they did before PHASE6.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Per-job execution surface. Production impl ([`IngestionRunner`])
/// runs `run_corpus_ingestion` and uploads to blob storage; tests
/// substitute a fake to drive the loop without spinning up real
/// infrastructure.
///
/// Returning `Ok(())` means the job finished cleanly and the loop
/// should mark it `Completed`. Returning `Err(message)`
/// means the loop should mark it `Failed` with that
/// message in the row.
pub trait JobRunner: Send + Sync {
    /// Run one job. The loop calls this with the freshly-claimed [`Job`];
    /// the implementation is responsible for updating
    /// `IngestionProgress` (the 500ms reporter runs in parallel) and
    /// uploading any resulting bundle.
    fn run<'a>(
        &'a self,
        job: &'a Job,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

/// Long-lived poll loop that drains `indexer_jobs` against a
/// [`JobQueueBackend`]. Cheap to construct; cancellation is via the
/// shared [`CancellationToken`] so the serve binary's shutdown handler
/// can cooperatively unwind.
pub struct WorkerLoop {
    queue: Arc<JobQueueBackend>,
    runner: Arc<dyn JobRunner>,
    poll_interval: Duration,
    cancel: CancellationToken,
}

impl WorkerLoop {
    /// Build with the workspace default poll interval. Use
    /// [`Self::with_poll_interval`] in tests to drive faster.
    #[must_use]
    pub fn new(
        queue: Arc<JobQueueBackend>,
        runner: Arc<dyn JobRunner>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            queue,
            runner,
            poll_interval: DEFAULT_POLL_INTERVAL,
            cancel,
        }
    }

    /// Override the poll interval. Test convenience; production should
    /// stick with the default.
    #[must_use]
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Drive the loop until the cancellation token fires. Idle ticks
    /// (no pending job) sleep for `poll_interval`; on cancel the sleep
    /// wakes immediately so shutdown is responsive.
    pub async fn run(self) {
        info!(
            poll_interval_secs = self.poll_interval.as_secs(),
            "WorkerLoop starting",
        );
        loop {
            if self.cancel.is_cancelled() {
                info!("WorkerLoop received cancel; exiting");
                return;
            }
            match self.queue.claim_next().await {
                Ok(Some(job)) => {
                    info!(
                        job_id = %job.id,
                        corpus_id = %job.corpus_id,
                        "WorkerLoop claimed job",
                    );
                    self.execute_job(&job).await;
                }
                Ok(None) => {
                    debug!("no pending jobs; sleeping");
                    self.sleep_or_cancel().await;
                }
                Err(e) => {
                    // Transient — log and back off. Don't crash the
                    // loop; a Postgres blip shouldn't take down the
                    // worker.
                    warn!(error = %e, "WorkerLoop claim_next failed; backing off");
                    self.sleep_or_cancel().await;
                }
            }
        }
    }

    async fn execute_job(&self, job: &Job) {
        let outcome = self.runner.run(job).await;
        let (status, error) = match outcome {
            Ok(()) => {
                info!(job_id = %job.id, "WorkerLoop completed job");
                (JobStatus::Completed, None)
            }
            Err(reason) => {
                warn!(
                    job_id = %job.id,
                    error = %reason,
                    "WorkerLoop job failed",
                );
                (JobStatus::Failed, Some(reason))
            }
        };
        if let Err(e) = self.queue.finish(&job.id, status, error).await {
            // The job ran (success or fail), but we couldn't write the
            // terminal status. PHASE4 chunk 2's reclaim_orphans path
            // catches this: the row stays `running` past `claimed_at +
            // timeout`, gets reclaimed, runs again. Log loudly so the
            // operator notices the duplicate work risk.
            warn!(
                job_id = %job.id,
                error = %e,
                "WorkerLoop could not write terminal status; row will be reclaimed",
            );
        }
    }

    async fn sleep_or_cancel(&self) {
        tokio::select! {
            () = self.cancel.cancelled() => {}
            () = tokio::time::sleep(self.poll_interval) => {}
        }
    }
}

/// Production [`JobRunner`] that builds a per-job
/// [`InfrastructureContext`](crate::infra::InfrastructureContext) and
/// drives `run_corpus_ingestion`. Holds the workspace config + model
/// settings + optional blob backend; cheap to clone (everything is
/// `Arc`'d or `Copy`'d).
///
/// The body is the same logic the deleted `cmd_indexer_worker`
/// contained — lifted here so the long-lived [`WorkerLoop`] can run it
/// in a loop instead of one-shot per ACA Job replica.
#[derive(Clone)]
pub struct IngestionRunner {
    pub config: Arc<MinistrConfig>,
    pub resolved_model: Arc<str>,
    pub resolved_dimension: Option<usize>,
    pub rerank_depth: Option<usize>,
    pub blob_backend: Option<Arc<ministr_cloud::BlobBackend>>,
    pub queue: Arc<JobQueueBackend>,
}

impl IngestionRunner {
    async fn execute(&self, job: &Job) -> Result<(), String> {
        let sources = match resolve_sources(&job.trigger) {
            Ok(s) => s,
            Err(reason) => return Err(reason),
        };

        info!(
            job_id = %job.id,
            corpus_id = %job.corpus_id,
            source_count = sources.len(),
            "IngestionRunner starting",
        );

        // Build infrastructure scoped to this job. The on-disk
        // `corpus_dir` is hashed from `sources`
        // (`init_infrastructure`'s local convention); the blob upload
        // below uses the job's deterministic corpus_id so the serve
        // pod's lookup matches.
        let ctx = crate::infra::init_infrastructure(
            &sources,
            &self.config,
            Some(self.resolved_model.as_ref()),
            self.resolved_dimension,
            self.rerank_depth,
        )
        .await
        .map_err(|e| format!("init_infrastructure: {e}"))?;

        let progress = Arc::new(ministr_core::ingestion::IngestionProgress::new());

        // PHASE3 fix B (preserved verbatim, ported from cmd_indexer_worker):
        // the 500ms reporter polls IngestionProgress and writes JobProgress
        // into the queue so the serve pod's SSE shows real per-file +
        // per-batch numbers. PHASE5 chunk 3 added the embedding-side fields.
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let reporter = {
            use ministr_mcp::admin::jobs::JobProgress;
            let queue = Arc::clone(&self.queue);
            let job_id = job.id.clone();
            let progress = Arc::clone(&progress);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(500));
                let mut cancel_rx = cancel_rx;
                loop {
                    tokio::select! {
                        _ = &mut cancel_rx => break,
                        _ = interval.tick() => {
                            let current = progress.current_file();
                            let snap = JobProgress {
                                stage: progress.phase().as_str().to_string(),
                                total_files: progress.files_total() as u64,
                                processed_files: progress.files_done() as u64,
                                current_file: if current.is_empty() {
                                    None
                                } else {
                                    Some(current)
                                },
                                sections_done: progress.sections_done() as u64,
                                embeddings_total: progress.embeddings_total() as u64,
                                embeddings_done: progress.embeddings_done() as u64,
                            };
                            if let Err(e) = queue.update_progress(&job_id, snap).await {
                                warn!(
                                    job_id = %job_id,
                                    error = %e,
                                    "update_progress failed; ingestion continues",
                                );
                            }
                        }
                    }
                }
            })
        };

        // PHASE4 chunk 4 streaming opt-in: persist HNSW every 4 files.
        let ingest_result =
            crate::ingestion::run_corpus_ingestion(&sources, &[], &ctx, &progress, Some(4)).await;

        let _ = cancel_tx.send(());
        let _ = reporter.await;

        ingest_result.map_err(|e| format!("ingestion failed: {e}"))?;

        // Upload the bundle under the job's deterministic corpus_id
        // (not `ctx.corpus_dir`'s hashed name). Serve pod looks up
        // `corpora/<job.corpus_id>/manifest.json` against the same id.
        if ctx.index.is_empty() {
            info!(
                corpus_id = %job.corpus_id,
                "no vectors after ingestion — skipping bundle upload",
            );
        } else if let Some(backend) = &self.blob_backend {
            let manifest = ministr_cloud::build_manifest_from_corpus_dir(
                &ctx.corpus_dir,
                self.resolved_model.as_ref(),
            )
            .await
            .map_err(|e| format!("manifest build failed: {e}"))?;
            let version = backend
                .upload_corpus(&job.corpus_id, &ctx.corpus_dir, &manifest)
                .await
                .map_err(|e| format!("blob upload failed: {e}"))?;
            info!(
                corpus_id = %job.corpus_id,
                version,
                "uploaded bundle to blob",
            );
        } else {
            info!("no blob backend configured — corpus indexed locally but not uploaded");
        }

        Ok(())
    }
}

impl JobRunner for IngestionRunner {
    fn run<'a>(
        &'a self,
        job: &'a Job,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(self.execute(job))
    }
}

/// Resolve the corpus source list from a job's trigger. Errors are
/// returned as `Err(reason)` so the [`WorkerLoop`] surfaces them as
/// `JobStatus::Failed` with the message in the row.
fn resolve_sources(trigger: &JobTrigger) -> Result<Vec<String>, String> {
    match trigger {
        JobTrigger::Tenant { paths, clone_url } => {
            // `clone_url` wins when present — it identifies a remote
            // source; `paths` describes local-mount paths. Both being
            // present is unexpected but `clone_url` wins.
            if let Some(url) = clone_url {
                Ok(vec![url.clone()])
            } else if !paths.is_empty() {
                Ok(paths.clone())
            } else {
                Err("tenant trigger has neither paths nor clone_url".to_string())
            }
        }
        other => Err(format!("unsupported trigger in WorkerLoop: {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    //! In-memory tests against the `JobQueueBackend::InMemory` variant.
    //! Confirms: cancel-before-claim exits cleanly; one job is claimed,
    //! run, and finished as Completed when the runner succeeds; a
    //! failing runner marks the job Failed.

    use super::*;
    use ministr_mcp::admin::jobs::{JobQueueBackend, JobTrigger, InMemoryJobQueue};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Test fake. Tracks how many times it was invoked and returns the
    /// outcome the test pre-loaded into `outcome`.
    struct FakeRunner {
        calls: AtomicUsize,
        outcome: std::sync::Mutex<Vec<Result<(), String>>>,
    }

    impl FakeRunner {
        fn new(outcomes: Vec<Result<(), String>>) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                outcome: std::sync::Mutex::new(outcomes),
            })
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl JobRunner for FakeRunner {
        fn run<'a>(
            &'a self,
            _job: &'a Job,
        ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let mut q = self.outcome.lock().unwrap();
                if q.is_empty() {
                    return Ok(());
                }
                q.remove(0)
            })
        }
    }

    fn fresh_queue() -> Arc<JobQueueBackend> {
        Arc::new(JobQueueBackend::InMemory(InMemoryJobQueue::default()))
    }

    fn tenant_trigger() -> JobTrigger {
        JobTrigger::Tenant {
            paths: vec!["/tmp/fake".to_string()],
            clone_url: None,
        }
    }

    #[tokio::test]
    async fn cancel_before_claim_exits_clean() {
        let queue = fresh_queue();
        let runner = FakeRunner::new(vec![]);
        let cancel = CancellationToken::new();

        let loop_handle = {
            let cancel = cancel.clone();
            let q = Arc::clone(&queue);
            let r: Arc<dyn JobRunner> = Arc::clone(&runner) as _;
            tokio::spawn(async move {
                WorkerLoop::new(q, r, cancel)
                    .with_poll_interval(Duration::from_millis(50))
                    .run()
                    .await;
            })
        };

        // Cancel before any job exists.
        cancel.cancel();
        // The loop should wake from sleep + exit.
        tokio::time::timeout(Duration::from_secs(2), loop_handle)
            .await
            .expect("WorkerLoop did not exit after cancel")
            .expect("WorkerLoop task panicked");

        assert_eq!(runner.call_count(), 0, "runner should not have fired");
    }

    #[tokio::test]
    async fn claims_runs_and_finishes_completed() {
        let queue = fresh_queue();
        let runner = FakeRunner::new(vec![Ok(())]);
        let cancel = CancellationToken::new();

        // Enqueue one job BEFORE starting the loop.
        let job = queue
            .enqueue("corpus-a".into(), tenant_trigger(), 0)
            .await
            .expect("enqueue");

        let loop_handle = {
            let cancel = cancel.clone();
            let q = Arc::clone(&queue);
            let r: Arc<dyn JobRunner> = Arc::clone(&runner) as _;
            tokio::spawn(async move {
                WorkerLoop::new(q, r, cancel)
                    .with_poll_interval(Duration::from_millis(50))
                    .run()
                    .await;
            })
        };

        // Poll until the job is terminal, then cancel.
        let final_job = wait_for_terminal(&queue, &job.id, Duration::from_secs(5)).await;
        cancel.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), loop_handle).await;

        assert_eq!(runner.call_count(), 1, "runner fired once");
        assert_eq!(final_job.status, JobStatus::Completed);
        assert!(final_job.error.is_none(), "no error on success");
    }

    #[tokio::test]
    async fn runner_error_marks_job_failed() {
        let queue = fresh_queue();
        let runner = FakeRunner::new(vec![Err("simulated failure".into())]);
        let cancel = CancellationToken::new();

        let job = queue
            .enqueue("corpus-b".into(), tenant_trigger(), 0)
            .await
            .expect("enqueue");

        let loop_handle = {
            let cancel = cancel.clone();
            let q = Arc::clone(&queue);
            let r: Arc<dyn JobRunner> = Arc::clone(&runner) as _;
            tokio::spawn(async move {
                WorkerLoop::new(q, r, cancel)
                    .with_poll_interval(Duration::from_millis(50))
                    .run()
                    .await;
            })
        };

        let final_job = wait_for_terminal(&queue, &job.id, Duration::from_secs(5)).await;
        cancel.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), loop_handle).await;

        assert_eq!(final_job.status, JobStatus::Failed);
        assert_eq!(final_job.error.as_deref(), Some("simulated failure"));
    }

    #[tokio::test]
    async fn drains_multiple_pending_jobs_in_sequence() {
        let queue = fresh_queue();
        let runner = FakeRunner::new(vec![Ok(()), Ok(()), Ok(())]);
        let cancel = CancellationToken::new();

        let mut ids = Vec::new();
        for i in 0..3 {
            let j = queue
                .enqueue(format!("corpus-{i}"), tenant_trigger(), 0)
                .await
                .expect("enqueue");
            ids.push(j.id);
        }

        let loop_handle = {
            let cancel = cancel.clone();
            let q = Arc::clone(&queue);
            let r: Arc<dyn JobRunner> = Arc::clone(&runner) as _;
            tokio::spawn(async move {
                WorkerLoop::new(q, r, cancel)
                    .with_poll_interval(Duration::from_millis(20))
                    .run()
                    .await;
            })
        };

        // All three should land Completed.
        for id in &ids {
            let final_job = wait_for_terminal(&queue, id, Duration::from_secs(5)).await;
            assert_eq!(final_job.status, JobStatus::Completed);
        }
        cancel.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), loop_handle).await;

        assert_eq!(runner.call_count(), 3);
    }

    async fn wait_for_terminal(queue: &JobQueueBackend, job_id: &str, timeout: Duration) -> Job {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let job = queue
                .get(job_id)
                .await
                .expect("get")
                .expect("job exists");
            if matches!(job.status, JobStatus::Completed | JobStatus::Failed) {
                return job;
            }
            assert!(
                std::time::Instant::now() <= deadline,
                "job {job_id} did not reach terminal status within timeout",
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}
