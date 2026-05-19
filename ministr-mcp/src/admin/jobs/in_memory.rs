//! Process-local job queue, used for tests and single-container deployments.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::super::ids::new_job_id;
use crate::time::epoch_now;
use super::{Job, JobProgress, JobQueue, JobQueueError, JobResult, JobStatus, JobTrigger};

#[derive(Debug, Clone, Default)]
pub(crate) struct InMemoryJobQueue {
    jobs: Arc<RwLock<HashMap<String, Job>>>,
}

impl InMemoryJobQueue {
    #[allow(dead_code)] // selected once the admin router is wired
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl JobQueue for InMemoryJobQueue {
    async fn enqueue(&self, corpus_id: String, trigger: JobTrigger) -> JobResult<Job> {
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
        };
        self.jobs.write().await.insert(job.id.clone(), job.clone());
        Ok(job)
    }

    async fn get(&self, job_id: &str) -> JobResult<Option<Job>> {
        Ok(self.jobs.read().await.get(job_id).cloned())
    }

    async fn claim_next(&self) -> JobResult<Option<Job>> {
        let mut jobs = self.jobs.write().await;
        let next_id = jobs
            .values()
            .filter(|j| j.status == JobStatus::Pending)
            .min_by_key(|j| j.created_at)
            .map(|j| j.id.clone());
        match next_id {
            Some(id) => {
                if let Some(job) = jobs.get_mut(&id) {
                    job.status = JobStatus::Running;
                    job.updated_at = epoch_now();
                    Ok(Some(job.clone()))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    async fn update_progress(&self, job_id: &str, progress: JobProgress) -> JobResult<()> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| JobQueueError::NotFound(job_id.to_owned()))?;
        job.progress = progress;
        job.updated_at = epoch_now();
        Ok(())
    }

    async fn finish(
        &self,
        job_id: &str,
        status: JobStatus,
        error: Option<String>,
    ) -> JobResult<()> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(job_id)
            .ok_or_else(|| JobQueueError::NotFound(job_id.to_owned()))?;
        job.status = status;
        job.error = error;
        job.updated_at = epoch_now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enqueue_and_get() {
        let q = InMemoryJobQueue::new();
        let job = q.enqueue("corpus-a".into(), JobTrigger::Manual).await.unwrap();
        let got = q.get(&job.id).await.unwrap().unwrap();
        assert_eq!(got.corpus_id, "corpus-a");
        assert_eq!(got.status, JobStatus::Pending);
    }

    #[tokio::test]
    async fn claim_next_is_fifo_and_transitions_to_running() {
        let q = InMemoryJobQueue::new();
        let a = q.enqueue("a".into(), JobTrigger::Manual).await.unwrap();
        // Force distinct created_at values: sleep 1s would slow tests; instead
        // adjust b's created_at after enqueue to simulate a later submission.
        let b = q.enqueue("b".into(), JobTrigger::Manual).await.unwrap();
        {
            let mut jobs = q.jobs.write().await;
            jobs.get_mut(&b.id).unwrap().created_at = a.created_at + 1;
        }

        let claimed = q.claim_next().await.unwrap().unwrap();
        assert_eq!(claimed.id, a.id);
        assert_eq!(claimed.status, JobStatus::Running);

        // No more Pending jobs but b is also Pending and should be next.
        let next = q.claim_next().await.unwrap().unwrap();
        assert_eq!(next.id, b.id);

        // Now no Pending jobs left.
        assert!(q.claim_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn finish_records_status_and_error() {
        let q = InMemoryJobQueue::new();
        let job = q.enqueue("c".into(), JobTrigger::Manual).await.unwrap();
        q.finish(&job.id, JobStatus::Failed, Some("boom".into()))
            .await
            .unwrap();
        let got = q.get(&job.id).await.unwrap().unwrap();
        assert_eq!(got.status, JobStatus::Failed);
        assert_eq!(got.error.as_deref(), Some("boom"));
    }
}
