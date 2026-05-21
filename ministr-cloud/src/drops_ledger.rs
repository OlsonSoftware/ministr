//! F6.1-d — Postgres-backed [`DropsLedger`] for durable session
//! eviction events.
//!
//! Mirrors [`crate::session_storage::PostgresSessionStorage`] in shape:
//! single thin layer over `session_drops` (migration 0009). Appends
//! go through one INSERT; reads through one indexed SELECT.

use std::sync::Arc;

use deadpool_postgres::Pool;
use ministr_api::{
    AppendDropFuture, DropEntry, DropsLedger, DropsLedgerError, ListDropsFuture,
};
use tracing::warn;

/// `DropsLedger` impl that round-trips drop events through the
/// `session_drops` table.
///
/// Cheap to clone — the inner pool is already an Arc.
#[derive(Debug, Clone)]
pub struct PostgresDropsLedger {
    pool: Arc<Pool>,
}

impl PostgresDropsLedger {
    /// Construct from an owned pool.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Construct from a shared `Arc<Pool>` — `cmd_serve_http` builds
    /// one pool and threads it through every cloud-side state.
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

impl DropsLedger for PostgresDropsLedger {
    fn append<'a>(&'a self, entry: &'a DropEntry) -> AppendDropFuture<'a> {
        let pool = Arc::clone(&self.pool);
        let owned = entry.clone();
        Box::pin(async move {
            let client = pool
                .get()
                .await
                .map_err(|e| DropsLedgerError::Storage(format!("get conn: {e}")))?;
            client
                .execute(
                    "INSERT INTO session_drops
                       (session_id, tenant_id, claim_id, evicted_at)
                     VALUES ($1, $2::uuid, $3, $4::timestamptz)",
                    &[
                        &owned.session_id,
                        &owned.tenant_id,
                        &owned.claim_id,
                        &owned.evicted_at,
                    ],
                )
                .await
                .map_err(|e| {
                    warn!(error = %e, session_id = %owned.session_id,
                          "PostgresDropsLedger: append failed");
                    DropsLedgerError::Storage(format!("insert session_drop: {e}"))
                })?;
            Ok(())
        })
    }

    fn list_for_session<'a>(
        &'a self,
        tenant_id: &'a str,
        session_id: &'a str,
    ) -> ListDropsFuture<'a> {
        let pool = Arc::clone(&self.pool);
        let tenant = tenant_id.to_owned();
        let id = session_id.to_owned();
        Box::pin(async move {
            let client = pool
                .get()
                .await
                .map_err(|e| DropsLedgerError::Storage(format!("get conn: {e}")))?;
            let rows = client
                .query(
                    "SELECT
                         session_id,
                         tenant_id::text  AS tenant_id_text,
                         claim_id,
                         to_char(evicted_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                          AS evicted_at_iso
                     FROM session_drops
                     WHERE tenant_id = $1::uuid AND session_id = $2
                     ORDER BY id ASC",
                    &[&tenant, &id],
                )
                .await
                .map_err(|e| {
                    DropsLedgerError::Storage(format!("list session_drops: {e}"))
                })?;
            Ok(rows
                .into_iter()
                .map(|r| DropEntry {
                    session_id: r.get("session_id"),
                    tenant_id: r.get("tenant_id_text"),
                    claim_id: r.get("claim_id"),
                    evicted_at: r.get("evicted_at_iso"),
                })
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check that `PostgresDropsLedger` satisfies the
    /// dyn-shape that `SessionRegistry`'s future
    /// `Option<Arc<dyn DropsLedger>>` field will use.
    #[test]
    fn is_dyn_usable() {
        fn _accepts(_: Arc<dyn DropsLedger>) {}
    }
}
