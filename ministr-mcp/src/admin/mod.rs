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

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use jobs::{InMemoryJobQueue, SqliteJobQueue};

pub use router::{admin_protected_routes, admin_public_routes};
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
    /// Construct an `AdminState` with an in-memory job queue. Job state
    /// is lost on restart — fine for local dev or single-container
    /// deployments where state doesn't need to outlive the process.
    #[must_use]
    pub fn in_memory(webhook_secret: Option<String>) -> Self {
        Self {
            queue: JobQueueBackend::InMemory(InMemoryJobQueue::new()),
            webhook_secret: webhook_secret.map(Arc::new),
            corpus_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Construct an `AdminState` backed by `SQLite` at `jobs_db_path`. Jobs
    /// survive process restarts — intended for ACA deployments where the
    /// path is on the Azure Files mount.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` if the database file cannot be opened or
    /// the schema cannot be initialised.
    pub fn persistent(jobs_db_path: &Path, webhook_secret: Option<String>) -> io::Result<Self> {
        let queue = SqliteJobQueue::open(jobs_db_path)
            .map(JobQueueBackend::Sqlite)
            .map_err(io::Error::other)?;
        Ok(Self {
            queue,
            webhook_secret: webhook_secret.map(Arc::new),
            corpus_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Update the corpus count surfaced by `/healthz`. Called by the CLI
    /// once corpora are discovered.
    pub fn set_corpus_count(&self, n: usize) {
        self.corpus_count.store(n, Ordering::Relaxed);
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

    #[test]
    fn webhook_disabled_when_no_secret() {
        let s = AdminState::in_memory(None);
        assert!(s.webhook_secret().is_none());
    }

    #[test]
    fn corpus_count_round_trips() {
        let s = AdminState::in_memory(Some("x".into()));
        assert_eq!(s.corpus_count(), 0);
        s.set_corpus_count(7);
        assert_eq!(s.corpus_count(), 7);
    }
}
