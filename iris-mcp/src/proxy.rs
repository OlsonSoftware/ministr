//! Thin MCP proxy that delegates to the iris daemon over UDS.
//!
//! [`ProxyServer`] implements the same MCP tools as [`IrisServer`] but
//! forwards all operations to the iris daemon via [`DaemonClient`].
//! Uses ~20 MB vs the monolithic server's ~2 GB+.

use std::sync::Arc;

use iris_api::client::DaemonClient;
use iris_core::session::{BudgetConfig, BudgetTracker, EvictionPolicy};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ErrorData as McpError;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeRequestParams, InitializeResult,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo,
};
use rmcp::schemars;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// MCP proxy server that delegates to the iris daemon.
#[derive(Clone)]
pub struct ProxyServer {
    client: Arc<DaemonClient>,
    corpus_id: Arc<Mutex<Option<String>>>,
    session_id: Arc<Mutex<Option<String>>>,
    corpus_paths: Vec<String>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    /// Local budget tracker — tracks tokens delivered through this proxy
    /// independently of the daemon's session system.
    local_budget: Arc<Mutex<BudgetTracker>>,
    /// Serializes daemon-launch attempts within this proxy process so two
    /// concurrent cold-start tool calls don't both remove the socket and
    /// spawn competing daemons.
    launch_mu: Arc<Mutex<()>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SurveyParams {
    pub query: String,
    #[serde(default)]
    pub top_k: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadParams {
    pub section_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExtractParams {
    pub section_id: String,
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolsParams {
    pub query: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DefinitionParams {
    pub symbol_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReferencesParams {
    pub symbol_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TocParams {
    #[serde(default)]
    pub document_id: Option<String>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RelatedParams {
    pub claim_id: String,
    #[serde(default)]
    pub relation_types: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BridgeParams {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub source_language: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompressParams {
    pub content_ids: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EvictedParams {
    pub content_ids: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AskParams {
    /// Natural-language question about the codebase.
    pub query: String,
}

impl ProxyServer {
    #[must_use]
    pub fn new(corpus_paths: Vec<String>) -> Self {
        let budget_config = BudgetConfig {
            max_context_tokens: 100_000,
            ..BudgetConfig::default()
        };
        Self {
            client: Arc::new(DaemonClient::new()),
            corpus_id: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
            corpus_paths,
            tool_router: Self::tool_router(),
            local_budget: Arc::new(Mutex::new(BudgetTracker::new(
                budget_config,
                EvictionPolicy::Fifo,
            ))),
            launch_mu: Arc::new(Mutex::new(())),
        }
    }

    /// Ensure the daemon is running, auto-starting it if necessary.
    ///
    /// If a stale socket is detected (file exists but daemon unresponsive),
    /// cleans up and retries the launch once. Launch attempts are
    /// serialized through `launch_mu` so concurrent first-callers can't
    /// race to both delete the socket and spawn competing daemons.
    async fn ensure_daemon(&self) -> Result<(), McpError> {
        // Fast path: socket exists and daemon responds, lock-free.
        if self.client.is_healthy().await {
            return Ok(());
        }

        // Serialize the launch path. Double-check once we hold the lock in
        // case another concurrent caller already brought the daemon up.
        let _launch_guard = self.launch_mu.lock().await;
        if self.client.is_healthy().await {
            return Ok(());
        }

        // Stale socket recovery: if file exists but daemon is dead, clean up.
        if self.client.is_socket_present() {
            warn!("daemon socket exists but is unresponsive — cleaning stale files");
            let _ = std::fs::remove_file(iris_api::daemon_socket_path());
            let _ = std::fs::remove_file(iris_api::daemon_pid_path());
        }

        self.launch_daemon().await
    }

    /// Spawn the daemon binary and wait for it to become responsive.
    async fn launch_daemon(&self) -> Result<(), McpError> {
        let daemon_bin = Self::find_daemon_binary();
        info!(bin = %daemon_bin.display(), "launching iris daemon");

        std::process::Command::new(&daemon_bin)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                McpError::internal_error(
                    format!("failed to start daemon at {}: {e}", daemon_bin.display()),
                    None,
                )
            })?;

        // Poll for the socket to appear (fast stat check).
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if self.client.is_socket_present() {
                break;
            }
        }

        // Confirm the daemon is actually responding.
        if self.client.is_healthy().await {
            info!("daemon started successfully");
            return Ok(());
        }

        Err(McpError::internal_error(
            "daemon did not become responsive within 5 seconds",
            None,
        ))
    }

    /// Find the iris-app binary by searching multiple well-known locations.
    ///
    /// Search order:
    /// 1. Same directory as the current executable (development / co-installed)
    /// 2. `~/.iris/bin/iris-app` (user install)
    /// 3. macOS app bundle: `/Applications/iris.app/Contents/MacOS/iris-app`
    /// 4. `PATH` fallback
    fn find_daemon_binary() -> std::path::PathBuf {
        // 1. Sibling of current executable.
        if let Some(sibling) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("iris-app")))
            .filter(|p| p.exists())
        {
            return sibling;
        }

        // 2. ~/.iris/bin/iris-app
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let user_bin = std::path::PathBuf::from(home)
                .join(".iris")
                .join("bin")
                .join("iris-app");
            if user_bin.exists() {
                return user_bin;
            }
        }

        // 3. macOS app bundle.
        #[cfg(target_os = "macos")]
        {
            let app_bundle =
                std::path::PathBuf::from("/Applications/iris.app/Contents/MacOS/iris-app");
            if app_bundle.exists() {
                return app_bundle;
            }
        }

        // 4. Fall back to PATH.
        std::path::PathBuf::from("iris-app")
    }

    async fn ensure_corpus(&self) -> Result<String, McpError> {
        // Serialize through the write lock so concurrent first-callers
        // don't both fire `register_corpus`. Daemon-side register is
        // idempotent, but holding the lock across the RPC avoids the
        // wasted round-trip and keeps the memoized ID deterministic.
        let mut guard = self.corpus_id.lock().await;
        if let Some(ref id) = *guard {
            return Ok(id.clone());
        }

        // `ensure_daemon` may take a while to spawn + health-check; it has
        // its own `launch_mu` that prevents a thundering herd, so holding
        // the corpus_id lock here just serializes corpus setup — exactly
        // the behaviour we want.
        self.ensure_daemon().await?;

        let resp = self
            .client
            .register_corpus(&self.corpus_paths)
            .await
            .map_err(|e| McpError::internal_error(format!("daemon: {e}"), None))?;

        *guard = Some(resp.corpus_id.clone());
        info!(corpus_id = %resp.corpus_id, "registered corpus with daemon");
        Ok(resp.corpus_id)
    }

    /// Eagerly register the corpus and create a daemon session.
    ///
    /// Call this at startup so the daemon (and GUI) can see the session
    /// immediately, rather than waiting for the first tool call.
    ///
    /// We intentionally do *not* `clear_sessions` for the corpus here —
    /// other proxies may be connected to the same corpus (GUI, second MCP
    /// client), and nuking every session on startup destroyed their
    /// turn/budget state. Crash-orphan cleanup belongs in a separate
    /// per-proxy tracking layer.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if the daemon is unreachable or registration fails.
    pub async fn initialize(&self) -> Result<(), McpError> {
        let _corpus_id = self.ensure_corpus().await?;
        let _session_id = self.ensure_session().await?;
        Ok(())
    }

    /// Destroy the daemon session created by this proxy.
    ///
    /// Best-effort cleanup — errors are logged but not propagated.
    pub async fn shutdown(&self) {
        let corpus_id = self.corpus_id.lock().await.clone();
        let session_id = self.session_id.lock().await.clone();
        if let (Some(cid), Some(sid)) = (corpus_id, session_id) {
            if let Err(e) = self.client.destroy_session(&cid, &sid).await {
                warn!(error = %e, "failed to destroy daemon session on shutdown");
            } else {
                info!(session_id = %sid, "destroyed daemon session");
            }
        }
    }

    /// Ensure a daemon session exists for this proxy, creating one lazily.
    ///
    /// Holds the write lock through the `create_session` RPC so two
    /// concurrent first-callers don't both ask the daemon to mint a
    /// session — that used to leak one orphan per race. Lock order:
    /// `corpus_id` acquired first (inside `ensure_corpus`), then
    /// `session_id` — matches everywhere else that touches both.
    async fn ensure_session(&self) -> Result<String, McpError> {
        // Grab the corpus ID before we take the session lock so we don't
        // invert lock order.
        let corpus_id = self.ensure_corpus().await?;

        let mut guard = self.session_id.lock().await;
        if let Some(ref id) = *guard {
            return Ok(id.clone());
        }

        let resp = self
            .client
            .create_session(&corpus_id, None)
            .await
            .map_err(|e| McpError::internal_error(format!("daemon session: {e}"), None))?;

        *guard = Some(resp.session_id.clone());
        info!(session_id = %resp.session_id, "created daemon session");
        Ok(resp.session_id)
    }

    /// Serialize a response into a `CallToolResult` using only the `content`
    /// field (text JSON).  We intentionally avoid `CallToolResult::structured`
    /// because it populates *both* `content` and `structured_content`, and
    /// Claude Code sends both to the model — effectively doubling every
    /// response's token cost in the conversation context.
    fn json_result<T: Serialize>(data: &T) -> Result<CallToolResult, McpError> {
        let text = serde_json::to_string(data)
            .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    fn err(e: &iris_api::client::ClientError) -> McpError {
        McpError::internal_error(e.to_string(), None)
    }
}

#[tool_router]
impl ProxyServer {
    #[tool(
        name = "iris_survey",
        description = "Search the indexed corpus for sections relevant to a natural language query. Returns ranked results with relevance scores."
    )]
    async fn survey(
        &self,
        Parameters(params): Parameters<SurveyParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = iris_api::query::SurveyRequest {
            query: params.query,
            top_k: params.top_k,
            session_id: Some(sid),
        };
        let resp = self
            .client
            .survey_req(&cid, &req)
            .await
            .map_err(|e| Self::err(&e))?;
        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_read",
        description = "Read the full content of a section by its ID."
    )]
    async fn read(
        &self,
        Parameters(params): Parameters<ReadParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let resp = self
            .client
            .session_read_section(&cid, &sid, &params.section_id)
            .await
            .map_err(|e| Self::err(&e))?;

        // Track delivered tokens locally so iris_budget reflects actual usage.
        let token_count = iris_core::token::count_tokens(&resp.text);
        {
            let mut budget = self.local_budget.lock().await;
            let _ = budget.record_tokens(&params.section_id, token_count);
        }

        // Compact: drop redundant fields and strip doc comment duplication
        // from text to save context tokens.
        let mut resp = resp;
        resp.summary = None;
        resp.budget_status = None;
        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_extract",
        description = "Extract atomic claims from a section. Optionally filter by a query."
    )]
    async fn extract(
        &self,
        Parameters(params): Parameters<ExtractParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = iris_api::query::ExtractRequest {
            section_id: params.section_id,
            query: params.query,
            session_id: Some(sid),
        };
        let resp = self
            .client
            .extract(&cid, &req)
            .await
            .map_err(|e| Self::err(&e))?;
        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_symbols",
        description = "Search for code symbols (functions, structs, traits) by name, kind, module, or visibility."
    )]
    async fn symbols(
        &self,
        Parameters(params): Parameters<SymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = iris_api::query::SymbolsRequest {
            query: params.query,
            kind: params.kind,
            module: params.module,
            visibility: params.visibility,
            limit: params.limit,
            session_id: Some(sid),
        };
        let mut resp = self
            .client
            .symbols(&cid, &req)
            .await
            .map_err(|e| Self::err(&e))?;

        // Compact: in listing context, drop source_context (already empty from
        // daemon), heading_path (derivable from file_path), and doc_comment
        // (signature is sufficient for deciding which symbol to drill into).
        for sym in &mut resp.symbols {
            sym.source_context = String::new();
            sym.heading_path.clear();
            sym.doc_comment = None;
        }

        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_definition",
        description = "Get the full source definition of a code symbol by its ID."
    )]
    async fn definition(
        &self,
        Parameters(params): Parameters<DefinitionParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let mut resp = self
            .client
            .definition(&cid, &params.symbol_id, Some(&sid))
            .await
            .map_err(|e| Self::err(&e))?;

        // Compact: when source_context is present, doc_comment and signature
        // are redundant (both appear in the source lines). heading_path is
        // derivable from file_path.
        if !resp.source_context.is_empty() {
            resp.doc_comment = None;
            resp.signature = String::new();
            resp.heading_path.clear();
        }

        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_references",
        description = "Find all callers, implementors, and importers of a code symbol."
    )]
    async fn references(
        &self,
        Parameters(params): Parameters<ReferencesParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let mut resp = self
            .client
            .references(&cid, &params.symbol_id, Some(&sid))
            .await
            .map_err(|e| Self::err(&e))?;

        // Compact: the caller already knows the target symbol — drop
        // redundant to_* fields from each reference.
        for r in &mut resp.references {
            r.to_symbol_id = String::new();
            r.to_name = String::new();
            r.to_file = String::new();
            r.to_line = 0;
        }

        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_toc",
        description = "Get a structural overview (table of contents) of the indexed corpus."
    )]
    async fn toc(
        &self,
        Parameters(params): Parameters<TocParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = iris_api::query::TocRequest {
            document_id: params.document_id,
            offset: params.offset,
            limit: params.limit,
            session_id: Some(sid),
        };
        let resp = self
            .client
            .toc(&cid, &req)
            .await
            .map_err(|e| Self::err(&e))?;
        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_related",
        description = "Find claims related to a given claim by relationship type (references, contradicts, depends_on, updates)."
    )]
    async fn related(
        &self,
        Parameters(params): Parameters<RelatedParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = iris_api::query::RelatedRequest {
            claim_id: params.claim_id,
            relation_types: params.relation_types.unwrap_or_default(),
            session_id: Some(sid),
        };
        let resp = self
            .client
            .related(&cid, &req)
            .await
            .map_err(|e| Self::err(&e))?;
        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_bridge",
        description = "Query cross-language bridge links (FFI, NAPI, PyO3, etc.) between source and target languages."
    )]
    async fn bridge(
        &self,
        Parameters(params): Parameters<BridgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = iris_api::query::BridgeRequest {
            query: params.query,
            kind: params.kind,
            source_language: params.source_language,
            limit: params.limit,
            session_id: Some(sid),
        };
        let resp = self
            .client
            .bridge(&cid, &req)
            .await
            .map_err(|e| Self::err(&e))?;
        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_budget",
        description = "Get the current context budget status: total budget, estimated usage, pressure level."
    )]
    async fn budget(&self) -> Result<CallToolResult, McpError> {
        // Use the local budget tracker — it reflects tokens delivered through
        // this proxy session, independent of the daemon's session system.
        let budget = self.local_budget.lock().await;
        let status = budget.budget_status();
        let resp = iris_api::session::SessionBudgetResponse {
            pressure_level: match status.pressure_level {
                iris_core::session::PressureLevel::Normal => "normal".into(),
                iris_core::session::PressureLevel::Elevated => "elevated".into(),
                iris_core::session::PressureLevel::Critical => "critical".into(),
            },
            tokens_used: status.tokens_used,
            tokens_remaining: status.tokens_remaining,
            utilization: status.utilization,
        };
        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_compress",
        description = "Generate compressed summaries for sections the agent wants to evict from context. Uses extractive TF-IDF compression (60-80% reduction)."
    )]
    async fn compress(
        &self,
        Parameters(params): Parameters<CompressParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = iris_api::session::CompressRequest {
            content_ids: params.content_ids,
            session_id: Some(sid),
        };
        let resp = self
            .client
            .compress(&cid, &req)
            .await
            .map_err(|e| Self::err(&e))?;
        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_evicted",
        description = "Signal that content IDs have been evicted from the agent's context window. Updates session tracking for accurate budget and deduplication."
    )]
    async fn evicted(
        &self,
        Parameters(params): Parameters<EvictedParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = iris_api::session::EvictRequest {
            content_ids: params.content_ids,
        };
        let resp = self
            .client
            .evict_content(&cid, &sid, &req)
            .await
            .map_err(|e| Self::err(&e))?;

        // Update local budget tracker so iris_budget reflects the evictions.
        {
            let mut budget = self.local_budget.lock().await;
            for id in &resp.evicted {
                budget.force_evict(id);
            }
        }

        Self::json_result(&resp)
    }

    #[tool(
        name = "iris_ask",
        description = "Ask a question about the codebase and get a synthesized answer. Uses cached sub-inference — much cheaper than manually surveying + reading multiple sections."
    )]
    async fn ask(
        &self,
        Parameters(params): Parameters<AskParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let resp = self
            .client
            .ask(&cid, &params.query, Some(&sid))
            .await
            .map_err(|e| Self::err(&e))?;
        Self::json_result(&resp)
    }
}

#[tool_handler]
impl ServerHandler for ProxyServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("iris-proxy", env!("CARGO_PKG_VERSION"))
                    .with_description("Thin MCP proxy — delegates to the iris daemon."),
            )
            .with_instructions(
                "iris proxy — use iris_survey to search, iris_read to read sections, \
                 iris_extract for claims, iris_symbols/iris_definition/iris_references \
                 for code navigation.",
            )
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        info!("proxy: initialize called — corpus registration deferred to first tool call");

        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }

        Ok(self.get_info())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult {
            prompts: vec![],
            next_cursor: None,
            meta: None,
        })
    }
}
