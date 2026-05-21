//! Constructors, builders, and accessor methods for [`MinistrServer`].
//!
//! Separates the construction and configuration API from the MCP handler
//! logic, making it easier to understand the server's public surface.

use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::debug;

use ministr_core::analytics::Analytics;
use ministr_core::embedding::Embedder;
use ministr_core::git::GitFetcher;
use ministr_core::index::VectorIndex;
use ministr_core::ingestion::{IngestionPipeline, IngestionProgress};
use ministr_core::service::QueryService;
use ministr_core::session::prefetch::PrefetchEngine;
use ministr_core::session::{AccessMode, SessionId, SessionRegistry, UsageConfig};
use ministr_core::storage::{SqliteStorage, Storage};
use ministr_core::web::fetcher::WebFetcher;

use super::MinistrServer;
use super::NegotiatedExtensions;
use super::helpers::{build_instructions, has_code_files_in_dir, uuid_v4};
use crate::task::McpTaskManager;

/// Read `MINISTR_PARENT_SESSION_ID` from the environment.
///
/// Set by a parent agent when spawning a subagent's `ministr serve`
/// process so the daemon can render the resulting session as nested
/// under its parent (rather than a flat sibling). Returns `None` when
/// unset or empty.
fn read_parent_session_env() -> Option<String> {
    std::env::var("MINISTR_PARENT_SESSION_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl MinistrServer {
    /// Create a new ministr MCP server instance backed by the given query service.
    ///
    /// Initializes session tracking and budget management with default
    /// configuration.
    #[must_use]
    pub fn new(service: Arc<QueryService>) -> Self {
        Self::with_budget_config(service, UsageConfig::default())
    }

    /// Create a new ministr MCP server with custom budget configuration.
    #[must_use]
    pub fn with_budget_config(service: Arc<QueryService>, budget_config: UsageConfig) -> Self {
        let session_id = uuid_v4();
        let mut registry = SessionRegistry::new(budget_config);
        registry.create_session(&session_id, None, AccessMode::ReadWrite);
        let backend = crate::backend::Backend::local(service.clone());
        Self {
            service: Some(service),
            backend,
            registry: Arc::new(Mutex::new(registry)),
            active_session_id: session_id,
            prefetch: Arc::new(Mutex::new(PrefetchEngine::with_default_capacity())),
            storage: None,
            analytics: None,
            web_fetcher: None,
            git_fetcher: None,
            ingestion_pipeline: Arc::new(IngestionPipeline::new()),
            embedder: None,
            index: None,
            ingestion_progress: Arc::new(IngestionProgress::new()),
            task_manager: Arc::new(McpTaskManager::new()),
            peer: Arc::new(Mutex::new(None)),
            subscriptions: Arc::new(Mutex::new(HashSet::new())),
            coherence_rx: Arc::new(Mutex::new(None)),
            negotiated_extensions: Arc::new(Mutex::new(NegotiatedExtensions::default())),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
            custom_instructions: None,
            parent_session_id_hint: read_parent_session_env(),
            client_name_hint: Arc::new(std::sync::Mutex::new(None)),
            corpus_registry: None,
        }
    }

    /// Create a server with session persistence backed by the given storage.
    ///
    /// If a session with the given ID exists in storage, it is restored.
    /// Otherwise a new session is created. Session state is persisted after
    /// each tool call that modifies session state.
    pub async fn with_persistence(
        service: Arc<QueryService>,
        budget_config: UsageConfig,
        storage: Arc<SqliteStorage>,
        session_id: Option<String>,
    ) -> Self {
        let sid = session_id.unwrap_or_else(uuid_v4);
        let session_id_obj = SessionId::from(sid.clone());

        let mut registry = SessionRegistry::new(budget_config.clone());

        match storage.load_session(&session_id_obj).await {
            Ok(Some(restored)) => {
                debug!(
                    session_id = %session_id_obj,
                    delivered_count = restored.delivered_count(),
                    "restored session from storage"
                );
                let entry =
                    registry.create_session(&sid, Some(budget_config), AccessMode::ReadWrite);
                // Replay delivered items into the budget tracker so it reflects
                // the token usage from the previous session run.
                for item in restored.delivered_items() {
                    let _ = entry
                        .budget
                        .record_tokens(item.content_id.as_ref(), item.token_count);
                }
                entry.session = restored;
            }
            _ => {
                registry.create_session(&sid, Some(budget_config), AccessMode::ReadWrite);
            }
        }

        let analytics = Arc::new(Analytics::new((*storage).clone()));
        let backend = crate::backend::Backend::local(service.clone());
        Self {
            service: Some(service),
            backend,
            registry: Arc::new(Mutex::new(registry)),
            active_session_id: sid,
            prefetch: Arc::new(Mutex::new(PrefetchEngine::with_default_capacity())),
            storage: Some(storage),
            analytics: Some(analytics),
            web_fetcher: None,
            git_fetcher: None,
            ingestion_pipeline: Arc::new(IngestionPipeline::new()),
            embedder: None,
            index: None,
            ingestion_progress: Arc::new(IngestionProgress::new()),
            task_manager: Arc::new(McpTaskManager::new()),
            peer: Arc::new(Mutex::new(None)),
            subscriptions: Arc::new(Mutex::new(HashSet::new())),
            coherence_rx: Arc::new(Mutex::new(None)),
            negotiated_extensions: Arc::new(Mutex::new(NegotiatedExtensions::default())),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
            custom_instructions: None,
            parent_session_id_hint: read_parent_session_env(),
            client_name_hint: Arc::new(std::sync::Mutex::new(None)),
            corpus_registry: None,
        }
    }

    /// Fork this server for a new MCP connection.
    ///
    /// Clones every Arc-shared field (registry, prefetch, storage, peer,
    /// etc.) so the new server observes the same daemon state, but assigns
    /// a fresh `active_session_id`. Without this, two MCP clients hitting
    /// the same primary's HTTP listener would share one session shadow —
    /// the parent's deduplication state would silently filter content from
    /// the subagent.
    ///
    /// Stays sync because rmcp's service factory is sync and the registry
    /// uses `tokio::sync::Mutex`. The fresh session is registered lazily
    /// on the first tool call via [`Self::ensure_session_mut`].
    #[must_use]
    pub fn fork_for_new_session(&self) -> Self {
        let mut forked = self.clone();
        forked.active_session_id = uuid_v4();
        forked
    }

    /// Construct a server that routes every shared MCP tool call through a
    /// running `ministr-daemon` via [`DaemonClient`], instead of an
    /// in-process [`QueryService`].
    ///
    /// This is the replacement for the old `ProxyServer`. Local-only
    /// features (the `ministr_fetch` / `ministr_clone` / `ministr_refresh`
    /// corpus-mutating tools, `ministr_task`, prefetch warming, analytics,
    /// elicitation, and the storage-backed survey helpers) are unavailable
    /// in daemon-forward mode and return errors when invoked.
    ///
    /// `corpus_id` and `session_id` must already be resolved by the caller
    /// (typically `ministr-cli/src/commands.rs::cmd_serve_proxy_stdio`,
    /// which performs the daemon-spawn handshake and corpus registration
    /// before constructing the server).
    #[must_use]
    pub fn with_daemon_backend(
        client: Arc<ministr_api::client::DaemonClient>,
        corpus_id: String,
        session_id: String,
    ) -> Self {
        let backend = crate::backend::Backend::daemon(client, corpus_id, Some(session_id.clone()));
        Self::from_daemon_pieces(backend, session_id)
    }

    /// Construct a server backed by a multi-corpus daemon backend.
    ///
    /// `session_id` here is the *primary* corpus's session — linked
    /// projects each carry their own session inside the
    /// `DaemonMultiBackend`. The primary session is the one that drives
    /// the MCP server's own session-state machinery
    /// (`registry.create_session`, `active_session_id`, etc.).
    #[must_use]
    pub fn with_daemon_multi_backend(
        multi: crate::backend::DaemonMultiBackend,
        session_id: String,
    ) -> Self {
        let backend = crate::backend::Backend::daemon_multi(multi);
        Self::from_daemon_pieces(backend, session_id)
    }

    /// Shared body for both daemon-backend constructors.
    fn from_daemon_pieces(backend: crate::backend::Backend, session_id: String) -> Self {
        let mut registry = SessionRegistry::new(UsageConfig::default());
        registry.create_session(&session_id, None, AccessMode::ReadWrite);
        Self {
            service: None,
            backend,
            registry: Arc::new(Mutex::new(registry)),
            active_session_id: session_id,
            prefetch: Arc::new(Mutex::new(PrefetchEngine::with_default_capacity())),
            storage: None,
            analytics: None,
            web_fetcher: None,
            git_fetcher: None,
            ingestion_pipeline: Arc::new(IngestionPipeline::new()),
            embedder: None,
            index: None,
            ingestion_progress: Arc::new(IngestionProgress::new()),
            task_manager: Arc::new(McpTaskManager::new()),
            peer: Arc::new(Mutex::new(None)),
            subscriptions: Arc::new(Mutex::new(HashSet::new())),
            coherence_rx: Arc::new(Mutex::new(None)),
            negotiated_extensions: Arc::new(Mutex::new(NegotiatedExtensions::default())),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
            custom_instructions: None,
            parent_session_id_hint: read_parent_session_env(),
            client_name_hint: Arc::new(std::sync::Mutex::new(None)),
            corpus_registry: None,
        }
    }

    /// Get a clone of the ingestion progress tracker for use by background tasks.
    #[must_use]
    pub fn ingestion_progress_arc(&self) -> Arc<IngestionProgress> {
        Arc::clone(&self.ingestion_progress)
    }

    /// Get the negotiated extensions for this session.
    ///
    /// Returns a snapshot of the extension negotiation result. Only meaningful
    /// after the initialization handshake has completed.
    pub async fn negotiated_extensions(&self) -> NegotiatedExtensions {
        self.negotiated_extensions.lock().await.clone()
    }

    /// Enable web fetching on this server.
    ///
    /// Sets up the `WebFetcher`, embedder, and vector index needed for the
    /// `ministr_fetch` tool. Without calling this, `ministr_fetch` returns an error.
    #[must_use]
    pub fn with_web_fetcher(
        mut self,
        web_fetcher: WebFetcher,
        embedder: Arc<dyn Embedder>,
        index: Arc<dyn VectorIndex>,
    ) -> Self {
        self.web_fetcher = Some(Arc::new(web_fetcher));
        self.embedder = Some(embedder);
        self.index = Some(index);
        self
    }

    /// Enable git cloning on this server.
    ///
    /// Sets up the `GitFetcher` needed for the `ministr_clone` tool.
    /// Also ensures embedder and index are set (needed for ingestion).
    /// Without calling this, `ministr_clone` returns an error.
    #[must_use]
    pub fn with_git_fetcher(
        mut self,
        git_fetcher: GitFetcher,
        embedder: Arc<dyn Embedder>,
        index: Arc<dyn VectorIndex>,
    ) -> Self {
        self.git_fetcher = Some(Arc::new(git_fetcher));
        if self.embedder.is_none() {
            self.embedder = Some(embedder);
        }
        if self.index.is_none() {
            self.index = Some(index);
        }
        self
    }

    /// Bridge the MCP surface to the daemon's multi-corpus registry.
    ///
    /// When `cmd_serve_http` mounts both the `/mcp` surface and the
    /// daemon's REST router (`/api/v1/corpora/*`) in the same process,
    /// it constructs a single `Arc<CorpusRegistry>` and hands the same
    /// clone to both. That way a corpus created via REST (or queried
    /// via REST) lands in the same state the MCP tools observe — a
    /// prerequisite for the multi-tenant scoping work in F1.2 and the
    /// quota middleware in F2.3.
    ///
    /// F2.x-a — when the server was constructed in `Backend::Local`
    /// mode (the cloud HTTP path), this also swaps the backend to
    /// [`crate::backend::Backend::Registry`] so per-call dispatch
    /// resolves the `project = corpus_id` argument through the shared
    /// registry instead of always answering against the startup-bound
    /// placeholder corpus. Without this swap, `ministr_survey`/`toc`/
    /// `read` against the cloud return empty even when the worker has
    /// indexed the corpus and `/api/v1/corpora` reports it present.
    ///
    /// Returns `self` so it composes with the other `with_*` builders.
    #[must_use]
    pub fn with_corpus_registry(
        mut self,
        registry: Arc<ministr_daemon::registry::CorpusRegistry>,
    ) -> Self {
        if let crate::backend::Backend::Local(local) = &self.backend {
            self.backend = crate::backend::Backend::registry(
                Arc::clone(local.service()),
                Arc::clone(&registry),
            );
        }
        self.corpus_registry = Some(registry);
        self
    }

    /// Remove tools that are irrelevant for the current corpus configuration.
    ///
    /// Inspects the server's capabilities (web fetcher, git fetcher) and scans
    /// corpus paths for code files and cross-language bridge frameworks to
    /// decide which tools to expose. Hidden tools are removed from the router
    /// so they don't appear in `tools/list`, saving agent schema tokens.
    ///
    /// Call this once after constructing the server, before the MCP handshake.
    pub fn prune_tools(&mut self, corpus_paths: &[std::path::PathBuf]) {
        let mut pruned = Vec::new();

        // Web tools: hide if no web fetcher configured
        if self.web_fetcher.is_none() {
            for name in &["ministr_fetch", "ministr_refresh"] {
                self.tool_router.remove_route(name);
                pruned.push(*name);
            }
        }

        // Git tools: hide if no git fetcher AND not running in daemon-
        // backend mode. In daemon mode `ministr_clone` works via the
        // daemon's clone-and-link endpoint regardless of local fetcher
        // state, so it stays exposed.
        let in_daemon_mode = matches!(self.backend, crate::backend::Backend::Daemon(_));
        if self.git_fetcher.is_none() && !in_daemon_mode {
            self.tool_router.remove_route("ministr_clone");
            pruned.push("ministr_clone");
        }

        // Task tool: hide if neither fetch nor clone are available (it's deprecated)
        if self.web_fetcher.is_none() && self.git_fetcher.is_none() && !in_daemon_mode {
            self.tool_router.remove_route("ministr_task");
            pruned.push("ministr_task");
        }

        // Code intelligence: hide if no code files in corpus
        let has_code = corpus_paths.iter().any(|root| has_code_files_in_dir(root));
        if !has_code {
            for name in &[
                "ministr_symbols",
                "ministr_definition",
                "ministr_references",
                "ministr_solid",
            ] {
                self.tool_router.remove_route(name);
                pruned.push(*name);
            }
        }

        // Bridge tool: hide if no cross-language bridge frameworks detected
        if has_code {
            let has_bridges = corpus_paths.iter().any(|root| {
                !ministr_core::code::bridge::detector::FrameworkDetector::detect(root).is_empty()
            });
            if !has_bridges {
                self.tool_router.remove_route("ministr_bridge");
                pruned.push("ministr_bridge");
            }
        } else {
            self.tool_router.remove_route("ministr_bridge");
            pruned.push("ministr_bridge");
        }

        if pruned.is_empty() {
            tracing::info!("tool pruning: all tools retained");
        } else {
            let remaining = self.tool_router.list_all().len();
            tracing::info!(
                pruned_count = pruned.len(),
                remaining,
                pruned_tools = ?pruned,
                "tool pruning complete — {} tools hidden",
                pruned.len(),
            );
        }

        // Build dynamic instructions that only mention registered tools.
        self.custom_instructions = Some(build_instructions(&self.tool_router));
    }

    /// Access the query service `Arc` for external use (e.g. A2A task handlers).
    ///
    /// Returns `None` when the server is running in daemon-forward mode
    /// (no local engine). Callers that require the service should error
    /// out gracefully in that case.
    #[must_use]
    pub fn service_arc(&self) -> Option<Arc<QueryService>> {
        self.service.as_ref().map(Arc::clone)
    }

    /// Access the session registry `Arc` for external use (e.g. coherence task).
    #[must_use]
    pub fn registry_arc(&self) -> Arc<Mutex<SessionRegistry>> {
        Arc::clone(&self.registry)
    }

    /// Access the shared daemon corpus registry, if one was wired via
    /// [`Self::with_corpus_registry`]. `None` for stdio / proxy
    /// transports.
    #[must_use]
    pub fn corpus_registry_arc(
        &self,
    ) -> Option<Arc<ministr_daemon::registry::CorpusRegistry>> {
        self.corpus_registry.as_ref().map(Arc::clone)
    }

    /// The active session ID for this MCP connection.
    #[must_use]
    pub fn active_session_id(&self) -> &str {
        &self.active_session_id
    }

    /// Set the coherence notification receiver for resource subscription push.
    ///
    /// The receiver carries affected section IDs from the coherence file watcher.
    /// It is consumed in `on_initialized` to spawn a task that pushes
    /// `notifications/resources/updated` to subscribed clients.
    pub fn set_coherence_receiver(&self, rx: tokio::sync::mpsc::UnboundedReceiver<Vec<String>>) {
        if let Ok(mut guard) = self.coherence_rx.try_lock() {
            *guard = Some(rx);
        }
    }

    /// Access the storage `Arc`, if persistence is enabled.
    #[must_use]
    pub fn storage_arc(&self) -> Option<Arc<SqliteStorage>> {
        self.storage.clone()
    }
}
