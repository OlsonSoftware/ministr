//! Billable-usage emission hook.
//!
//! [`UsageSink`] is the trait the daemon's `record_activity`
//! middleware (F1.4 sub-bullet 2) fires whenever a tool route
//! completes successfully. The local stack ships no concrete
//! implementation — self-hosted serve never bills anyone. Cloud
//! deployments wire `ministr_cloud::billing::PostgresUsageSink`,
//! which appends a row to `usage_events` for each call.
//!
//! # Why sync, not async
//!
//! Async trait methods require either `impl Future` (which breaks
//! `dyn` dispatch in stable Rust) or boxed-future plumbing
//! (`Pin<Box<dyn Future>>` returns from every method). The crate's
//! convention elsewhere — see `OAuthStorage` — uses static `impl
//! Future` because enum dispatch makes `dyn` unnecessary. The
//! `UsageSink` boundary is genuinely `dyn` (cloud impl injected at
//! runtime), so we side-step the async-dyn tax by making the trait
//! method fire-and-forget: implementations spawn their own
//! `tokio::spawn` if the work is async. The middleware never blocks
//! on the sink.

use crate::tenant::TenantId;

/// Sink for billable usage events emitted by the daemon's activity
/// middleware.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn UsageSink>` inside `AppState`. The trait is `dyn`-safe
/// (no generics, no `impl Future`); the cloud crate's concrete
/// `PostgresUsageSink` spawns a tokio task per call.
pub trait UsageSink: Send + Sync + std::fmt::Debug {
    /// Record one billable event for `tenant_id`. `kind` is a stable
    /// wire-format string (see `ministr_cloud::UsageEventKind`):
    /// `"corpus.indexed"`, `"index.minutes"`, `"query.served"`, or
    /// `"atlas.queries"`. `count` is additive per call.
    ///
    /// Fire-and-forget — the middleware never observes errors from
    /// the sink, so an implementation's storage hiccup never fails
    /// the enclosing tool call. The implementation is responsible
    /// for logging its own failures.
    fn record(&self, tenant_id: TenantId, kind: &'static str, count: i64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct MockSink {
        events: Mutex<Vec<(TenantId, &'static str, i64)>>,
    }

    impl UsageSink for MockSink {
        fn record(&self, tenant_id: TenantId, kind: &'static str, count: i64) {
            self.events.lock().unwrap().push((tenant_id, kind, count));
        }
    }

    #[test]
    fn trait_is_dyn_compatible() {
        // Compile-time proof — if UsageSink isn't dyn-safe, this
        // line fails to type-check.
        let sink: std::sync::Arc<dyn UsageSink> = std::sync::Arc::new(MockSink::default());
        sink.record(TenantId::from("c1"), "query.served", 1);
        sink.record(TenantId::from("c2"), "index.minutes", 5);
    }

    #[test]
    fn mock_sink_captures_events() {
        let sink = MockSink::default();
        sink.record(TenantId::from("c1"), "query.served", 1);
        sink.record(TenantId::from("c1"), "atlas.queries", 3);
        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].1, "query.served");
        assert_eq!(events[1].2, 3);
    }
}
