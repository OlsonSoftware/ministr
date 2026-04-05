//! Thin MCP proxy that delegates to the iris daemon over UDS.
//!
//! [`ProxyServer`] implements the same MCP tools as [`IrisServer`] but
//! forwards all operations to the iris daemon via [`DaemonClient`].
//! Uses ~20 MB vs the monolithic server's ~2 GB+.

use std::sync::Arc;

use iris_api::client::DaemonClient;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ErrorData as McpError;
use rmcp::model::{
    CallToolResult, Implementation, InitializeRequestParams, InitializeResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams, ServerCapabilities,
    ServerInfo,
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
    tool_router: ToolRouter<Self>,
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
        Self {
            client: Arc::new(DaemonClient::new()),
            corpus_id: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
            corpus_paths,
            tool_router: Self::tool_router(),
        }
    }

    /// Ensure the daemon is running, auto-starting it if necessary.
    ///
    /// If a stale socket is detected (file exists but daemon unresponsive),
    /// cleans up and retries the launch once.
    async fn ensure_daemon(&self) -> Result<(), McpError> {
        // Fast path: socket exists and daemon responds.
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
        {
            let guard = self.corpus_id.lock().await;
            if let Some(ref id) = *guard {
                return Ok(id.clone());
            }
        }

        self.ensure_daemon().await?;

        let resp = self
            .client
            .register_corpus(&self.corpus_paths)
            .await
            .map_err(|e| McpError::internal_error(format!("daemon: {e}"), None))?;

        let mut guard = self.corpus_id.lock().await;
        *guard = Some(resp.corpus_id.clone());
        info!(corpus_id = %resp.corpus_id, "registered corpus with daemon");
        Ok(resp.corpus_id)
    }

    /// Eagerly register the corpus and create a daemon session.
    ///
    /// Call this at startup so the daemon (and GUI) can see the session
    /// immediately, rather than waiting for the first tool call.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if the daemon is unreachable or registration fails.
    pub async fn initialize(&self) -> Result<(), McpError> {
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
    async fn ensure_session(&self) -> Result<String, McpError> {
        {
            let guard = self.session_id.lock().await;
            if let Some(ref id) = *guard {
                return Ok(id.clone());
            }
        }

        let corpus_id = self.ensure_corpus().await?;
        let resp = self
            .client
            .create_session(&corpus_id, None)
            .await
            .map_err(|e| McpError::internal_error(format!("daemon session: {e}"), None))?;

        let mut guard = self.session_id.lock().await;
        *guard = Some(resp.session_id.clone());
        info!(session_id = %resp.session_id, "created daemon session");
        Ok(resp.session_id)
    }

    fn json_result<T: Serialize>(data: &T) -> Result<CallToolResult, McpError> {
        let v = serde_json::to_value(data)
            .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
        Ok(CallToolResult::structured(v))
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
        let resp = self
            .client
            .survey(&cid, &params.query, params.top_k)
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
        let resp = self
            .client
            .read_section(&cid, &params.section_id)
            .await
            .map_err(|e| Self::err(&e))?;
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
        let req = iris_api::query::ExtractRequest {
            section_id: params.section_id,
            query: params.query,
            session_id: None,
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
        let req = iris_api::query::SymbolsRequest {
            query: params.query,
            kind: params.kind,
            module: params.module,
            visibility: params.visibility,
            limit: params.limit,
        };
        let resp = self
            .client
            .symbols(&cid, &req)
            .await
            .map_err(|e| Self::err(&e))?;
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
        let resp = self
            .client
            .definition(&cid, &params.symbol_id)
            .await
            .map_err(|e| Self::err(&e))?;
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
        let resp = self
            .client
            .references(&cid, &params.symbol_id)
            .await
            .map_err(|e| Self::err(&e))?;
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
        let req = iris_api::query::TocRequest {
            document_id: params.document_id,
            offset: params.offset,
            limit: params.limit,
            session_id: None,
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
        let req = iris_api::query::RelatedRequest {
            claim_id: params.claim_id,
            relation_types: params.relation_types.unwrap_or_default(),
            session_id: None,
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
        let req = iris_api::query::BridgeRequest {
            query: params.query,
            kind: params.kind,
            source_language: params.source_language,
            limit: params.limit,
            session_id: None,
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
        let cid = self.ensure_corpus().await?;
        let sid = self.ensure_session().await?;
        let resp = self
            .client
            .session_budget(&cid, &sid)
            .await
            .map_err(|e| Self::err(&e))?;
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
        let req = iris_api::session::CompressRequest {
            content_ids: params.content_ids,
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
