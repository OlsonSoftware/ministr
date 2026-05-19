//! Admin endpoints: health probe, indexer trigger, SSE progress, GH webhook.
//!
//! # Layout (SOLID)
//!
//! ```text
//!     router.rs    — composes axum Router (one place to wire routes)
//!         │
//!     handlers.rs  — /healthz, /reindex (POST), SSE progress stream
//!     webhook.rs   — /webhook/github with HMAC-SHA256 verification
//!         │
//!     mod.rs       — AdminState façade handlers code against
//!         │
//!     jobs/        — JobQueue trait + InMemoryJobQueue + SqliteJobQueue
//!                    + JobQueueBackend enum (concrete dispatch)
//!     ids.rs       — admin-local id generation
//! ```

mod handlers;
mod ids;
mod jobs;
mod router;
mod webhook;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub use router::admin_routes;
pub(crate) use jobs::JobQueueBackend;

/// State shared by every admin handler.
///
/// Cheap to `Clone` (everything is `Arc` or already-cloneable). Lifecycle
/// matches the axum server: constructed once in `cmd_serve_http`, attached
/// as `State<AdminState>`.
#[derive(Debug, Clone)]
pub struct AdminState {
    queue: JobQueueBackend,
    webhook_secret: Option<Arc<String>>,
    corpus_count: Arc<AtomicUsize>,
}

impl AdminState {
    /// Construct an `AdminState` from its building blocks.
    #[must_use]
    #[allow(dead_code)] // wired into cmd_serve_http in PR1.4
    pub(crate) fn new(
        queue: JobQueueBackend,
        webhook_secret: Option<String>,
        corpus_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            queue,
            webhook_secret: webhook_secret.map(Arc::new),
            corpus_count,
        }
    }

    fn corpus_count(&self) -> usize {
        self.corpus_count.load(Ordering::Relaxed)
    }

    fn webhook_secret(&self) -> Option<&str> {
        self.webhook_secret.as_deref().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::jobs::InMemoryJobQueue;

    fn state(secret: Option<String>, count: usize) -> AdminState {
        AdminState::new(
            JobQueueBackend::InMemory(InMemoryJobQueue::new()),
            secret,
            Arc::new(AtomicUsize::new(count)),
        )
    }

    #[test]
    fn webhook_disabled_when_no_secret() {
        let s = state(None, 0);
        assert!(s.webhook_secret().is_none());
    }

    #[test]
    fn corpus_count_round_trips() {
        let s = state(Some("x".into()), 7);
        assert_eq!(s.corpus_count(), 7);
        s.corpus_count.store(11, Ordering::Relaxed);
        assert_eq!(s.corpus_count(), 11);
    }
}
