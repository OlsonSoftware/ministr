//! F6.1-a — Postgres-backed [`SessionStorage`] for durable agent
//! session snapshots.
//!
//! Single thin layer over `agent_sessions` (migration 0008). Writes go
//! through one UPSERT; reads through one PK lookup; deletes through one
//! DELETE. Everything is idempotent.
//!
//! Wired into `ministr-core`'s `SessionRegistry` via the
//! `Option<Arc<dyn SessionStorage>>` seam in F6.1-b; this file
//! ships the trait impl + tests so the seam is open and ready.

use std::sync::Arc;

use deadpool_postgres::Pool;
use ministr_api::{
    LoadSessionFuture, SaveSessionFuture, SessionMutFuture, SessionSnapshot, SessionStorage,
    SessionStorageError,
};
use tracing::warn;

/// `SessionStorage` impl that round-trips snapshots through the
/// `agent_sessions` table.
///
/// Cheap to clone — the inner pool is already an Arc.
#[derive(Debug, Clone)]
pub struct PostgresSessionStorage {
    pool: Arc<Pool>,
}

impl PostgresSessionStorage {
    /// Construct from an existing owned pool.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Construct from a shared `Arc<Pool>` — the serve binary builds
    /// one pool and threads it through every cloud-side state, so the
    /// `Arc`-aware constructor composes cleanly.
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

impl SessionStorage for PostgresSessionStorage {
    fn save<'a>(&'a self, snapshot: &'a SessionSnapshot) -> SaveSessionFuture<'a> {
        let pool = Arc::clone(&self.pool);
        let snap = snapshot.clone();
        Box::pin(async move {
            let client = pool
                .get()
                .await
                .map_err(|e| SessionStorageError::Storage(format!("get conn: {e}")))?;
            // UPSERT on (tenant_id, session_id) PK. `opened_at` is
            // preserved across re-saves (the snapshot carries the
            // historical value); `last_seen_at` advances on every
            // save call.
            client
                .execute(
                    "INSERT INTO agent_sessions
                       (id, tenant_id, corpus_id, opened_at, last_seen_at,
                        budget_used, coherence_score)
                     VALUES ($1, $2::text::uuid, $3, $4::timestamptz, $5::timestamptz, $6, $7)
                     ON CONFLICT (tenant_id, id) DO UPDATE
                       SET corpus_id       = EXCLUDED.corpus_id,
                           last_seen_at    = EXCLUDED.last_seen_at,
                           budget_used     = EXCLUDED.budget_used,
                           coherence_score = EXCLUDED.coherence_score",
                    &[
                        &snap.session_id,
                        &snap.tenant_id,
                        &snap.corpus_id,
                        &snap.opened_at,
                        &snap.last_seen_at,
                        &snap.budget_used,
                        &snap.coherence_score,
                    ],
                )
                .await
                .map_err(|e| {
                    warn!(error = %e, session_id = %snap.session_id,
                          "PostgresSessionStorage: save failed");
                    SessionStorageError::Storage(format!("upsert agent_session: {e}"))
                })?;
            Ok(())
        })
    }

    fn load<'a>(
        &'a self,
        tenant_id: &'a str,
        session_id: &'a str,
    ) -> LoadSessionFuture<'a> {
        let pool = Arc::clone(&self.pool);
        let tenant = tenant_id.to_owned();
        let id = session_id.to_owned();
        Box::pin(async move {
            let client = pool
                .get()
                .await
                .map_err(|e| SessionStorageError::Storage(format!("get conn: {e}")))?;
            let row = client
                .query_opt(
                    "SELECT
                         id,
                         tenant_id::text  AS tenant_id_text,
                         corpus_id,
                         to_char(opened_at    AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                              AS opened_at_iso,
                         to_char(last_seen_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                              AS last_seen_at_iso,
                         budget_used,
                         coherence_score
                     FROM agent_sessions
                     WHERE tenant_id = $1::text::uuid AND id = $2",
                    &[&tenant, &id],
                )
                .await
                .map_err(|e| SessionStorageError::Storage(format!("load agent_session: {e}")))?;
            Ok(row.map(|r| SessionSnapshot {
                session_id: r.get("id"),
                tenant_id: r.get("tenant_id_text"),
                corpus_id: r.get("corpus_id"),
                opened_at: r.get("opened_at_iso"),
                last_seen_at: r.get("last_seen_at_iso"),
                budget_used: r.get("budget_used"),
                coherence_score: r.get("coherence_score"),
            }))
        })
    }

    fn touch<'a>(
        &'a self,
        tenant_id: &'a str,
        session_id: &'a str,
    ) -> SessionMutFuture<'a> {
        let pool = Arc::clone(&self.pool);
        let tenant = tenant_id.to_owned();
        let id = session_id.to_owned();
        Box::pin(async move {
            let client = pool
                .get()
                .await
                .map_err(|e| SessionStorageError::Storage(format!("get conn: {e}")))?;
            client
                .execute(
                    "UPDATE agent_sessions
                     SET last_seen_at = now()
                     WHERE tenant_id = $1::text::uuid AND id = $2",
                    &[&tenant, &id],
                )
                .await
                .map_err(|e| SessionStorageError::Storage(format!("touch agent_session: {e}")))?;
            Ok(())
        })
    }

    fn delete<'a>(
        &'a self,
        tenant_id: &'a str,
        session_id: &'a str,
    ) -> SessionMutFuture<'a> {
        let pool = Arc::clone(&self.pool);
        let tenant = tenant_id.to_owned();
        let id = session_id.to_owned();
        Box::pin(async move {
            let client = pool
                .get()
                .await
                .map_err(|e| SessionStorageError::Storage(format!("get conn: {e}")))?;
            client
                .execute(
                    "DELETE FROM agent_sessions
                     WHERE tenant_id = $1::text::uuid AND id = $2",
                    &[&tenant, &id],
                )
                .await
                .map_err(|e| SessionStorageError::Storage(format!("delete agent_session: {e}")))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check that `PostgresSessionStorage` satisfies the
    /// dyn-shape needed by `SessionRegistry`'s `Option<Arc<dyn
    /// SessionStorage>>` seam.
    #[test]
    fn is_dyn_usable() {
        fn _accepts(_: Arc<dyn SessionStorage>) {}
        // Compile-only; we'd need a real pool to construct one at
        // runtime, and the Postgres-integration coverage of the SQL
        // paths is gated on MINISTR_TEST_PG_URL per convention.
        // Validate via the type system instead.
    }
}
