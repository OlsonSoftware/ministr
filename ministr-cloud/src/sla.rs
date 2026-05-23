//! F5.5-b-persist-write — persist the in-process `LatencyTracker`
//! snapshot to Postgres so historical p95 evidence survives pod
//! recycles.
//!
//! The in-memory rolling-window histogram in
//! `ministr-mcp/src/admin/latency.rs` is great for "right now" reads
//! at the `/sla` endpoint but evaporates at every restart. A periodic
//! flush from a tokio task captures the percentiles at the cadence
//! configured by the `MINISTR_SLA_FLUSH_SECS` env var (default 60).
//!
//! Cost / shape rationale:
//!
//! - One row per pod per flush interval. At the 60s default, ~1440
//!   rows/pod/day; ~43K rows/pod/month. Each row is `~50 bytes`, so
//!   ~2 MB/pod/month — negligible.
//! - Pure write API: this module ships only the INSERT path. The
//!   `F5.5-b-persist-read` follow-up chunk adds the query helper +
//!   `/sla.latency_window_30d` JSON field.
//! - No tenant or org scoping — latency is fleet-wide signal, not a
//!   per-customer measurement.

use std::sync::Arc;

use deadpool_postgres::Pool;
use ministr_api::{MaxP95Future, SlaWindowStore, SlaWindowStoreError};

/// Errors surfaced by [`persist_snapshot`]. Both arms are treated as
/// non-fatal by the calling tokio task — the loop logs at warn and
/// continues so a transient backend blip never wedges the metrics
/// pipeline. Surfaced as a real error type rather than a `bool`
/// so the future cleanup task (`F5.5-b-persist-retention`) can
/// distinguish "real failure" from "no row written this tick".
#[derive(Debug, thiserror::Error)]
pub enum SlaError {
    /// Could not acquire a connection from the pool.
    #[error("sla snapshot get_conn: {0}")]
    GetConn(String),
    /// The INSERT statement was rejected by the backend.
    #[error("sla snapshot insert: {0}")]
    Insert(String),
}

/// F5.5-b-persist-write — append one `request_latency_snapshots` row.
///
/// Pure async function so the caller composes its own retry, error
/// handling, and tracing semantics. Microsecond fields are taken as
/// `u32` (matching the `LatencyTracker`'s on-the-wire shape) and
/// stored as PG INTEGER via `i32` cast — clamps at `i32::MAX`
/// (~35 minutes per sample) which is comfortably above any in-pocket
/// budget.
///
/// # Errors
///
/// Returns [`SlaError::GetConn`] when the pool can't hand out a
/// connection, [`SlaError::Insert`] when the SQL statement fails.
pub async fn persist_snapshot(
    pool: &Pool,
    ts_unix: i64,
    sample_count: usize,
    p50_us: u32,
    p95_us: u32,
    p99_us: u32,
) -> Result<(), SlaError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| SlaError::GetConn(e.to_string()))?;
    // Postgres INTEGER is i32; clamp the u32/usize sources.
    let count_i32 = i32::try_from(sample_count).unwrap_or(i32::MAX);
    let p50_i32 = i32::try_from(p50_us).unwrap_or(i32::MAX);
    let p95_i32 = i32::try_from(p95_us).unwrap_or(i32::MAX);
    let p99_i32 = i32::try_from(p99_us).unwrap_or(i32::MAX);
    conn.execute(
        "INSERT INTO request_latency_snapshots
             (ts_unix, sample_count, p50_us, p95_us, p99_us)
         VALUES ($1, $2, $3, $4, $5)",
        &[&ts_unix, &count_i32, &p50_i32, &p95_i32, &p99_i32],
    )
    .await
    .map_err(|e| SlaError::Insert(e.to_string()))?;
    Ok(())
}

/// F5.5-b-persist-read — Postgres-backed implementation of
/// [`SlaWindowStore`]. Wires `request_latency_snapshots` into the
/// `/sla` handler so it can render `latency.window_30d_max_p95_ms`.
///
/// SELECT MAX hits the `idx_request_latency_snapshots_ts` DESC
/// index for a single page scan; even at the projected 30-day-per-pod
/// row count (~43K rows) the query is sub-millisecond. A future
/// caching layer (`F5.5-b-persist-cache`) could memoize the result
/// with a 60s TTL if `/sla` polling cadence ever justifies it.
#[derive(Debug, Clone)]
pub struct PostgresSlaWindowStore {
    pool: Pool,
}

impl PostgresSlaWindowStore {
    /// Build a store bound to a Postgres pool. Shares the pool with
    /// every other cloud-side reader.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Convenience: wrap as an `Arc<dyn SlaWindowStore>` for
    /// `AdminState::with_sla_window_store`.
    #[must_use]
    pub fn into_dyn(self) -> Arc<dyn SlaWindowStore> {
        Arc::new(self)
    }
}

