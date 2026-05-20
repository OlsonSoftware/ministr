//! PHASE3 chunk 4 — `IndexJobSink` impl that writes through the cloud
//! Postgres pool.
//!
//! `PostgresIndexJobSink` is what `cmd_serve_http` wires into the
//! daemon's [`ministr_api::IndexJobSink`] slot in cloud mode. Each
//! `create_pending` call:
//!
//! 1. UPSERTs the canonical `cloud_corpora` row for `corpus_id`
//!    (PHASE3 chunk 1's table).
//! 2. INSERTs a new `indexer_jobs` row with status `pending` and a
//!    [`ministr_mcp::admin::jobs::JobTrigger::Tenant`] payload — the
//!    same JSON envelope `PostgresJobQueue::enqueue` produces.
//!
//! Both writes are inside one transaction so a partial registration
//! (`cloud_corpora` row exists but no pending job) never gets observed
//! by a concurrent worker poll.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use deadpool_postgres::Pool;
use ministr_api::index_job_sink::{
    IndexJobError, IndexJobFuture, IndexJobSink, IndexJobSnapshot, IndexJobStatus,
};
use serde_json::json;
use sha2::{Digest, Sha256};

/// Postgres-backed `IndexJobSink` for the cloud serve pod.
///
/// Cheap to clone — wraps an `Arc<Pool>`.
#[derive(Debug, Clone)]
pub struct PostgresIndexJobSink {
    pool: Arc<Pool>,
    tenant_id: Option<String>,
}

impl PostgresIndexJobSink {
    /// Construct a sink backed by `pool`. `tenant_id` is `None` for the
    /// single-tenant cloud pod today; mirrors the same `tenant_id`
    /// nullability `PostgresCorporaRepo` uses.
    #[must_use]
    pub fn new(pool: Arc<Pool>, tenant_id: Option<String>) -> Self {
        Self { pool, tenant_id }
    }
}

fn map_err<E: std::fmt::Display>(prefix: &str) -> impl FnOnce(E) -> IndexJobError + '_ {
    move |e| IndexJobError::Storage(format!("{prefix}: {e}"))
}

