//! `usage_events` write path (F1.4 sub-bullet 1).
//!
//! Every billable operation lands a row in `usage_events`. The daily
//! aggregator (F1.4 sub-bullet 3, future module) rolls those into
//! `usage_rollups`; the billing UI (F1.4 sub-bullet 4) reads the
//! rollups. Direct event writes also feed Stripe Meter events
//! (F1.5).
//!
//! # Kinds (canonical strings)
//!
//! Mirrors ROADMAP §4 F1.4 verbatim:
//! - `corpus.indexed` — count of corpora freshly indexed.
//! - `index.minutes` — cumulative CPU-minutes spent indexing.
//! - `query.served` — count of `ministr_survey` / read-path tool calls.
//! - `atlas.queries` — count of `/atlas/*` reads (separate quota path).
//!
//! These strings are stable: rollup queries `GROUP BY kind`, billing
//! invoices key off them, and the F2.3 quota middleware compares them
//! against per-plan caps. Adding a kind = adding a variant here AND
//! teaching the rollup query.

use std::str::FromStr;

use deadpool_postgres::Pool;

use crate::db::DbError;

/// Billable activity kinds the cloud writes into `usage_events`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UsageEventKind {
    /// One additional corpus has been (re-)indexed end-to-end.
    CorpusIndexed,
    /// Indexing CPU-minutes consumed since the last write (count =
    /// minutes; the caller is responsible for batching).
    IndexMinutes,
    /// A `ministr_survey` / `ministr_read` / similar query was served.
    QueryServed,
    /// A query against an Atlas corpus (separate quota path from
    /// `query.served`).
    AtlasQueries,
}

impl UsageEventKind {
    /// Canonical, stable wire string for this kind. Used for the
    /// `usage_events.kind` column, Stripe Meter event names, and the
    /// billing-UI breakdown labels.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CorpusIndexed => "corpus.indexed",
            Self::IndexMinutes => "index.minutes",
            Self::QueryServed => "query.served",
            Self::AtlasQueries => "atlas.queries",
        }
    }

}

/// Sentinel for an unrecognised `usage_events.kind` string. Returned
/// by [`UsageEventKind::from_str`]; the rollup query treats unknown
/// rows as quarantined rather than panicking.
#[derive(Debug, thiserror::Error)]
#[error("unknown usage event kind: {0}")]
pub struct UnknownKind(pub String);

impl FromStr for UsageEventKind {
    type Err = UnknownKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "corpus.indexed" => Ok(Self::CorpusIndexed),
            "index.minutes" => Ok(Self::IndexMinutes),
            "query.served" => Ok(Self::QueryServed),
            "atlas.queries" => Ok(Self::AtlasQueries),
            other => Err(UnknownKind(other.to_owned())),
        }
    }
}

/// Write a single billable usage event for `tenant_id`.
///
/// `tenant_id` is the UUID string of either a user (personal Pro) or
/// an org (Team/Enterprise) — the polymorphic owner side of F1.2's
/// schema. The runtime resolver in `ministr-mcp::auth::middleware`
/// guarantees the value points at a live row.
///
/// `count` is treated as additive: each call appends a row rather
/// than upserting, so the daily aggregator can sum across an
/// unbounded number of events without contention. `count` is allowed
/// to exceed 1 for kinds like `index.minutes` where the caller has
/// already batched a window.
///
/// # Errors
///
/// Returns [`DbError::GetConn`] if a pooled connection can't be
/// acquired, [`DbError::Sql`] if the insert fails (e.g. `tenant_id`
/// is not a valid UUID).
pub async fn record_usage(
    pool: &Pool,
    tenant_id: &str,
    kind: UsageEventKind,
    count: i64,
) -> Result<(), DbError> {
    let client = pool
        .get()
        .await
        .map_err(|e| DbError::GetConn(format!("record_usage: {e}")))?;
    client
        .execute(
            "INSERT INTO usage_events (tenant_id, kind, count)
             VALUES ($1::text::uuid, $2, $3)",
            &[&tenant_id, &kind.as_str(), &count],
        )
        .await
        .map_err(|e| DbError::Sql(format!("record_usage insert: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{connect, run_migrations};

    #[test]
    fn as_str_matches_roadmap_spec() {
        assert_eq!(UsageEventKind::CorpusIndexed.as_str(), "corpus.indexed");
        assert_eq!(UsageEventKind::IndexMinutes.as_str(), "index.minutes");
        assert_eq!(UsageEventKind::QueryServed.as_str(), "query.served");
        assert_eq!(UsageEventKind::AtlasQueries.as_str(), "atlas.queries");
    }

    #[test]
    fn from_str_round_trips_every_variant() {
        for variant in [
            UsageEventKind::CorpusIndexed,
            UsageEventKind::IndexMinutes,
            UsageEventKind::QueryServed,
            UsageEventKind::AtlasQueries,
        ] {
            assert_eq!(variant.as_str().parse::<UsageEventKind>().unwrap(), variant);
        }
    }

    #[test]
    fn from_str_rejects_unknown_kinds() {
        assert!("".parse::<UsageEventKind>().is_err());
        assert!("corpus.deleted".parse::<UsageEventKind>().is_err());
        // Case-sensitive: the wire spec is lowercase.
        assert!("INDEX.MINUTES".parse::<UsageEventKind>().is_err());
    }

    /// Integration test gated on `MINISTR_TEST_PG_URL` — same pattern
    /// as the F1.2 migration tests. Skipped by default so `cargo
    /// test` stays dependency-free.
    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn record_usage_inserts_a_row() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("connect");
        run_migrations(&pool).await.expect("migrate");
        let client = pool.get().await.unwrap();

        // Seed a tenant — usage_events.tenant_id is polymorphic but
        // we materialise a real users row so the test exercises the
        // same path production hits.
        let tenant_id_row = client
            .query_one(
                "INSERT INTO users (email, plan_id)
                 VALUES ($1, $2)
                 RETURNING id::text",
                &[&format!("usage-{}@test", std::process::id()), &"pro"],
            )
            .await
            .unwrap();
        let tenant_id: String = tenant_id_row.get("id");

        record_usage(&pool, &tenant_id, UsageEventKind::QueryServed, 7)
            .await
            .unwrap();
        record_usage(&pool, &tenant_id, UsageEventKind::IndexMinutes, 3)
            .await
            .unwrap();

        let row = client
            .query_one(
                "SELECT
                     COALESCE(SUM(count) FILTER (WHERE kind = 'query.served'), 0) AS q,
                     COALESCE(SUM(count) FILTER (WHERE kind = 'index.minutes'), 0) AS m
                 FROM usage_events
                 WHERE tenant_id = $1::text::uuid",
                &[&tenant_id],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, i64>("q"), 7);
        assert_eq!(row.get::<_, i64>("m"), 3);
    }
}
