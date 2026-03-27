//! Shared application state for the iris daemon.

use std::sync::Arc;

use crate::registry::CorpusRegistry;

/// Application-wide shared state.
///
/// Passed to both Tauri commands (GUI) and axum handlers (daemon API)
/// via `Arc`. Holds the single [`CorpusRegistry`] that manages all
/// indexed corpora and the shared embedding model.
#[derive(Clone)]
pub struct AppState {
    /// Central corpus registry — the heart of the daemon.
    pub registry: Arc<CorpusRegistry>,
    /// Daemon start time for uptime reporting.
    pub started_at: std::time::Instant,
}

impl AppState {
    /// Create a new `AppState` wrapping the given registry.
    #[must_use]
    pub fn new(registry: CorpusRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
            started_at: std::time::Instant::now(),
        }
    }

    /// Daemon uptime in seconds.
    #[must_use]
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
