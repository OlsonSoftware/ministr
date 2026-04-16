//! Shared application state for the iris daemon.

use std::sync::Arc;

use crate::inference::{ClaudeCliInference, Inference};
use crate::registry::CorpusRegistry;

/// Default maximum concurrent expensive queries (survey, symbols, compress).
const DEFAULT_QUERY_CONCURRENCY: usize = 4;

/// Application-wide shared state.
///
/// Passed to both Tauri commands (GUI) and axum handlers (daemon API)
/// via `Arc`. Holds the single [`CorpusRegistry`] that manages all
/// indexed corpora and the shared embedding model.
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<CorpusRegistry>,
    pub started_at: std::time::Instant,
    /// Semaphore limiting concurrent expensive operations (survey, symbols, compress).
    pub query_semaphore: Arc<tokio::sync::Semaphore>,
    /// Sub-inference engine for `iris_ask`.
    pub inference: Arc<dyn Inference>,
}

impl AppState {
    #[must_use]
    pub fn new(registry: CorpusRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
            started_at: std::time::Instant::now(),
            query_semaphore: Arc::new(tokio::sync::Semaphore::new(DEFAULT_QUERY_CONCURRENCY)),
            inference: Arc::new(ClaudeCliInference::new()),
        }
    }

    /// Create state from an already-shared registry.
    #[must_use]
    pub fn from_arc(registry: Arc<CorpusRegistry>) -> Self {
        Self {
            registry,
            started_at: std::time::Instant::now(),
            query_semaphore: Arc::new(tokio::sync::Semaphore::new(DEFAULT_QUERY_CONCURRENCY)),
            inference: Arc::new(ClaudeCliInference::new()),
        }
    }

    /// Override the inference engine (for testing).
    #[must_use]
    pub fn with_inference(mut self, inference: Arc<dyn Inference>) -> Self {
        self.inference = inference;
        self
    }

    #[must_use]
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
