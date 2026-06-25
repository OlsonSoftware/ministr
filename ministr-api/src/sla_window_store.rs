//! historical SLA-window query seam.
//!
//! Open-core boundary that lets `ministr-mcp`'s `/sla` handler emit
//! "worst p95 in the last N seconds" without depending on
//! `ministr-cloud` for the Postgres-backed read. The cloud crate ships
//! a `PostgresSlaWindowStore` that scans `request_latency_snapshots`;
//! self-hosted serve leaves the field `None` and the `/sla` response
//! omits the window-aggregate field.
//!
//! # Why a separate trait
//!
//! Same reason as [`crate::ApiKeyResolver`] and
//! [`crate::PlanResolver`]: the trait holds the shape `ministr-mcp`
//! (MIT) needs without forcing the closed-cloud `request_latency_snapshots`
//! schema into the open-core surface. The Postgres impl lives in
//! `ministr-cloud` and is wired into `AdminState` via
//! `with_sla_window_store` at cloud-serve startup.
//!
//! The write side lives in `ministr-cloud`
//! directly (no trait) because no other crate writes snapshots. Only
//! the read needs the seam.

use std::future::Future;
use std::pin::Pin;

/// Errors a [`SlaWindowStore`] implementation can surface to the
/// `/sla` handler.
#[derive(Debug, thiserror::Error)]
pub enum SlaWindowStoreError {
    /// Storage layer rejected the query (network, schema drift, etc.).
    /// Treated as "no window data available" by the handler — the
    /// `/sla` response renders the window field as `null` rather than
    /// failing the whole probe (which load balancers depend on).
    #[error("sla window store: {0}")]
    Storage(String),
}

/// Returned future shape for [`SlaWindowStore::max_p95_since`].
pub type MaxP95Future<'a> =
    Pin<Box<dyn Future<Output = Result<Option<u32>, SlaWindowStoreError>> + Send + 'a>>;

/// Query historical SLA snapshots persisted by the
/// flush task.
///
/// Wired into `AdminState` via `with_sla_window_store`; the `/sla`
/// handler calls [`max_p95_since`] with `now - 30 days` to render the
/// `latency.window_30d_max_p95_ms` field.
///
/// Implementations return `Ok(None)` when the window is empty (a
/// freshly-restarted pod hasn't flushed anything yet, or all rows
/// have aged out) so the handler renders JSON `null`. Reserve
/// `Err(Storage)` for genuine backend failures the operator should
/// see in the warn log.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn SlaWindowStore>` inside `AdminState`.
///
/// [`max_p95_since`]: SlaWindowStore::max_p95_since
pub trait SlaWindowStore: Send + Sync + std::fmt::Debug {
    /// Return the maximum `p95_us` over snapshots with `ts_unix >= since_ts_unix`.
    ///
    /// "Max" is the right summary for an SLA contract — if max p95
    /// stays under the contractual ceiling across the window, the
    /// SLA was honoured. Median or mean would smooth over breaches.
    ///
    /// # Errors
    ///
    /// Returns [`SlaWindowStoreError::Storage`] when the backend
    /// rejects the query. The handler logs at warn and renders the
    /// window field as `null`.
    fn max_p95_since(&self, since_ts_unix: i64) -> MaxP95Future<'_>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct StubStore {
        canned: Option<u32>,
        boom: bool,
    }

    impl SlaWindowStore for StubStore {
        fn max_p95_since(&self, _since: i64) -> MaxP95Future<'_> {
            let canned = self.canned;
            let boom = self.boom;
            Box::pin(async move {
                if boom {
                    Err(SlaWindowStoreError::Storage("synthetic".into()))
                } else {
                    Ok(canned)
                }
            })
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let store: Arc<dyn SlaWindowStore> = Arc::new(StubStore::default());
        let _ = store;
    }

    #[tokio::test]
    async fn stub_returns_canned_value() {
        let s = StubStore {
            canned: Some(7_500),
            boom: false,
        };
        let v = s.max_p95_since(0).await.unwrap();
        assert_eq!(v, Some(7_500));
    }

    #[tokio::test]
    async fn stub_returns_none_when_empty() {
        let s = StubStore::default();
        let v = s.max_p95_since(0).await.unwrap();
        assert!(v.is_none());
    }

    #[tokio::test]
    async fn stub_surfaces_storage_error() {
        let s = StubStore {
            boom: true,
            ..Default::default()
        };
        let err = s.max_p95_since(0).await.unwrap_err();
        assert!(matches!(err, SlaWindowStoreError::Storage(_)));
    }
}
