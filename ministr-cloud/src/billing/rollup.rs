//! Daily usage aggregator (F1.4 sub-bullet 3).
//!
//! [`rollup_day`] pre-aggregates one day's `usage_events` into the
//! `usage_rollups` table. The aggregator runs as a nightly Container
//! Apps Job (provisioned opt-in via Pulumi config — separate landing)
//! that calls [`rollup_day`] with `offset = -1` at 01:00 UTC; the
//! `/api/v1/billing/usage` endpoint reads only from `usage_rollups`,
//! never scans raw events.
//!
//! # Idempotence
//!
//! The function is an `INSERT ... SELECT ... GROUP BY ... ON CONFLICT
//! DO UPDATE`, so re-running it for the same day always replaces the
//! existing totals with the freshly-computed values. Three behaviours
//! follow:
//!
//! 1. **Cron retries are safe.** A failed run that partially wrote
//!    rows can be re-fired with the same `offset`; the second run
//!    converges the table to the correct state.
//! 2. **Mid-day partial rollups work.** Callers can run
//!    `rollup_day(0)` from a manual ops endpoint to refresh "today,
//!    so far" without coordinating with the nightly job.
//! 3. **Late-arriving events are absorbed.** If an event lands on
//!    day N after the nightly rollup, the next morning's rerun of
//!    `rollup_day(-1)` (or an ops re-run) picks it up.

use deadpool_postgres::Pool;

use crate::db::DbError;

/// Roll up `usage_events` for the day at `day_offset` (0 = today,
/// `-1` = yesterday in UTC) into `usage_rollups`.
///
/// Returns the number of `(tenant_id, kind)` combinations that were
/// upserted — useful for the cron's log line ("rolled up N tenant×kind
/// combinations for 2026-05-18").
///
/// # Errors
///
/// [`DbError::GetConn`] when the pool cannot hand out a connection;
/// [`DbError::Sql`] for any statement-level failure.
pub async fn rollup_day(pool: &Pool, day_offset: i32) -> Result<u64, DbError> {
    let client = pool
        .get()
        .await
        .map_err(|e| DbError::GetConn(format!("rollup_day: {e}")))?;
    let affected = client
        .execute(
            "INSERT INTO usage_rollups (day, tenant_id, kind, total, rolled_up_at)
             SELECT
                 (CURRENT_DATE + ($1::int))::date AS day,
                 tenant_id,
                 kind,
                 COALESCE(SUM(count), 0)::bigint AS total,
                 now()
             FROM usage_events
             WHERE ts >= (CURRENT_DATE + ($1::int))::timestamptz
               AND ts <  (CURRENT_DATE + ($1::int) + 1)::timestamptz
             GROUP BY tenant_id, kind
             ON CONFLICT (day, tenant_id, kind) DO UPDATE
                 SET total        = EXCLUDED.total,
                     rolled_up_at = EXCLUDED.rolled_up_at",
            &[&day_offset],
        )
        .await
        .map_err(|e| DbError::Sql(format!("rollup_day insert: {e}")))?;
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::billing::usage::{record_usage, UsageEventKind};
    use crate::db::{connect, run_migrations};

    async fn seed_user(pool: &Pool, suffix: &str) -> String {
        let client = pool.get().await.unwrap();
        client
            .query_one(
                "INSERT INTO users (email, plan_id)
                 VALUES ($1, $2)
                 RETURNING id::text",
                &[&format!("rollup-{suffix}@test"), &"pro"],
            )
            .await
            .unwrap()
            .get("id")
    }

    /// Helper for the gated tests: read back the rolled-up total for
    /// (today, tenant, kind).
    async fn rollup_total(pool: &Pool, tenant: &str, kind: &str) -> Option<i64> {
        let client = pool.get().await.unwrap();
        client
            .query_opt(
                "SELECT total FROM usage_rollups
                 WHERE day = CURRENT_DATE AND tenant_id = $1::uuid AND kind = $2",
                &[&tenant, &kind],
            )
            .await
            .unwrap()
            .map(|row| row.get::<_, i64>("total"))
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn rollup_aggregates_todays_events() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("connect");
        run_migrations(&pool).await.expect("migrate");

        let tenant = seed_user(&pool, &format!("agg-{}", std::process::id())).await;
        record_usage(&pool, &tenant, UsageEventKind::QueryServed, 3)
            .await
            .unwrap();
        record_usage(&pool, &tenant, UsageEventKind::QueryServed, 4)
            .await
            .unwrap();
        record_usage(&pool, &tenant, UsageEventKind::IndexMinutes, 2)
            .await
            .unwrap();

        let n = rollup_day(&pool, 0).await.unwrap();
        // Two (tenant, kind) combinations for our tenant — plus any
        // from concurrent test rows on shared CI databases. The strict
        // check is on the totals below.
        assert!(n >= 2, "expected at least 2 rolled-up combos, got {n}");

        assert_eq!(
            rollup_total(&pool, &tenant, "query.served").await,
            Some(7)
        );
        assert_eq!(
            rollup_total(&pool, &tenant, "index.minutes").await,
            Some(2)
        );
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn rerunning_rollup_replaces_totals_in_place() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("connect");
        run_migrations(&pool).await.expect("migrate");

        let tenant = seed_user(&pool, &format!("idem-{}", std::process::id())).await;
        record_usage(&pool, &tenant, UsageEventKind::QueryServed, 5)
            .await
            .unwrap();
        rollup_day(&pool, 0).await.unwrap();
        assert_eq!(rollup_total(&pool, &tenant, "query.served").await, Some(5));

        // Add more events and re-run; the rollup row updates in place
        // rather than duplicating.
        record_usage(&pool, &tenant, UsageEventKind::QueryServed, 11)
            .await
            .unwrap();
        rollup_day(&pool, 0).await.unwrap();
        assert_eq!(rollup_total(&pool, &tenant, "query.served").await, Some(16));

        // Row count guard — exactly one row per (day, tenant, kind).
        let client = pool.get().await.unwrap();
        let count: i64 = client
            .query_one(
                "SELECT COUNT(*)::bigint AS n FROM usage_rollups
                 WHERE day = CURRENT_DATE AND tenant_id = $1::uuid AND kind = 'query.served'",
                &[&tenant],
            )
            .await
            .unwrap()
            .get("n");
        assert_eq!(count, 1, "rollup must keep exactly one row per key");
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn empty_day_produces_no_rows() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("connect");
        run_migrations(&pool).await.expect("migrate");

        // Roll up a far-future day with no events. Returns 0 affected
        // rows; nothing is inserted.
        let n = rollup_day(&pool, 30_000).await.unwrap();
        assert_eq!(n, 0);
    }
}
