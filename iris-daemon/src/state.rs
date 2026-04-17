//! Shared application state for the iris daemon.

use std::collections::VecDeque;
use std::sync::Arc;

use iris_api::activity::ActivityEvent;
use tokio::sync::RwLock;

use crate::inference::{ClaudeCliInference, Inference};
use crate::registry::CorpusRegistry;

/// Default maximum concurrent expensive queries (survey, symbols, compress).
const DEFAULT_QUERY_CONCURRENCY: usize = 4;

/// Capacity of the in-memory activity ring buffer.
///
/// Old events age out as new tool calls arrive; callers (Tauri, CLI, MCP)
/// should poll often enough to catch events before they fall off the end.
/// At a sustained 10 calls/sec that's ~50s of history.
pub const ACTIVITY_BUFFER_CAPACITY: usize = 500;

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
    /// Recent tool-call activity (newest at back, popped from front when
    /// capacity is exceeded). Written fire-and-forget from each tool route;
    /// read by the Tauri app, `/activity` HTTP endpoint, and any other
    /// `DaemonClient` consumer.
    pub activity: Arc<RwLock<VecDeque<ActivityEvent>>>,
}

impl AppState {
    #[must_use]
    pub fn new(registry: CorpusRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
            started_at: std::time::Instant::now(),
            query_semaphore: Arc::new(tokio::sync::Semaphore::new(DEFAULT_QUERY_CONCURRENCY)),
            inference: Arc::new(ClaudeCliInference::new()),
            activity: Arc::new(RwLock::new(VecDeque::with_capacity(
                ACTIVITY_BUFFER_CAPACITY,
            ))),
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
            activity: Arc::new(RwLock::new(VecDeque::with_capacity(
                ACTIVITY_BUFFER_CAPACITY,
            ))),
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

    /// Record a tool-call activity event. Fire-and-forget: if the lock is
    /// contended or the buffer is poisoned, the event is silently dropped
    /// rather than failing the enclosing tool call.
    pub async fn push_activity(&self, event: ActivityEvent) {
        let mut buf = self.activity.write().await;
        while buf.len() >= ACTIVITY_BUFFER_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(event);
    }

    /// Snapshot the most recent `limit` events, newest first.
    ///
    /// The buffer is stored newest-at-back for O(1) appends; this method
    /// reverses on read.
    pub async fn recent_activity(&self, limit: usize) -> Vec<ActivityEvent> {
        let buf = self.activity.read().await;
        buf.iter().rev().take(limit).cloned().collect()
    }

    /// Snapshot events newer than `since_ms` (unix millis), newest first.
    pub async fn activity_since(&self, since_ms: u64, limit: usize) -> Vec<ActivityEvent> {
        let buf = self.activity.read().await;
        buf.iter()
            .rev()
            .filter(|e| e.timestamp_ms > since_ms)
            .take(limit)
            .cloned()
            .collect()
    }
}
