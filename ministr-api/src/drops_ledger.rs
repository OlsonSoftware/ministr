//! Append-only ledger of agent-session evictions.
//!
//! F6.1-d — open-core seam for persisting "this claim was evicted from
//! the agent's context window at this timestamp" events. The trait
//! lives in `ministr-api` (MIT) so `ministr-core`'s session/drops
//! logic can fire append calls into a `dyn`-typed sink without
//! depending on `ministr-cloud`. The cloud ships a
//! `PostgresDropsLedger` that writes to the `session_drops` table.
//!
//! # Why separate from [`SessionStorage`]
//!
//! [`SessionStorage`] is "tell me where the session is right now"
//! (occasional, full state — budget, coherence). The drops ledger is
//! "log this eviction event" (every drop, append-only). Mixing them
//! would force snapshot writes to bloat with every drop OR force
//! drops to wait for the next snapshot, neither of which is right.
//!
//! Cleaner: a separate trait with append-only semantics. The two
//! traits compose at the call site (Session calls both; cloud wires
//! both with the same Postgres pool).
//!
//! [`SessionStorage`]: crate::session_storage::SessionStorage
//!
//! # Shape parallels [`AuditSink`]
//!
//! Fire-and-forget posture for the append path (the eviction itself
//! already happened in-memory; persisting it is a durability layer,
//! not a request gate). Read path is async with a return value so
//! `try_restore` can hydrate evicted-content awareness.
//!
//! [`AuditSink`]: crate::audit::AuditSink

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

/// Errors a [`DropsLedger`] implementation can surface.
#[derive(Debug, thiserror::Error)]
pub enum DropsLedgerError {
    /// Storage layer rejected the call (network, schema drift, etc.).
    /// Callers typically log + continue — a ledger hiccup must not
    /// fail the enclosing tool call.
    #[error("drops ledger: {0}")]
    Storage(String),
}

/// One persisted eviction event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropEntry {
    /// Session id as the agent presented it.
    pub session_id: String,
    /// Tenant UUID string. Indexed alongside `session_id` for the
    /// `(tenant_id, session_id)` lookup hot path.
    pub tenant_id: String,
    /// Claim / content id that was evicted. Mirrors the `content_id`
    /// vocabulary the [`crate::session_storage::SessionSnapshot`] /
    /// in-memory tracker speak.
    pub claim_id: String,
    /// ISO-8601 UTC timestamp of the eviction.
    pub evicted_at: String,
}

/// Returned future shape for [`DropsLedger::append`].
pub type AppendDropFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), DropsLedgerError>> + Send + 'a>>;

/// Returned future shape for [`DropsLedger::list_for_session`].
pub type ListDropsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<DropEntry>, DropsLedgerError>> + Send + 'a>>;

/// Append-only ledger of session evictions.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn DropsLedger>` inside `SessionRegistry` (alongside
/// [`SessionStorage`]).
///
/// # Failure posture
///
/// `append` is fire-and-forget — the eviction itself happened
/// in-memory before this call; the ledger is a durability layer.
/// `list_for_session` is async with a `Result` so `try_restore` can
/// distinguish "no drops recorded" from a storage hiccup.
///
/// [`SessionStorage`]: crate::session_storage::SessionStorage
pub trait DropsLedger: Send + Sync + std::fmt::Debug {
    /// Append a single drop record. Implementations spawn their own
    /// async work so the caller's hot path is never blocked.
    fn append<'a>(&'a self, entry: &'a DropEntry) -> AppendDropFuture<'a>;

    /// List every drop recorded for `(tenant_id, session_id)`,
    /// ordered oldest-first. Used by `try_restore` to hydrate the
    /// resumed session's drop awareness.
    fn list_for_session<'a>(
        &'a self,
        tenant_id: &'a str,
        session_id: &'a str,
    ) -> ListDropsFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct StubLedger {
        rows: Mutex<Vec<DropEntry>>,
    }

    impl DropsLedger for StubLedger {
        fn append<'a>(&'a self, entry: &'a DropEntry) -> AppendDropFuture<'a> {
            let owned = entry.clone();
            Box::pin(async move {
                self.rows.lock().unwrap().push(owned);
                Ok(())
            })
        }
        fn list_for_session<'a>(
            &'a self,
            tenant_id: &'a str,
            session_id: &'a str,
        ) -> ListDropsFuture<'a> {
            Box::pin(async move {
                let rows = self.rows.lock().unwrap();
                Ok(rows
                    .iter()
                    .filter(|r| r.tenant_id == tenant_id && r.session_id == session_id)
                    .cloned()
                    .collect())
            })
        }
    }

    fn fixture(claim: &str) -> DropEntry {
        DropEntry {
            session_id: "sess-1".into(),
            tenant_id: "tenant-uuid".into(),
            claim_id: claim.into(),
            evicted_at: "2026-05-21T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn append_then_list_round_trips() {
        let stub = Arc::new(StubLedger::default());
        let ledger: Arc<dyn DropsLedger> = Arc::clone(&stub) as _;
        ledger.append(&fixture("c1")).await.unwrap();
        ledger.append(&fixture("c2")).await.unwrap();
        let rows = ledger
            .list_for_session("tenant-uuid", "sess-1")
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].claim_id, "c1");
        assert_eq!(rows[1].claim_id, "c2");
    }

    #[tokio::test]
    async fn list_filters_by_tenant_and_session() {
        let stub = Arc::new(StubLedger::default());
        let ledger: Arc<dyn DropsLedger> = Arc::clone(&stub) as _;
        ledger.append(&fixture("c1")).await.unwrap();
        let mut other = fixture("c2");
        other.session_id = "sess-other".into();
        ledger.append(&other).await.unwrap();
        let mut diff_tenant = fixture("c3");
        diff_tenant.tenant_id = "other-tenant".into();
        ledger.append(&diff_tenant).await.unwrap();
        let rows = ledger
            .list_for_session("tenant-uuid", "sess-1")
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].claim_id, "c1");
    }

    #[tokio::test]
    async fn list_returns_empty_for_unknown_session() {
        let stub = Arc::new(StubLedger::default());
        let ledger: Arc<dyn DropsLedger> = Arc::clone(&stub) as _;
        let rows = ledger
            .list_for_session("nobody", "nothing")
            .await
            .unwrap();
        assert!(rows.is_empty());
    }
}
