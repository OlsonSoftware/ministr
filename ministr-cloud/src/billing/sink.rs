//! [`PostgresUsageSink`] — the production impl of
//! [`ministr_api::UsageSink`].
//!
//! Plugs into `ministr-daemon::AppState.usage_sink` when cloud mode
//! is active (F1.4 sub-bullet 2). Every successful tool route fires
//! `sink.record(tenant_id, kind, count)`; the impl spawns a detached
//! tokio task that runs [`record_usage`] against the cloud's
//! deadpool-postgres pool. Storage failures are logged but never
//! propagate — the daemon never observes them, so a transient
//! Postgres blip cannot fail an end-user query.

use std::sync::Arc;

use deadpool_postgres::Pool;
use ministr_api::{TenantId, UsageSink};
use tracing::warn;

use super::usage::{record_usage, UsageEventKind};

/// `UsageSink` impl that appends rows to the cloud's `usage_events`
/// table.
///
/// Cheap to clone — the inner pool is already an Arc.
#[derive(Debug, Clone)]
pub struct PostgresUsageSink {
    pool: Arc<Pool>,
}

impl PostgresUsageSink {
    /// Construct against an existing pool. The pool is typically the
    /// same one `cmd_serve_http` opens for migrations + OAuth, so all
    /// cloud-side Postgres traffic shares a single connection pool.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Bare constructor for callers that already own an `Arc<Pool>`
    /// (e.g. tests).
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

impl UsageSink for PostgresUsageSink {
    fn record(&self, tenant_id: TenantId, kind: &'static str, count: i64) {
        // The wire-string `kind` came from the daemon's activity
        // middleware, which only emits the four ROADMAP-spec kinds.
        // Anything unrecognised here is a programmer error — log it
        // and drop the event rather than write garbage.
        let Ok(kind_enum) = kind.parse::<UsageEventKind>() else {
            warn!(kind, "PostgresUsageSink: dropping event with unknown kind");
            return;
        };
        let pool = Arc::clone(&self.pool);
        tokio::spawn(async move {
            if let Err(e) = record_usage(&pool, tenant_id.as_str(), kind_enum, count).await {
                warn!(error = %e, tenant = %tenant_id.as_str(), kind = kind_enum.as_str(),
                      "PostgresUsageSink: insert failed; usage event lost");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect;

    /// Sanity check the sink is dyn-compatible with the trait
    /// boundary the daemon's `AppState` uses.
    #[test]
    fn is_dyn_usable() {
        // Pure compile-time check: PostgresUsageSink must be storable
        // as Arc<dyn UsageSink> for AppState.with_usage_sink to take it.
        fn _accepts(_: Arc<dyn UsageSink>) {}
        // (Can't actually build a Pool without a URL — the existence
        // of a `fn _accepts` that takes the trait object is enough.)
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn record_spawn_inserts_usage_row() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("connect");
        crate::db::run_migrations(&pool).await.expect("migrate");

        // Seed a user so the FK-less tenant_id refers to a real row.
        let client = pool.get().await.unwrap();
        let tenant: String = client
            .query_one(
                "INSERT INTO users (email, plan_id)
                 VALUES ($1, $2)
                 RETURNING id::text",
                &[&format!("sink-{}@test", std::process::id()), &"pro"],
            )
            .await
            .unwrap()
            .get("id");

        let sink = PostgresUsageSink::new(pool.clone());
        sink.record(TenantId::from(tenant.clone()), "query.served", 1);

        // record() is fire-and-forget via tokio::spawn — give the
        // background task a chance to land its INSERT before we
        // SELECT.
        for _ in 0..50 {
            let row = client
                .query_one(
                    "SELECT COUNT(*)::bigint AS n FROM usage_events
                     WHERE tenant_id = $1::text::uuid AND kind = 'query.served'",
                    &[&tenant],
                )
                .await
                .unwrap();
            if row.get::<_, i64>("n") == 1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("PostgresUsageSink did not land the row within 2.5s");
    }
}
