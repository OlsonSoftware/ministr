//! Durable agent-session checkpoint seam.
//!
//! open-core boundary for snapshotting agent-session state to
//! durable storage so a pod restart or cross-pod load-balance doesn't
//! reset a session. The trait lives in `ministr-api` (MIT) so
//! `ministr-core`'s `SessionRegistry` can hold an
//! `Option<Arc<dyn SessionStorage>>` without depending on the closed
//! `ministr-cloud` crate. The cloud ships a `PostgresSessionStorage`
//! that writes to the `agent_sessions` table; self-hosted serve leaves
//! the field `None` and sessions remain in-memory.
//!
//! # What's persisted vs. what's recomputed
//!
//! v0 stores only the load-bearing snapshot fields:
//! - `budget_used` — cumulative tokens; a resumed session can't
//!   accidentally re-burn the budget the prior pod already consumed.
//! - `coherence_score` — the live coherence number; resuming with the
//!   warm score avoids the cold-start visual artifact.
//! - `last_seen_at` — for the eventual stale-session prune cron.
//!
//! The much larger in-memory state (claims, drops, invalidated
//! sections, memory tracker) is reconstructed lazily on resume by
//! re-reading from the corpus. Persisting that would multiply storage
//! cost without obvious agent-visible benefit; can revisit if
//! drops-ledger semantics demand it.
//!
//! # Open / save / load semantics
//!
//! - `save` is fire-and-forget (returns a future the caller awaits,
//!   but the future logs + swallows errors rather than propagating
//!   them — a checkpoint failure must not break the user's tool call).
//! - `load` returns `Option<SessionSnapshot>` so a fresh pod can ask
//!   "do you remember this session?" without distinguishing "no
//!   session" from "storage error" at the call site.
//! - `touch` updates only `last_seen_at` — cheaper than a full save
//!   for the hot path where the session is unchanged.
//! - `delete` removes a session row on explicit close.

use std::future::Future;
use std::pin::Pin;

/// Errors a [`SessionStorage`] implementation can surface.
#[derive(Debug, thiserror::Error)]
pub enum SessionStorageError {
    /// Storage layer rejected the call (network, schema drift, etc.).
    /// Callers typically log + continue — a checkpoint hiccup should
    /// not fail the enclosing tool call.
    #[error("session storage: {0}")]
    Storage(String),
}

/// One persisted snapshot of an agent session.
///
/// Field order mirrors the columns in `0008_agent_sessions.sql`. The
/// snapshot is the contract between `ministr-core`'s `SessionRegistry`
/// and the cloud's Postgres backend; future field additions go on the
/// end so older crates can deserialise newer rows.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    /// Session id as the agent presented it (free-form string).
    pub session_id: String,
    /// Tenant UUID string. The PK is `(tenant_id, session_id)` so two
    /// tenants can use the same id without colliding.
    pub tenant_id: String,
    /// Bound corpus id, if any. Sessions opened before a corpus is
    /// chosen leave this `None`.
    pub corpus_id: Option<String>,
    /// ISO-8601 UTC timestamp the session was first opened.
    pub opened_at: String,
    /// ISO-8601 UTC timestamp the session was most recently touched.
    pub last_seen_at: String,
    /// Cumulative tokens this session has consumed.
    pub budget_used: i64,
    /// Cross-session coherence score, in `[0.0, 1.0]`.
    pub coherence_score: f64,
}

/// Returned future shape for [`SessionStorage::save`].
pub type SaveSessionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), SessionStorageError>> + Send + 'a>>;

/// Returned future shape for [`SessionStorage::load`].
pub type LoadSessionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<SessionSnapshot>, SessionStorageError>> + Send + 'a>>;

/// Returned future shape for [`SessionStorage::touch`] and [`SessionStorage::delete`].
pub type SessionMutFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), SessionStorageError>> + Send + 'a>>;

/// Snapshot agent-session state to durable storage.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn SessionStorage>` inside `ministr-core`'s `SessionRegistry`.
///
/// # Failure posture
///
/// Storage errors are surfaced via `Result` so the registry can choose
/// its own posture (today: log + continue; future: prometheus counter).
/// A storage outage must never break a live tool call — the in-memory
/// `SessionEntry` is the source of truth for the active request, and the
/// snapshot is a best-effort durability layer.
pub trait SessionStorage: Send + Sync + std::fmt::Debug {
    /// Write the full snapshot, upserting on `(tenant_id, session_id)`.
    fn save<'a>(&'a self, snapshot: &'a SessionSnapshot) -> SaveSessionFuture<'a>;