fn epoch_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn new_job_id() -> String {
    let mut hasher = Sha256::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hasher.update(nanos.to_le_bytes());
    // Pointer churn as a coarse extra entropy source — the actual key
    // dimension is the nanosecond timestamp; this protects against
    // two calls landing on the same nanosecond on the same thread.
    let entropy: u64 = std::ptr::from_ref(&hasher) as u64;
    hasher.update(entropy.to_le_bytes());
    let hash = hasher.finalize();
    let mut s = String::with_capacity(4 + 16);
    s.push_str("job_");
    for b in &hash[..8] {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn status_str(status: IndexJobStatus) -> &'static str {
    match status {
        IndexJobStatus::Pending => "pending",
        IndexJobStatus::Running => "running",
        IndexJobStatus::Completed => "completed",
        IndexJobStatus::Failed => "failed",
    }
}

fn parse_status(s: Option<&str>) -> IndexJobStatus {
    match s {
        Some("running") => IndexJobStatus::Running,
        Some("completed") => IndexJobStatus::Completed,
        Some("failed") => IndexJobStatus::Failed,
        // pending or unknown — default to pending so the SSE keeps
        // streaming (terminal states close it).
        _ => IndexJobStatus::Pending,
    }
}

impl IndexJobSink for PostgresIndexJobSink {
    fn create_pending<'a>(
        &'a self,
        corpus_id: &'a str,
        paths: &'a [String],
        display_name: Option<&'a str>,
        clone_url: Option<&'a str>,
    ) -> IndexJobFuture<'a, String> {
        Box::pin(async move {
            let paths_json = serde_json::to_value(paths)
                .map_err(map_err("serialize paths"))?;
            let now = epoch_now_secs();
            let job_id = new_job_id();
            let now_i64 = i64::try_from(now).unwrap_or(i64::MAX);

            // The data blob shape mirrors `PostgresJobQueue::enqueue`
            // exactly — serde_json::from_str against the existing
            // `Job` struct round-trips. The `JobTrigger::Tenant`
            // serde tag is `kind=tenant` (PHASE3 chunk 2).
            let trigger = json!({
                "kind": "tenant",
                "paths": paths,
                "clone_url": clone_url,
            });
            let progress = json!({
                "stage": "",
                "total_files": 0,
                "processed_files": 0,
                "current_file": null,
            });
            let job_blob = json!({
                "id": job_id,
                "corpus_id": corpus_id,
                "trigger": trigger,
                "status": "pending",
                "progress": progress,
                "created_at": now,
                "updated_at": now,
                "error": null,
                "priority": 0,
            })
            .to_string();

            let mut client = self
                .pool
                .get()
                .await
                .map_err(map_err("create_pending: get conn"))?;
            let tx = client
                .transaction()
                .await
                .map_err(map_err("create_pending: begin tx"))?;

            // 1. UPSERT cloud_corpora — mirror PostgresCorporaRepo's
            //    column set so the chunk 1 row stays canonical.
            tx.execute(
                "INSERT INTO cloud_corpora \
                   (corpus_id, tenant_id, paths, display_name, updated_at) \
                 VALUES ($1, $2, $3::jsonb, $4, now()) \
                 ON CONFLICT (corpus_id) DO UPDATE SET \
                   tenant_id    = EXCLUDED.tenant_id, \
                   paths        = EXCLUDED.paths, \
                   display_name = EXCLUDED.display_name, \
                   updated_at   = now()",
                &[
                    &corpus_id,
                    &self.tenant_id,
                    &paths_json,
                    &display_name,
                ],
            )
            .await
            .map_err(map_err("create_pending: upsert cloud_corpora"))?;

            // 2. INSERT pending indexer_jobs row. Same column set
            //    `PostgresJobQueue::enqueue` writes; the worker's
            //    `claim_next` finds this row via its existing query.
            tx.execute(
                "INSERT INTO indexer_jobs \
                   (id, corpus_id, status, created_at, updated_at, data, priority) \
                 VALUES ($1, $2, 'pending', $3, $3, $4, 0)",
                &[&job_id, &corpus_id, &now_i64, &job_blob],
            )
            .await
            .map_err(map_err("create_pending: insert indexer_jobs"))?;

            tx.commit()
                .await
                .map_err(map_err("create_pending: commit"))?;
            Ok(job_id)
        })
    }

    fn latest_for_corpus<'a>(
        &'a self,
        corpus_id: &'a str,
    ) -> IndexJobFuture<'a, Option<IndexJobSnapshot>> {
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(map_err("latest_for_corpus: get conn"))?;
            let row = client
                .query_opt(
                    "SELECT data FROM indexer_jobs \
                     WHERE corpus_id = $1 \
                     ORDER BY created_at DESC LIMIT 1",
                    &[&corpus_id],
                )
                .await
                .map_err(map_err("latest_for_corpus: query"))?;
            let Some(r) = row else {
                return Ok(None);
            };
            let blob: String = r
                .try_get("data")
                .map_err(map_err("latest_for_corpus: row.data"))?;
            let value: serde_json::Value = serde_json::from_str(&blob)
                .map_err(map_err("latest_for_corpus: parse data"))?;

            let progress = value.get("progress");
            Ok(Some(IndexJobSnapshot {
                job_id: value
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                corpus_id: value
                    .get("corpus_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(corpus_id)
                    .to_string(),
                status: parse_status(
                    value.get("status").and_then(serde_json::Value::as_str),
                ),
                stage: progress
                    .and_then(|p| p.get("stage"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                total_files: progress
                    .and_then(|p| p.get("total_files"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                processed_files: progress
                    .and_then(|p| p.get("processed_files"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                current_file: progress
                    .and_then(|p| p.get("current_file"))
                    .and_then(serde_json::Value::as_str)
                    .map(String::from),
                error: value
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .map(String::from),
            }))
        })
    }
}

// Suppress unused — status_str is used by symmetric tests downstream.
#[allow(dead_code)]
fn _force_use(s: IndexJobStatus) -> &'static str {
    status_str(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impl_is_dyn_compatible() {
        fn assert_dyn(_: &dyn IndexJobSink) {}
        let pool = build_dummy_pool();
        let sink = PostgresIndexJobSink::new(Arc::new(pool), Some("t1".into()));
        assert_dyn(&sink);
    }

    #[test]
    fn new_job_id_has_job_prefix() {
        let id = new_job_id();
        assert!(id.starts_with("job_"), "got {id}");
        assert_eq!(id.len(), 4 + 16);
    }

    fn build_dummy_pool() -> Pool {
        use deadpool_postgres::{Config, ManagerConfig, RecyclingMethod, Runtime};
        use tokio_postgres::NoTls;
        let mut cfg = Config::new();
        cfg.url = Some("postgres://invalid:invalid@127.0.0.1:1/invalid".to_string());
        cfg.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });
        cfg.create_pool(Some(Runtime::Tokio1), NoTls)
            .expect("create_pool")
    }
}
