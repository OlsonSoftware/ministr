//! Thin MCP proxy that delegates to the ministr daemon over UDS.
//!
//! [`ProxyServer`] implements the same MCP tools as [`MinistrServer`] but
//! forwards all operations to the ministr daemon via [`DaemonClient`].
//! Uses ~20 MB vs the monolithic server's ~2 GB+.

use std::sync::Arc;

use ministr_api::client::DaemonClient;
use ministr_core::session::{BudgetConfig, BudgetTracker, EvictionPolicy};
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

/// Apply platform-specific flags so a spawned child runs detached, in the
/// background, with no visible console or window of its own.
///
/// On Windows: `CREATE_NO_WINDOW` suppresses any console window for
/// console-subsystem children (a no-op for `windows_subsystem = "windows"`
/// binaries, but covers debug builds and any other console binary in the
/// search path); `DETACHED_PROCESS` cuts the inherited console handle so
/// the child never tries to draw on the proxy's stdio.
///
/// On Unix: `process_group(0)` puts the child in its own process group,
/// so the proxy exiting (or receiving a signal) doesn't propagate.
fn configure_detached_spawn(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW = 0x0800_0000, DETACHED_PROCESS = 0x0000_0008
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    // Other platforms (wasi, etc.): no detachment primitive needed.
    let _ = cmd;
}

/// MCP proxy server that delegates to the ministr daemon.
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

impl ProxyServer {
    #[must_use]
    pub fn new(corpus_paths: Vec<String>) -> Self {
        // Inherit the env-driven window (MINISTR_CONTEXT_WINDOW, else the
        // 200k fallback) rather than pinning a misleadingly small 100k.
        let budget_config = BudgetConfig::default();
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

        // Stale endpoint recovery: if the endpoint is present but the
        // daemon isn't responding, clean up whatever needs cleaning.
        // Only Unix leaves a socket-file artifact — Windows named pipes
        // vanish with the owning process, so the `remove_file` path
        // doesn't apply there. PID file cleanup runs on both.
        if self.client.is_endpoint_present() {
            warn!("daemon endpoint is present but unresponsive — cleaning stale files");
            #[cfg(unix)]
            let _ = std::fs::remove_file(ministr_api::daemon_socket_path());
            let _ = std::fs::remove_file(ministr_api::daemon_pid_path());
        }

        self.launch_daemon().await
    }

    /// Spawn the daemon binary and wait for it to become responsive.
    ///
    /// The child is spawned fully detached from the proxy on every platform:
    /// no console window flashes on Windows, the process is in its own
    /// process group on Unix so the parent dying doesn't take it down, and
    /// stdio is null so nothing leaks into the MCP transport.
    async fn launch_daemon(&self) -> Result<(), McpError> {
        let daemon_bin = Self::find_daemon_binary();
        info!(bin = %daemon_bin.display(), "launching ministr daemon");

        let mut cmd = std::process::Command::new(&daemon_bin);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        configure_detached_spawn(&mut cmd);

        cmd.spawn().map_err(|e| {
            McpError::internal_error(
                format!("failed to start daemon at {}: {e}", daemon_bin.display()),
                None,
            )
        })?;

        // Poll for the endpoint to appear (fast stat / pipe-metadata check).
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if self.client.is_endpoint_present() {
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

    /// Find the ministr-app binary by searching multiple well-known locations.
    ///
    /// Search order:
    /// 1. Same directory as the current executable (development / co-installed)
    /// 2. `~/.ministr/bin/ministr-app` (user install)
    /// 3. macOS app bundle: `/Applications/ministr.app/Contents/MacOS/ministr-app`
    /// 4. `PATH` fallback
    fn find_daemon_binary() -> std::path::PathBuf {
        // 1. Sibling of current executable.
        if let Some(sibling) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("ministr-app")))
            .filter(|p| p.exists())
        {
            return sibling;
        }

        // 2. ~/.ministr/bin/ministr-app
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let user_bin = std::path::PathBuf::from(home)
                .join(".ministr")
                .join("bin")
                .join("ministr-app");
            if user_bin.exists() {
                return user_bin;
            }
        }

        // 3. macOS app bundle.
        #[cfg(target_os = "macos")]
        {
            let app_bundle =
                std::path::PathBuf::from("/Applications/ministr.app/Contents/MacOS/ministr-app");
            if app_bundle.exists() {
                return app_bundle;
            }
        }

        // 4. Fall back to PATH.
        std::path::PathBuf::from("ministr-app")
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

    fn err(e: &ministr_api::client::ClientError) -> McpError {
        McpError::internal_error(e.to_string(), None)
    }
}