    /// Load a snapshot by `(tenant_id, session_id)`. Returns `Ok(None)`
    /// when no row exists — distinguishable from a storage error so
    /// the registry can hydrate a fresh in-memory `SessionEntry` vs.
    /// fail closed on backend issues.
    fn load<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> LoadSessionFuture<'a>;

    /// Touch `last_seen_at` for an existing row. Cheaper than a full
    /// `save` when the budget / coherence haven't changed — useful for
    /// keep-alive on long-lived sessions whose state is mostly stable.
    fn touch<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> SessionMutFuture<'a>;

    /// Remove a session row. Idempotent — deleting a non-existent
    /// session is `Ok(())`.
    fn delete<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> SessionMutFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct StubStorage {
        rows: Mutex<Vec<SessionSnapshot>>,
    }

    impl SessionStorage for StubStorage {
        fn save<'a>(&'a self, snapshot: &'a SessionSnapshot) -> SaveSessionFuture<'a> {
            let owned = snapshot.clone();
            Box::pin(async move {
                let mut rows = self.rows.lock().unwrap();
                rows.retain(|r| {
                    !(r.tenant_id == owned.tenant_id && r.session_id == owned.session_id)
                });
                rows.push(owned);
                Ok(())
            })
        }

        fn load<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> LoadSessionFuture<'a> {
            Box::pin(async move {
                let rows = self.rows.lock().unwrap();
                Ok(rows
                    .iter()
                    .find(|r| r.tenant_id == tenant_id && r.session_id == session_id)
                    .cloned())
            })
        }

        fn touch<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> SessionMutFuture<'a> {
            Box::pin(async move {
                let mut rows = self.rows.lock().unwrap();
                if let Some(row) = rows
                    .iter_mut()
                    .find(|r| r.tenant_id == tenant_id && r.session_id == session_id)
                {
                    row.last_seen_at = "2026-05-21T00:00:00Z".into();
                }
                Ok(())
            })
        }

        fn delete<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> SessionMutFuture<'a> {
            Box::pin(async move {
                let mut rows = self.rows.lock().unwrap();
                rows.retain(|r| !(r.tenant_id == tenant_id && r.session_id == session_id));
                Ok(())
            })
        }
    }

    fn fixture() -> SessionSnapshot {
        SessionSnapshot {
            session_id: "sess-1".into(),
            tenant_id: "tenant-uuid".into(),
            corpus_id: Some("corpus-a".into()),
            opened_at: "2026-05-21T00:00:00Z".into(),
            last_seen_at: "2026-05-21T00:00:00Z".into(),
            budget_used: 1024,
            coherence_score: 0.87,
        }
    }

    #[tokio::test]
    async fn save_then_load_round_trips() {
        let stub = StubStorage::default();
        let storage: Arc<dyn SessionStorage> = Arc::new(stub);
        let snap = fixture();
        storage.save(&snap).await.unwrap();
        let loaded = storage
            .load(&snap.tenant_id, &snap.session_id)
            .await
            .unwrap();
        assert_eq!(loaded, Some(snap));
    }

    #[tokio::test]
    async fn load_returns_none_for_unknown_session() {
        let storage: Arc<dyn SessionStorage> = Arc::new(StubStorage::default());
        let loaded = storage.load("nobody", "nothing").await.unwrap();
        assert_eq!(loaded, None);
    }

    #[tokio::test]
    async fn save_upserts_on_tenant_session_pk() {
        let storage: Arc<dyn SessionStorage> = Arc::new(StubStorage::default());
        let mut snap = fixture();
        storage.save(&snap).await.unwrap();
        snap.budget_used = 4096;
        storage.save(&snap).await.unwrap();
        let loaded = storage
            .load(&snap.tenant_id, &snap.session_id)
            .await
            .unwrap();
        assert_eq!(loaded.unwrap().budget_used, 4096);
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let storage: Arc<dyn SessionStorage> = Arc::new(StubStorage::default());
        // Deleting a non-existent session should still return Ok.
        storage.delete("nobody", "nothing").await.unwrap();
        let snap = fixture();
        storage.save(&snap).await.unwrap();
        storage
            .delete(&snap.tenant_id, &snap.session_id)
            .await
            .unwrap();
        // Second delete on the now-gone row is still Ok.
        storage
            .delete(&snap.tenant_id, &snap.session_id)
            .await
            .unwrap();
        assert_eq!(
            storage
                .load(&snap.tenant_id, &snap.session_id)
                .await
                .unwrap(),
            None
        );
    }
}
