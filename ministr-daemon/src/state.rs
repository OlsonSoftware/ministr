//! Shared application state for the ministr daemon.

use std::collections::VecDeque;
use std::sync::Arc;

use ministr_api::activity::ActivityEvent;
use ministr_api::coherence::CoherenceEvent;
use ministr_api::{
    AuditSink, BlobSink, IndexJobSink, InstallationTokenMinter, TenantCorpusVisibility, UsageSink,
};
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

/// Capacity of the in-memory coherence (file-change) ring buffer.
///
/// File-change events are lower-frequency than tool calls — 500 entries
/// is comfortably deep for realistic editing sessions.
pub const COHERENCE_BUFFER_CAPACITY: usize = 500;

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
    /// Sub-inference engine for `ministr_ask`.
    pub inference: Arc<dyn Inference>,
    /// Recent tool-call activity (newest at back, popped from front when
    /// capacity is exceeded). Written fire-and-forget from each tool route;
    /// read by the Tauri app, `/activity` HTTP endpoint, and any other
    /// `DaemonClient` consumer.
    pub activity: Arc<RwLock<VecDeque<ActivityEvent>>>,
    /// Recent file-change events — one entry per distinct file observed
    /// during a watcher debounce window. Populated by a subscriber task
    /// per registered corpus; read by the Tauri app, `/coherence-events`
    /// HTTP endpoint, and `DaemonClient::recent_coherence_events`.
    pub coherence: Arc<RwLock<VecDeque<CoherenceEvent>>>,
    /// Billable-usage emission sink. `Some` when cloud mode has wired
    /// the closed `ministr_cloud::billing::PostgresUsageSink` (F1.4
    /// sub-bullet 2); `None` for self-hosted serve where no usage is
    /// billed. The activity middleware fires this fire-and-forget
    /// whenever a tool route completes successfully.
    pub usage_sink: Option<Arc<dyn UsageSink>>,
    /// GitHub App installation-token minter (F2.1). `Some` when cloud
    /// mode has wired `ministr_cloud::github::GitHubAppClient`; `None`
    /// on self-hosted serve where the PAT-in-URL path is the only
    /// authenticated-clone option. The `clone_repo` handler awaits this
    /// when the request body carries `github_installation_id`.
    pub installation_minter: Option<Arc<dyn InstallationTokenMinter>>,
    /// Durable corpus-bundle export sink (Phase 2). `Some` when cloud
    /// mode has wired `ministr_cloud::blob_sink::BlobBackendSink`;
    /// `None` for self-hosted serve where the user's local disk is
    /// already durable. The registry's completion reactor fires this
    /// fire-and-forget whenever a corpus finishes ingesting so the
    /// bundle lands in Azure Blob Storage before the pod recycles.
    pub blob_sink: Option<Arc<dyn BlobSink>>,
    /// PHASE3 chunk 4 — cloud serve-pod enqueue hook. `Some` when
    /// `cmd_serve_http` has wired
    /// `ministr_cloud::PostgresIndexJobSink`; `None` on self-hosted
    /// serve. When wired, the `POST /api/v1/corpora` and clone
    /// handlers route through this instead of running ingestion
    /// inline; the progress SSE polls `latest_for_corpus` against
    /// Postgres instead of the in-memory `IngestionProgress`.
    pub index_job_sink: Option<Arc<dyn IndexJobSink>>,
    /// F3.2-iii — tenant-aware corpus visibility filter. `Some` when
    /// cloud mode has wired `ministr_cloud::PostgresTenantCorpusFilter`
    /// (the same struct implements both `TenantCorpusFilter` for the
    /// MCP-side gate and this trait for the daemon-side list). `None`
    /// on self-hosted serve where every authenticated caller sees
    /// every corpus.
    pub corpus_visibility: Option<Arc<dyn TenantCorpusVisibility>>,
    /// F3.7b — audit-log emission sink for corpus-mutation actions.
    /// `Some` in cloud mode wires `ministr_cloud::PostgresAuditSink`;
    /// the daemon's `register_corpus` / `clone_repo` / `unregister_corpus`
    /// handlers call `record` after a successful state change so
    /// `audit_events` carries the `corpus.created` / `corpus.cloned` /
    /// `corpus.deleted` row. `None` on self-hosted serve.
    pub audit_sink: Option<Arc<dyn AuditSink>>,
    /// The daemon-hosted exec run engine (exec-epic). ONE engine per
    /// daemon so cross-process kill and live log tails work: the exec
    /// routes spawn through it, and any client (MCP forward, Tauri app)
    /// reaches the same cancel tokens + live capture buffers. Lazy —
    /// the engine opens its run store on first use.
    pub exec: Arc<crate::exec::EngineCell>,
}