#[tool_router]
impl ProxyServer {
    #[tool(
        name = "ministr_survey",
        description = "Search the indexed corpus by natural-language query. Start here for any vague question; follow up with ministr_read (full text) or ministr_extract (atomic claims) on top results."
    )]
    async fn survey(
        &self,
        Parameters(params): Parameters<SurveyParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = ministr_api::query::SurveyRequest {
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
        name = "ministr_read",
        description = "Full content of a section by ID, with delta delivery for changed content and short stubs for unchanged re-requests. Call ministr_extract instead if you only need atomic claims."
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

        // Track delivered tokens locally so ministr_budget reflects actual usage.
        let token_count = ministr_core::token::count_tokens(&resp.text);
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
        name = "ministr_extract",
        description = "Atomic claims from a section, optionally query-filtered. Cheaper than ministr_read when you don't need full prose."
    )]
    async fn extract(
        &self,
        Parameters(params): Parameters<ExtractParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = ministr_api::query::ExtractRequest {
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
        name = "ministr_symbols",
        description = "Find code symbols (functions, structs, traits, etc.) by name, kind, module, or visibility. Pair with ministr_definition for source and ministr_references before modifying."
    )]
    async fn symbols(
        &self,
        Parameters(params): Parameters<SymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = ministr_api::query::SymbolsRequest {
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
        name = "ministr_definition",
        description = "Full source of a code symbol by ID. Call ministr_references first if you intend to modify or delete the symbol."
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
        name = "ministr_references",
        description = "All callers, implementors, and importers of a code symbol. Call before deleting or significantly modifying any non-trivial public symbol — zero references means safe to delete."
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

    // TODO(toc-schema-convergence): the standalone `MinistrServer::toc` emits
    // `ToolResponse<TocResponse>` with `{ corpus_stats, roots, entries }` and the
    // rich per-entry shape from `ministr_core::types::TocEntry`. This proxy path
    // forwards `ministr_api::query::TocResponse`'s flatter `{ entries, total }`
    // with the lossy `{ id, title, kind, depth, children, source_path }` entry
    // shape. To unify: extend `ministr_api::query::{TocEntry, TocResponse}` with
    // the rich fields, update `ministr_daemon::convert::toc_entry` to preserve
    // them, compute `corpus_stats`/`roots` in the daemon's `toc` handler, and
    // wrap the response here in the same envelope `MinistrServer` builds.
    // Tracked alongside the doc note in `docs-next/content/docs/tools/toc.mdx`.
    #[tool(
        name = "ministr_toc",
        description = "Structural overview (table of contents) of the indexed corpus. Use to orient on an unfamiliar codebase before drilling in."
    )]
    async fn toc(
        &self,
        Parameters(params): Parameters<TocParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = ministr_api::query::TocRequest {
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
        name = "ministr_related",
        description = "Follow relationship edges (references, contradicts, depends_on, updates) from a claim. Use when one claim's truth depends on another."
    )]
    async fn related(
        &self,
        Parameters(params): Parameters<RelatedParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = ministr_api::query::RelatedRequest {
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
        name = "ministr_bridge",
        description = "Cross-language bridge links (Tauri commands, NAPI exports, PyO3 functions, FFI, HTTP routes, etc.). Call before modifying any IPC or FFI boundary so you see every cross-language call site."
    )]
    async fn bridge(
        &self,
        Parameters(params): Parameters<BridgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = ministr_api::query::BridgeRequest {
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
        name = "ministr_budget",
        description = "Internal ministr budget bookkeeping (token estimate + eviction candidates). Advisory only: the figures are anchored to a configured window, not your real model context window, so do NOT use them to decide you are low on context or to stop work. Safe to ignore."
    )]
    async fn budget(&self) -> Result<CallToolResult, McpError> {
        // Use the local budget tracker — it reflects tokens delivered through
        // this proxy session, independent of the daemon's session system.
        let budget = self.local_budget.lock().await;
        let status = budget.budget_status();
        let resp = ministr_api::session::SessionBudgetResponse {
            pressure_level: match status.pressure_level {
                ministr_core::session::PressureLevel::Normal => "normal".into(),
                ministr_core::session::PressureLevel::Elevated => "elevated".into(),
                ministr_core::session::PressureLevel::Critical => "critical".into(),
            },
            tokens_used: status.tokens_used,
            tokens_remaining: status.tokens_remaining,
            utilization: status.utilization,
        };
        Self::json_result(&resp)
    }

    #[tool(
        name = "ministr_compress",
        description = "Extractive TF-IDF summaries (60-80% reduction) for sections you intend to evict. Pair with ministr_evicted after dropping the originals from context."
    )]
    async fn compress(
        &self,
        Parameters(params): Parameters<CompressParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = ministr_api::session::CompressRequest {
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
        name = "ministr_evicted",
        description = "Call immediately after dropping content from your context window. Keeps dedup and budget tracking accurate; without this, future ministr_read calls on dropped IDs return short 'already_delivered' stubs instead of the actual text."
    )]
    async fn evicted(
        &self,
        Parameters(params): Parameters<EvictedParams>,
    ) -> Result<CallToolResult, McpError> {
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let req = ministr_api::session::EvictRequest {
            content_ids: params.content_ids,
        };
        let resp = self
            .client
            .evict_content(&cid, &sid, &req)
            .await
            .map_err(|e| Self::err(&e))?;

        // Update local budget tracker so ministr_budget reflects the evictions.
        {
            let mut budget = self.local_budget.lock().await;
            for id in &resp.evicted {
                budget.force_evict(id);
            }
        }

        Self::json_result(&resp)
    }
}

#[tool_handler]
impl ServerHandler for ProxyServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("ministr-proxy", env!("CARGO_PKG_VERSION"))
                    .with_description("Thin MCP proxy — delegates to the ministr daemon."),
            )
            .with_instructions(crate::server::DEFAULT_INSTRUCTIONS)
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