/// F5.5-b-persist-retention — DELETE every `request_latency_snapshots`
/// row whose `ts_unix` is strictly less than `cutoff_ts_unix`. Returns
/// the number of rows deleted so the calling CLI / operator script
/// can log what happened.
///
/// Pure async function so an operator's cron / Container Apps Job
/// invokes it via the `ministr cloud sla-prune-snapshots` CLI; no
/// in-process tokio task — retention cadence is an operator policy
/// decision that shouldn't be coupled to serve restarts.
///
/// # Errors
///
/// Returns [`SlaError::GetConn`] when the pool can't hand out a
/// connection, [`SlaError::Insert`] (the existing variant covers
/// DELETE too — same backend error class) when the statement fails.
pub async fn delete_snapshots_older_than(
    pool: &Pool,
    cutoff_ts_unix: i64,
) -> Result<u64, SlaError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| SlaError::GetConn(e.to_string()))?;
    let rows = conn
        .execute(
            "DELETE FROM request_latency_snapshots WHERE ts_unix < $1",
            &[&cutoff_ts_unix],
        )
        .await
        .map_err(|e| SlaError::Insert(e.to_string()))?;
    Ok(rows)
}

impl SlaWindowStore for PostgresSlaWindowStore {
    fn max_p95_since(&self, since_ts_unix: i64) -> MaxP95Future<'_> {
        Box::pin(async move {
            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| SlaWindowStoreError::Storage(format!("get_conn: {e}")))?;
            // MAX(p95_us) over the window — single index scan via
            // idx_request_latency_snapshots_ts. Returns NULL when the
            // window is empty, which try_get surfaces as Option::None
            // through the explicit nullable cast.
            let row = conn
                .query_one(
                    "SELECT MAX(p95_us) AS max_p95 FROM request_latency_snapshots WHERE ts_unix >= $1",
                    &[&since_ts_unix],
                )
                .await
                .map_err(|e| SlaWindowStoreError::Storage(format!("query: {e}")))?;
            let max_i32: Option<i32> = row
                .try_get("max_p95")
                .map_err(|e| SlaWindowStoreError::Storage(format!("read max_p95: {e}")))?;
            // Negative values would violate the table's CHECK
            // constraint, so a clamp at 0 is purely defensive.
            Ok(max_i32.map(|v| u32::try_from(v).unwrap_or(0)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{connect, run_migrations};

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn persist_snapshot_round_trips() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("open pool");
        run_migrations(&pool).await.expect("migrate");

        // Unique ts_unix so concurrent test runs don't collide.
        let ts: i64 = i64::from(std::process::id()) + 1_700_000_000;
        persist_snapshot(&pool, ts, 100, 5_000, 50_000, 100_000)
            .await
            .expect("persist");

        let conn = pool.get().await.expect("get conn");
        let row = conn
            .query_one(
                "SELECT sample_count, p50_us, p95_us, p99_us
                   FROM request_latency_snapshots
                  WHERE ts_unix = $1",
                &[&ts],
            )
            .await
            .expect("read back");
        let count: i32 = row.get("sample_count");
        let p50: i32 = row.get("p50_us");
        let p95: i32 = row.get("p95_us");
        let p99: i32 = row.get("p99_us");
        assert_eq!(count, 100);
        assert_eq!(p50, 5_000);
        assert_eq!(p95, 50_000);
        assert_eq!(p99, 100_000);

        // Clean up so the test is rerunnable.
        let conn = pool.get().await.expect("get conn");
        conn.execute(
            "DELETE FROM request_latency_snapshots WHERE ts_unix = $1",
            &[&ts],
        )
        .await
        .expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn delete_snapshots_older_than_removes_old_rows_only() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("open pool");
        run_migrations(&pool).await.expect("migrate");

        // Two rows: one ancient, one recent. Prune above the ancient
        // ts but below the recent ts; assert only the ancient is gone.
        let old_ts: i64 = 1_500_000_000;
        let new_ts: i64 = 9_500_000_000;
        persist_snapshot(&pool, old_ts, 10, 100, 200, 300)
            .await
            .expect("persist old");
        persist_snapshot(&pool, new_ts, 10, 100, 200, 300)
            .await
            .expect("persist new");

        // Cutoff sits between the two rows.
        let cutoff = 5_000_000_000;
        let deleted = delete_snapshots_older_than(&pool, cutoff)
            .await
            .expect("prune");
        assert!(
            deleted >= 1,
            "expected at least one row deleted, got {deleted}"
        );

        // Confirm the recent row survived; cleanup it ourselves.
        let conn = pool.get().await.expect("get conn");
        let survived: i64 = conn
            .query_one(
                "SELECT count(*) FROM request_latency_snapshots WHERE ts_unix = $1",
                &[&new_ts],
            )
            .await
            .expect("count survived")
            .get(0);
        assert_eq!(survived, 1, "recent row should survive");

        let conn = pool.get().await.expect("get conn");
        conn.execute(
            "DELETE FROM request_latency_snapshots WHERE ts_unix IN ($1, $2)",
            &[&old_ts, &new_ts],
        )
        .await
        .expect("cleanup");
    }
}