impl AppState {
    #[must_use]
    pub fn new(registry: CorpusRegistry) -> Self {
        let coherence = Arc::new(RwLock::new(VecDeque::with_capacity(
            COHERENCE_BUFFER_CAPACITY,
        )));
        // Wire the sink BEFORE wrapping in Arc so any later `register`
        // call — including the first one from `restore` — spawns a
        // pusher task that feeds this buffer.
        registry.set_coherence_sink(Arc::clone(&coherence));
        Self {
            registry: Arc::new(registry),
            started_at: std::time::Instant::now(),
            query_semaphore: Arc::new(tokio::sync::Semaphore::new(DEFAULT_QUERY_CONCURRENCY)),
            inference: Arc::new(ClaudeCliInference::new()),
            activity: Arc::new(RwLock::new(VecDeque::with_capacity(
                ACTIVITY_BUFFER_CAPACITY,
            ))),
            coherence,
            usage_sink: None,
            installation_minter: None,
            blob_sink: None,
            index_job_sink: None,
            corpus_visibility: None,
            audit_sink: None,
            exec: Arc::new(crate::exec::EngineCell::default()),
        }
    }

    /// Create state from an already-shared registry.
    #[must_use]
    pub fn from_arc(registry: Arc<CorpusRegistry>) -> Self {
        let coherence = Arc::new(RwLock::new(VecDeque::with_capacity(
            COHERENCE_BUFFER_CAPACITY,
        )));
        registry.set_coherence_sink(Arc::clone(&coherence));
        Self {
            registry,
            started_at: std::time::Instant::now(),
            query_semaphore: Arc::new(tokio::sync::Semaphore::new(DEFAULT_QUERY_CONCURRENCY)),
            inference: Arc::new(ClaudeCliInference::new()),
            activity: Arc::new(RwLock::new(VecDeque::with_capacity(
                ACTIVITY_BUFFER_CAPACITY,
            ))),
            coherence,
            usage_sink: None,
            installation_minter: None,
            blob_sink: None,
            index_job_sink: None,
            corpus_visibility: None,
            audit_sink: None,
            exec: Arc::new(crate::exec::EngineCell::default()),
        }
    }

    /// Wire a billable-usage sink (cloud mode). Returns `self` for
    /// chainable construction in `cmd_serve_http`.
    #[must_use]
    pub fn with_usage_sink(mut self, sink: Arc<dyn UsageSink>) -> Self {
        self.usage_sink = Some(sink);
        self
    }

    /// Wire a GitHub App installation-token minter (F2.1 cloud mode).
    /// Returns `self` for chainable construction.
    #[must_use]
    pub fn with_installation_minter(mut self, minter: Arc<dyn InstallationTokenMinter>) -> Self {
        self.installation_minter = Some(minter);
        self
    }

    /// Wire a durable corpus-bundle export sink (Phase 2 cloud mode).
    /// Returns `self` for chainable construction in `cmd_serve_http`.
    #[must_use]
    pub fn with_blob_sink(mut self, sink: Arc<dyn BlobSink>) -> Self {
        self.blob_sink = Some(sink);
        self
    }

    /// Wire a cloud-mode index-job enqueue sink (PHASE3 chunk 4).
    /// Returns `self` for chainable construction in `cmd_serve_http`.
    #[must_use]
    pub fn with_index_job_sink(mut self, sink: Arc<dyn IndexJobSink>) -> Self {
        self.index_job_sink = Some(sink);
        self
    }

    /// F3.2-iii — wire a tenant-aware corpus visibility filter for
    /// `GET /api/v1/corpora`. When set, the list handler reads the
    /// `Tenant` request extension + asks the filter for the set of
    /// visible `corpus_id`s, then intersects with `registry.list()`.
    /// When unset (self-hosted serve), the list returns every
    /// in-memory corpus — matches the pre-F3.2-iii behaviour.
    #[must_use]
    pub fn with_corpus_visibility(mut self, visibility: Arc<dyn TenantCorpusVisibility>) -> Self {
        self.corpus_visibility = Some(visibility);
        self
    }

    /// F3.7b — wire an audit-log sink for corpus mutations. When set,
    /// the daemon's `register_corpus`, `clone_repo`, and
    /// `unregister_corpus` handlers fire an `audit_events` row on
    /// success. Fire-and-forget inside the sink: a Postgres outage
    /// never propagates to the user's response.
    #[must_use]
    pub fn with_audit_sink(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = Some(sink);
        self
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

    /// Record a file-change coherence event. Fire-and-forget mirrors of
    /// [`push_activity`](Self::push_activity); drops events rather than
    /// blocking the watcher task under buffer pressure.
    pub async fn push_coherence(&self, event: CoherenceEvent) {
        let mut buf = self.coherence.write().await;
        while buf.len() >= COHERENCE_BUFFER_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(event);
    }

    /// Snapshot the most recent `limit` coherence events, newest first.
    pub async fn recent_coherence(&self, limit: usize) -> Vec<CoherenceEvent> {
        let buf = self.coherence.read().await;
        buf.iter().rev().take(limit).cloned().collect()
    }

    /// Snapshot coherence events newer than `since_ms`, newest first.
    pub async fn coherence_since(&self, since_ms: u64, limit: usize) -> Vec<CoherenceEvent> {
        let buf = self.coherence.read().await;
        buf.iter()
            .rev()
            .filter(|e| e.timestamp_ms > since_ms)
            .take(limit)
            .cloned()
            .collect()
    }
}
