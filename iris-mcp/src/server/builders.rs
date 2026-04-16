//! Constructors, builders, and accessor methods for [`IrisServer`].
//!
//! Separates the construction and configuration API from the MCP handler
//! logic, making it easier to understand the server's public surface.

use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::debug;

use iris_core::analytics::Analytics;
use iris_core::embedding::Embedder;
use iris_core::git::GitFetcher;
use iris_core::index::VectorIndex;
use iris_core::ingestion::{IngestionPipeline, IngestionProgress};
use iris_core::service::QueryService;
use iris_core::session::prefetch::PrefetchEngine;
use iris_core::session::{AccessMode, BudgetConfig, SessionId, SessionRegistry};
use iris_core::storage::{SqliteStorage, Storage};
use iris_core::web::fetcher::WebFetcher;

use super::IrisServer;
use super::NegotiatedExtensions;
use super::helpers::{build_instructions, has_code_files_in_dir, uuid_v4};
use crate::task::McpTaskManager;

impl IrisServer {
    /// Create a new iris MCP server instance backed by the given query service.
    ///
    /// Initializes session tracking and budget management with default
    /// configuration.
    #[must_use]
    pub fn new(service: Arc<QueryService>) -> Self {
        Self::with_budget_config(service, BudgetConfig::default())
    }

    /// Create a new iris MCP server with custom budget configuration.
    #[must_use]
    pub fn with_budget_config(service: Arc<QueryService>, budget_config: BudgetConfig) -> Self {
        let session_id = uuid_v4();
        let mut registry = SessionRegistry::new(budget_config);
        registry.create_session(&session_id, None, AccessMode::ReadWrite);
        Self {
            service,
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
        }
    }

    /// Create a server with session persistence backed by the given storage.
    ///
    /// If a session with the given ID exists in storage, it is restored.
    /// Otherwise a new session is created. Session state is persisted after
    /// each tool call that modifies session state.
    pub async fn with_persistence(
        service: Arc<QueryService>,
        budget_config: BudgetConfig,
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
        Self {
            service,
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
    /// `iris_fetch` tool. Without calling this, `iris_fetch` returns an error.
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
    /// Sets up the `GitFetcher` needed for the `iris_clone` tool.
    /// Also ensures embedder and index are set (needed for ingestion).
    /// Without calling this, `iris_clone` returns an error.
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
            for name in &["iris_fetch", "iris_refresh"] {
                self.tool_router.remove_route(name);
                pruned.push(*name);
            }
        }

        // Git tools: hide if no git fetcher configured
        if self.git_fetcher.is_none() {
            self.tool_router.remove_route("iris_clone");
            pruned.push("iris_clone");
        }

        // Task tool: hide if neither fetch nor clone are available (it's deprecated)
        if self.web_fetcher.is_none() && self.git_fetcher.is_none() {
            self.tool_router.remove_route("iris_task");
            pruned.push("iris_task");
        }

        // Code intelligence: hide if no code files in corpus
        let has_code = corpus_paths.iter().any(|root| has_code_files_in_dir(root));
        if !has_code {
            for name in &["iris_symbols", "iris_definition", "iris_references"] {
                self.tool_router.remove_route(name);
                pruned.push(*name);
            }
        }

        // Bridge tool: hide if no cross-language bridge frameworks detected
        if has_code {
            let has_bridges = corpus_paths.iter().any(|root| {
                !iris_core::code::bridge::detector::FrameworkDetector::detect(root).is_empty()
            });
            if !has_bridges {
                self.tool_router.remove_route("iris_bridge");
                pruned.push("iris_bridge");
            }
        } else {
            self.tool_router.remove_route("iris_bridge");
            pruned.push("iris_bridge");
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
    #[must_use]
    pub fn service_arc(&self) -> Arc<QueryService> {
        Arc::clone(&self.service)
    }

    /// Access the session registry `Arc` for external use (e.g. coherence task).
    #[must_use]
    pub fn registry_arc(&self) -> Arc<Mutex<SessionRegistry>> {
        Arc::clone(&self.registry)
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
