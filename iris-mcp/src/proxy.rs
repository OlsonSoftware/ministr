//! Thin MCP proxy that delegates to the iris daemon over UDS.
//!
//! [`ProxyServer`] implements the same MCP tools as [`IrisServer`] but
//! forwards all operations to the iris daemon via [`DaemonClient`].
//! Uses ~20 MB vs the monolithic server's ~2 GB+.

use std::sync::Arc;

use iris_api::client::DaemonClient;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Implementation, InitializeRequestParams, InitializeResult,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo,
};
use rmcp::model::ErrorData as McpError;
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

impl ProxyServer {
    pub fn new(corpus_paths: Vec<String>) -> Self {
        Self {
            client: Arc::new(DaemonClient::new()),
            corpus_id: Arc::new(Mutex::new(None)),
            corpus_paths,
            tool_router: Self::tool_router(),
        }
    }

    async fn ensure_corpus(&self) -> Result<String, McpError> {
        {
            let guard = self.corpus_id.lock().await;
            if let Some(ref id) = *guard {
                return Ok(id.clone());
            }
        }

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

    fn json_result<T: Serialize>(data: &T) -> Result<CallToolResult, McpError> {
        let v = serde_json::to_value(data)
            .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
        Ok(CallToolResult::structured(v))
    }

    fn err(e: iris_api::client::ClientError) -> McpError {
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
            .map_err(Self::err)?;
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
            .map_err(Self::err)?;
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
        };
        let resp = self.client.extract(&cid, &req).await.map_err(Self::err)?;
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
        let resp = self.client.symbols(&cid, &req).await.map_err(Self::err)?;
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
            .map_err(Self::err)?;
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
            .map_err(Self::err)?;
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
        if !self.client.is_available() {
            warn!("daemon not running at {:?}", self.client.socket_path());
        }

        if let Err(e) = self.ensure_corpus().await {
            warn!(error = %e.message, "corpus registration failed on init");
        }

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
