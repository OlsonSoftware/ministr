//! MCP server implementation for ministr.
//!
//! Implements the rmcp `ServerHandler` trait with `#[tool]` macro-based
//! tool registration. The server exposes ministr tools (`ministr_survey`,
//! `ministr_read`, `ministr_extract`, `ministr_related`, `ministr_dropped`,
//! `ministr_usage`, `ministr_compress`, `ministr_toc`, `ministr_fetch`,
//! `ministr_refresh`, `ministr_clone`, `ministr_task`) over the MCP protocol.
//!
//! Every tool response includes a `usage_status` object with the current
//! token budget state. Survey and read responses are deduplicated against
//! the session shadow to avoid re-delivering content the agent already has.
//!
//! When the agent re-requests unchanged content, ministr treats it as a
//! fault-based eviction signal — the agent's context window dropped the
//! content before our estimator predicted. The window estimate is corrected
//! and the content is re-delivered. The `ministr_dropped` tool accepts explicit
//! eviction feedback from the agent.

mod builders;
mod helpers;
mod prefetch;
mod progress;
mod refresh;
mod resources;
mod session;
pub mod types;

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::Arc;

use rmcp::RoleServer;
use rmcp::ServerHandler;
use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ErrorData as McpError;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, CancelTaskParams, CompleteRequestParams, CompleteResult,
    CompletionInfo, Content, CreateTaskResult, ExtensionCapabilities, GetPromptRequestParams,
    GetPromptResult, GetTaskInfoParams, GetTaskPayloadResult, GetTaskResult, GetTaskResultParams,
    Implementation, InitializeRequestParams, InitializeResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListTasksResult, PaginatedRequestParams,
    PromptMessage, PromptMessageRole, RawResource, RawResourceTemplate, ReadResourceRequestParams,
    ReadResourceResult, Reference, Resource, ResourceTemplate, ServerCapabilities, ServerInfo,
    SubscribeRequestParams, UnsubscribeRequestParams,
};
use rmcp::schemars;
use rmcp::service::{NotificationContext, Peer, RequestContext};
use rmcp::{prompt, prompt_handler, prompt_router, tool, tool_handler, tool_router};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug, info_span, warn};

use ministr_core::analytics::Analytics;
use ministr_core::embedding::Embedder;
use ministr_core::git::GitFetcher;
use ministr_core::index::VectorIndex;
use ministr_core::ingestion::{IngestionPipeline, IngestionProgress};
use ministr_core::service::QueryService;
use ministr_core::session::prefetch::PrefetchEngine;
use ministr_core::session::{SessionRegistry, UsageLevel};
use ministr_core::storage::{SqliteStorage, Storage, SymbolFilter};
use ministr_core::token::count_tokens;
use ministr_core::types::{
    ContentId, RefKind, RelationType, Resolution, SectionId, parent_section_id,
};
use ministr_core::web::fetcher::WebFetcher;

use helpers::{
    MAX_INTENT_PREFETCH_SURVEY, content_hash, format_query_error, parse_resolution,
    structured_result,
};
use progress::{run_ingestion_progress_notifier, run_subscription_notifier};
use types::{
    AlreadyDeliveredResponse, BridgeEndpointSummary, BridgeLinkSummary, BridgeParams,
    BridgeResponse, CloneOutputData, CloneParams, CompressParams, CompressResponse,
    CorpusStatsHeader, DefinitionParams, DroppedParams, DroppedResponse, ExtractParams,
    ExtractResponse, FetchOutputData, FetchParams, FetchResponse, NextAction, ReadOutputData,
    ReadParams, ReferencesParams, ReferencesResponse, RefreshParams, RefreshResponse,
    RelatedParams, RelatedResponse, SessionMetricsResponse, SurveyParams, SurveyResponse,
    SymbolSummary, SymbolsParams, SymbolsResponse, TaskParams, TaskStatusResponse, TocParams,
    TocResponse, ToolResponse, UsageResponse, tool_output_schema,
};

use crate::task::{McpTaskManager, task_to_cancel_result, task_to_get_result};

/// Server-level instructions surfaced to MCP clients during initialization.
///
/// This is loaded into the agent's system prompt by spec-compliant clients
/// (Claude Code, mcp-inspector, etc.). It is the canonical place to teach
/// the agent how to use ministr's tools effectively, which downstream
/// consumers cannot get from the project's `CLAUDE.md` (that file is only
/// loaded when editing ministr itself).
pub(crate) const DEFAULT_INSTRUCTIONS: &str = "\
ministr is a code intelligence MCP server. It gives you AST-level \
understanding of the codebase: semantic search, symbol navigation, \
real reference graphs, and cross-language bridge detection — not text \
matching. Prefer it over Read/Grep/Glob for any exploration. As a bonus, \
ministr remembers what it has already shown you this session, so re-asking \
for the same content costs almost nothing and you only get back what changed.

# Where to start
- Vague concept question → ministr_survey
- Know the symbol name → ministr_symbols, then ministr_definition
- Know the file → ministr_toc, then ministr_read (or ministr_extract for atomic claims)
- Need project layout → ministr_toc
- Following claim dependencies → ministr_related

# Before you mutate code
- Before deleting or significantly modifying a public symbol → ministr_references first. \
Zero references means safe to delete; non-zero means you have to update each call site.
- Before changing any IPC or FFI boundary (Tauri command, NAPI export, PyO3 fn, HTTP route, etc.) \
→ ministr_bridge first to see every cross-language call site.

# Read the response wrapper
Some tool responses include metadata worth reacting to:
- `coherence_alerts` non-empty → underlying file changed since last delivery; \
re-call ministr_read on the listed sections to get the delta.
- `indexing_in_progress: true` → results may be incomplete; consider re-running \
search-style tools when it clears.
- `next_actions` array → concrete suggested next tool calls with arguments and reasons. \
Treat as advisory but high-signal; the server picked them based on session state.

Note: ministr tracks delivery internally only to avoid re-sending content \
you already have. It does not, and is not designed to, tell you how full \
your context window is — any internal figures are anchored to a configured \
window, not your real model context window. Manage your own context as you \
normally would; do not treat ministr as a signal that you are low on room.

# Anti-patterns
- Don't shell out to grep/rg/find/ag/cat for search — use ministr_survey or ministr_symbols.
- Don't Read a file just to explore — use ministr_read so ministr can track what \
you've seen and return only the delta on later calls.
";

/// Minimum survey score for a top-result follow-up suggestion.
///
/// Below this, the top hit is too uncertain to be worth nudging the agent
/// to read it — the survey ranking signal is already noisy in that range.
const TOP_HIT_FOLLOWUP_THRESHOLD: f32 = 0.5;

/// Suggest a follow-up read on a survey's top result when it's confidently
/// above noise. Symbol-resolution hits route to `ministr_definition`;
/// section/claim/summary hits route to `ministr_read` on the section ID.
fn top_hit_next_action(results: &[ministr_core::service::SurveyResult]) -> Vec<NextAction> {
    let Some(top) = results.first() else {
        return Vec::new();
    };
    if top.score < TOP_HIT_FOLLOWUP_THRESHOLD {
        return Vec::new();
    }
    if top.resolution.starts_with("symbol_") {
        vec![NextAction {
            action: "ministr_definition".to_string(),
            args: serde_json::json!({ "symbol_id": top.content_id }),
            reason: format!(
                "Top survey match (score {:.2}) — fetch full definition",
                top.score
            ),
        }]
    } else {
        // Claim hits use a parent section ID for the read; everything else
        // already names a section. Fall back to the content_id directly.
        let section_id = if top.resolution == "claim" {
            ministr_core::types::parent_section_id(&top.content_id)
                .map_or_else(|| top.content_id.clone(), str::to_string)
        } else {
            top.content_id.clone()
        };
        vec![NextAction {
            action: "ministr_read".to_string(),
            args: serde_json::json!({ "section_id": section_id }),
            reason: format!(
                "Top survey match (score {:.2}) — read full section",
                top.score
            ),
        }]
    }
}

/// Suggest fetching a definition when `ministr_symbols` returned exactly
/// one match — the agent almost always wants the source next.
fn single_symbol_next_action(symbols: &[SymbolSummary]) -> Vec<NextAction> {
    if symbols.len() != 1 {
        return Vec::new();
    }
    let only = &symbols[0];
    vec![NextAction {
        action: "ministr_definition".to_string(),
        args: serde_json::json!({ "symbol_id": only.id }),
        reason: format!("Single match for `{}` — fetch full source", only.name),
    }]
}

// ── ministr MCP extension identifiers (SEP-1724) ──────────────────────────

/// Extension identifier for the ministr budget protocol.
///
/// Advertises that the server provides per-response budget status snapshots,
/// proactive eviction recommendations at elevated pressure, and context-window
/// token accounting.
pub const EXT_USAGE_PROTOCOL: &str = "dev.ministr/usage-protocol";

/// Extension identifier for ministr coherence notifications.
///
/// Advertises that the server emits `notifications/resources/updated` when
/// underlying corpus files change, enabling agents to refresh stale context.
pub const EXT_COHERENCE: &str = "dev.ministr/coherence";

/// Extension identifier for ministr multi-tier compression.
///
/// Advertises that the server supports compressing context at multiple
/// granularity levels: full text, atomic claims, and summaries.
pub const EXT_COMPRESSION: &str = "dev.ministr/compression";

/// Build the `ExtensionCapabilities` map advertising all ministr extensions.
fn ministr_extension_capabilities() -> ExtensionCapabilities {
    let mut extensions = ExtensionCapabilities::new();
    extensions.insert(
        EXT_USAGE_PROTOCOL.to_string(),
        serde_json::from_value(serde_json::json!({ "version": "1" }))
            .expect("static JSON is valid"),
    );
    extensions.insert(
        EXT_COHERENCE.to_string(),
        serde_json::from_value(serde_json::json!({ "version": "1" }))
            .expect("static JSON is valid"),
    );
    extensions.insert(
        EXT_COMPRESSION.to_string(),
        serde_json::from_value(serde_json::json!({
            "version": "1",
            "tiers": ["summary", "claims", "full"]
        }))
        .expect("static JSON is valid"),
    );
    extensions
}

/// Result of extension negotiation during the initialization handshake.
///
/// Each flag is `true` when **both** client and server advertise support for
/// the corresponding extension. Tools can check these flags to adapt their
/// behavior — e.g. omitting eviction recommendations when the client does not
/// understand the budget protocol.
#[derive(Debug, Clone, Default)]
pub struct NegotiatedExtensions {
    /// Client understands per-response budget status and eviction recommendations.
    pub usage_protocol: bool,
    /// Client can handle coherence change notifications via resource subscriptions.
    pub coherence: bool,
    /// Client supports multi-tier compression (summary / claims / full).
    pub compression: bool,
}

impl NegotiatedExtensions {
    /// Negotiate extensions by intersecting server-advertised extensions with
    /// the client's declared extension support.
    fn negotiate(client_extensions: Option<&ExtensionCapabilities>) -> Self {
        let Some(client) = client_extensions else {
            return Self::default();
        };
        Self {
            usage_protocol: client.contains_key(EXT_USAGE_PROTOCOL),
            coherence: client.contains_key(EXT_COHERENCE),
            compression: client.contains_key(EXT_COMPRESSION),
        }
    }
}

/// MCP server that exposes ministr context-cache tools to LLM agents.
///
/// `MinistrServer` adapts the [`QueryService`] to the MCP protocol.
/// It handles tool registration, request routing, and response formatting.
/// Tracks session state for deduplication and budget management.
#[derive(Clone)]
pub struct MinistrServer {
    service: Arc<QueryService>,
    /// Federated session registry managing all agent sessions.
    registry: Arc<Mutex<SessionRegistry>>,
    /// ID of the active session for this MCP connection.
    active_session_id: String,
    prefetch: Arc<Mutex<PrefetchEngine>>,
    storage: Option<Arc<SqliteStorage>>,
    analytics: Option<Arc<Analytics>>,
    web_fetcher: Option<Arc<WebFetcher>>,
    git_fetcher: Option<Arc<GitFetcher>>,
    ingestion_pipeline: Arc<IngestionPipeline>,
    embedder: Option<Arc<dyn Embedder>>,
    index: Option<Arc<dyn VectorIndex>>,
    /// Shared ingestion progress tracker.
    ingestion_progress: Arc<IngestionProgress>,
    /// MCP peer for sending server-initiated notifications.
    peer: Arc<Mutex<Option<Peer<RoleServer>>>>,
    /// Resource URIs that the client has subscribed to for change notifications.
    subscriptions: Arc<Mutex<HashSet<String>>>,
    /// Receiver for coherence change notifications from the file watcher task.
    /// Consumed once in `on_initialized` to spawn the notification dispatcher.
    coherence_rx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Vec<String>>>>>,
    /// MCP Tasks manager for async fetch/clone operations (SEP-1686).
    task_manager: Arc<McpTaskManager>,
    /// Result of MCP extension negotiation (SEP-1724).
    ///
    /// Populated during the initialization handshake by intersecting
    /// server-advertised ministr extensions with the client's declared support.
    negotiated_extensions: Arc<Mutex<NegotiatedExtensions>>,
    /// Macro-generated tool router for dispatching tool calls.
    tool_router: ToolRouter<Self>,
    /// Macro-generated prompt router for dispatching prompt requests.
    #[allow(dead_code)]
    prompt_router: PromptRouter<Self>,
    /// Dynamic instructions string, updated by `prune_tools()` to only
    /// mention tools that are actually registered.
    custom_instructions: Option<String>,
    /// Parent session id captured at startup from
    /// `MINISTR_PARENT_SESSION_ID`. Stamped onto the
    /// [`SessionEntry::parent_session_id`] when the session is first
    /// resolved via [`Self::ensure_session_mut`]. Used by the tray and
    /// `SessionDashboard` to render subagent rows nested under their
    /// parent.
    parent_session_id_hint: Option<String>,
    /// MCP `clientInfo.name` captured during the `initialize`
    /// handshake. Stamped onto [`SessionEntry::client_name`] the first
    /// time a session entry is resolved with the field still empty.
    ///
    /// `std::sync::Mutex` (not `tokio::sync::Mutex`) so
    /// `ensure_session_mut` can read it without yielding inside the
    /// registry's tokio mutex hold — the lock is brief and never held
    /// across an `.await`.
    client_name_hint: Arc<std::sync::Mutex<Option<String>>>,
}

#[tool_handler]
#[prompt_handler]
impl ServerHandler for MinistrServer {
    fn get_info(&self) -> ServerInfo {
        let instructions = self
            .custom_instructions
            .as_deref()
            .unwrap_or(DEFAULT_INSTRUCTIONS);

        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_subscribe()
                .enable_tasks()
                .enable_prompts()
                .enable_completions()
                .enable_extensions_with(ministr_extension_capabilities())
                .build(),
        )
        .with_server_info(
            Implementation::new("ministr", env!("CARGO_PKG_VERSION")).with_description(
                "Code intelligence MCP server for AI coding agents — semantic \
                     search, symbol navigation, reference graphs, and \
                     cross-language bridge detection.",
            ),
        )
        .with_instructions(instructions)
    }

    // ── Extension Negotiation (SEP-1724) ─────────────────────────────

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        // Negotiate ministr extensions by intersecting our capabilities with the
        // client's declared extension support.
        let negotiated = NegotiatedExtensions::negotiate(request.capabilities.extensions.as_ref());
        tracing::info!(
            budget = negotiated.usage_protocol,
            coherence = negotiated.coherence,
            compression = negotiated.compression,
            "extension negotiation complete"
        );
        *self.negotiated_extensions.lock().await = negotiated;

        // Capture clientInfo.name for the tray / SessionDashboard so the
        // user can tell e.g. claude-code from claude-subagent from
        // mcp-inspector apart. Hint is stamped onto the session entry on
        // first tool call via `ensure_session_mut`.
        let client_name = request.client_info.name.clone();
        if !client_name.is_empty()
            && let Ok(mut guard) = self.client_name_hint.lock()
        {
            *guard = Some(client_name);
        }

        // Preserve the default rmcp behavior: store peer info for later access.
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }

        Ok(self.get_info())
    }

    // ── MCP Tasks (SEP-1686) ────────────────────────────────────────

    async fn enqueue_task(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CreateTaskResult, McpError> {
        // Capture the cancellation token from the request context so that both
        // `notifications/cancelled` (rmcp transport-level) and `tasks/cancel`
        // (MCP task-level) can signal the same token.
        let ct = context.ct.clone();

        // Create the task record first, then dispatch to the tool.
        let task = self
            .task_manager
            .create(&format!("starting {}", request.name), None, Some(ct))
            .await;
        let task_id = task.task_id.clone();
        let task_mgr = Arc::clone(&self.task_manager);

        // Clone self for the spawned closure.
        let server = self.clone();
        let handle = tokio::spawn(async move {
            // Execute the tool call synchronously in the background.
            match server.call_tool(request, context).await {
                Ok(result) => {
                    task_mgr.complete(&task_id, result).await;
                }
                Err(e) => {
                    task_mgr
                        .fail(&task_id, &format!("tool call failed: {e}"))
                        .await;
                }
            }
        });

        // Attach the join handle so cancellation works.
        self.task_manager
            .set_join_handle(&task.task_id, handle)
            .await;

        Ok(CreateTaskResult::new(task))
    }

    async fn list_tasks(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListTasksResult, McpError> {
        let task_list = self.task_manager.list_tasks().await;
        Ok(ListTasksResult::new(task_list.tasks))
    }

    async fn get_task_info(
        &self,
        request: GetTaskInfoParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, McpError> {
        self.task_manager
            .get_task(&request.task_id)
            .await
            .map(task_to_get_result)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "unknown task ID: {}. Tasks expire 5 minutes after completion.",
                        request.task_id
                    ),
                    None,
                )
            })
    }

    async fn get_task_result(
        &self,
        request: GetTaskResultParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetTaskPayloadResult, McpError> {
        // First check the task exists and is complete.
        let task = self
            .task_manager
            .get_task(&request.task_id)
            .await
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "unknown task ID: {}. Tasks expire 5 minutes after completion.",
                        request.task_id
                    ),
                    None,
                )
            })?;

        if task.status != rmcp::model::TaskStatus::Completed {
            return Err(McpError::invalid_params(
                format!(
                    "task {} has status {:?}, result is only available when completed",
                    request.task_id, task.status
                ),
                None,
            ));
        }

        self.task_manager
            .get_result(&request.task_id)
            .await
            .map(|r| GetTaskPayloadResult::new(serde_json::to_value(r).unwrap_or_default()))
            .ok_or_else(|| {
                McpError::internal_error("task completed but result not found".to_string(), None)
            })
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CancelTaskResult, McpError> {
        self.task_manager
            .cancel(&request.task_id)
            .await
            .map(task_to_cancel_result)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "unknown task ID: {}. Tasks expire 5 minutes after completion.",
                        request.task_id
                    ),
                    None,
                )
            })
    }

    // ── Resources ───────────────────────────────────────────────────

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                Resource::new(
                    RawResource {
                        uri: "mcp://server-card.json".to_string(),
                        name: "server card".to_string(),
                        description: Some(
                            "MCP Server Card (SEP-1649) — tool catalog, capabilities, \
                             version metadata, and ministr extension declarations"
                                .to_string(),
                        ),
                        mime_type: Some("application/json".to_string()),
                        size: None,
                        icons: None,
                        meta: None,
                        title: None,
                    },
                    None,
                ),
                Resource::new(
                    RawResource {
                        uri: "ministr://status".to_string(),
                        name: "ministr status".to_string(),
                        description: Some(
                            "Index statistics — vector count, dimension, session state, and budget"
                                .to_string(),
                        ),
                        mime_type: Some("application/json".to_string()),
                        size: None,
                        icons: None,
                        meta: None,
                        title: None,
                    },
                    None,
                ),
            ],
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
            resource_templates: vec![ResourceTemplate::new(
                RawResourceTemplate {
                    uri_template: "ministr://corpus/{path}".to_string(),
                    name: "corpus document".to_string(),
                    description: Some(
                        "Document metadata — title, source path, summary, and section count"
                            .to_string(),
                    ),
                    mime_type: Some("application/json".to_string()),
                    icons: None,
                    title: None,
                },
                None,
            )],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;
        if uri == "mcp://server-card.json" {
            Ok(self.read_server_card_resource())
        } else if uri == "ministr://status" {
            self.read_status_resource().await
        } else if let Some(path) = uri.strip_prefix("ministr://corpus/") {
            self.read_corpus_resource(path).await
        } else {
            Err(McpError::new(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                format!("unknown resource URI: {uri}"),
                None,
            ))
        }
    }

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        let uri = &request.uri;
        // Only ministr://status supports subscriptions for now.
        if uri != "ministr://status" {
            return Err(McpError::new(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                format!("resource URI does not support subscriptions: {uri}"),
                None,
            ));
        }
        let mut subs = self.subscriptions.lock().await;
        subs.insert(uri.clone());
        tracing::info!(uri, "client subscribed to resource");
        Ok(())
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        let mut subs = self.subscriptions.lock().await;
        subs.remove(&request.uri);
        tracing::info!(uri = %request.uri, "client unsubscribed from resource");
        Ok(())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        tracing::info!("client initialized, starting ingestion progress notifier");
        // Store the peer for server-initiated notifications.
        if let Ok(mut guard) = self.peer.try_lock() {
            *guard = Some(context.peer);
        }
        let progress = Arc::clone(&self.ingestion_progress);
        let peer_lock = Arc::clone(&self.peer);
        tokio::spawn(async move {
            run_ingestion_progress_notifier(progress, peer_lock).await;
        });

        // Spawn the resource subscription notification dispatcher.
        // Takes ownership of the coherence receiver (if present) and pushes
        // `notifications/resources/updated` to the client when subscribed
        // resources change due to file modifications.
        let coherence_rx = {
            let mut guard = self.coherence_rx.lock().await;
            guard.take()
        };
        if let Some(rx) = coherence_rx {
            let peer_lock = Arc::clone(&self.peer);
            let subscriptions = Arc::clone(&self.subscriptions);
            tokio::spawn(async move {
                run_subscription_notifier(rx, peer_lock, subscriptions).await;
            });
        }
    }

    // ── Completions ──────────────────────────────────────────────────

    async fn complete(
        &self,
        request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, McpError> {
        let partial = &request.argument.value;
        let values = match &request.r#ref {
            Reference::Prompt(prompt_ref) => {
                // For the dependency-chain prompt, complete the "concept" argument
                // by fuzzy-matching section IDs.
                if prompt_ref.name == "dependency-chain" && request.argument.name == "concept" {
                    self.complete_section_ids(partial).await
                } else {
                    Vec::new()
                }
            }
            Reference::Resource(resource_ref) => {
                // Complete ministr://corpus/{path} resource URIs.
                if resource_ref.uri.starts_with("ministr://corpus/") {
                    self.complete_corpus_paths(partial).await
                } else {
                    Vec::new()
                }
            }
        };

        let info = CompletionInfo::with_all_values(values)
            .map_err(|e| McpError::new(rmcp::model::ErrorCode::INTERNAL_ERROR, e, None))?;
        Ok(CompleteResult::new(info))
    }
}

#[tool_router]
impl MinistrServer {
    /// Search the corpus for sections relevant to your query.
    ///
    /// Returns ranked summaries with relevance scores across all resolution
    /// levels (document summaries, section text, atomic claims).
    /// Results that were already delivered in this session are filtered out.
    #[tool(
        name = "ministr_survey",
        description = "Search the indexed corpus by natural-language query. Start here for any vague question; follow up with ministr_read (full text) or ministr_extract (atomic claims) on top results.",
        output_schema = tool_output_schema::<ToolResponse<SurveyResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn survey(
        &self,
        Parameters(params): Parameters<SurveyParams>,
    ) -> Result<CallToolResult, McpError> {
        let top_k = params.top_k.unwrap_or(10);
        let span = info_span!("ministr_survey", query_len = params.query.len(), top_k);

        async {
            debug!(query = %params.query, top_k, "ministr_survey request");

            // Collect delivered IDs so the service can exclude them
            // before truncating to top_k (prevents the over-fetch buffer
            // from being wasted by premature truncation).
            let exclude_ids = {
                let reg = self.registry.lock().await;
                let entry = reg
                    .get_session(&self.active_session_id)
                    .expect("active session exists");
                entry.session.delivered_ids()
            };

            // Run the survey; if results are ambiguous, try eliciting a refined query.
            let survey_result = self
                .service
                .survey_excluding(&params.query, top_k, &exclude_ids)
                .await;

            // Attempt disambiguation: if top score is low and scores are clustered,
            // elicit a refined query from the agent and re-run the search.
            let survey_result = match &survey_result {
                Ok((results, _)) if results.len() >= 3 => {
                    let top_score = results.first().map_or(0.0, |r| r.score);
                    let fifth_score = results.get(4).map_or(0.0, |r| r.score);
                    let spread = top_score - fifth_score;

                    if top_score < 0.5 && spread < 0.1 {
                        debug!(
                            top_score,
                            spread, "ambiguous survey results, attempting elicitation"
                        );
                        let peer_guard = self.peer.lock().await;
                        if let Some(peer) = peer_guard.clone() {
                            drop(peer_guard);
                            let preview: String = results
                                .iter()
                                .take(5)
                                .enumerate()
                                .map(|(i, r)| {
                                    format!(
                                        "  {}. [score={:.2}] {}",
                                        i + 1,
                                        r.score,
                                        r.text.chars().take(80).collect::<String>()
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            let message = format!(
                                "Your query '{}' returned ambiguous results \
                                 (top score {top_score:.2}, spread {spread:.2}):\n\
                                 {preview}\n\n\
                                 Provide a more specific refined_query, or leave empty to \
                                 keep these results.",
                                params.query
                            );
                            if let Some(refinement) = crate::elicitation::try_elicit::<
                                crate::elicitation::SearchRefinement,
                            >(&peer, &message)
                            .await
                            {
                                if refinement.refined_query.is_empty() {
                                    survey_result
                                } else {
                                    debug!(
                                        refined = %refinement.refined_query,
                                        "re-running survey with refined query"
                                    );
                                    self.service
                                        .survey_excluding(
                                            &refinement.refined_query,
                                            top_k,
                                            &exclude_ids,
                                        )
                                        .await
                                }
                            } else {
                                survey_result
                            }
                        } else {
                            drop(peer_guard);
                            survey_result
                        }
                    } else {
                        survey_result
                    }
                }
                _ => survey_result,
            };

            match survey_result {
                Ok((results, deduplicated_count)) => {
                    debug!(
                        result_count = results.len(),
                        deduplicated_count, "ministr_survey success"
                    );

                    // Record delivered content in session and budget
                    let mut reg = self.registry.lock().await;
                    let entry = self.ensure_session_mut(&mut reg);
                    let turn = entry.session.current_turn() + 1;

                    // Track query for task-aware salience scoring
                    entry.session.record_query(&params.query);

                    for r in &results {
                        let token_count = count_tokens(&r.text);
                        let hash = content_hash(&r.text);
                        let resolution = parse_resolution(&r.resolution);
                        entry.session.record_delivery(
                            &ContentId(r.content_id.clone()),
                            resolution,
                            token_count,
                            turn,
                            hash,
                        );
                        let _ = entry.budget.record_tokens(&r.content_id, token_count);
                    }
                    let usage_status = entry.budget.usage_status();

                    // Survey-triggered prefetch: pre-warm parent sections of claim hits
                    let claim_section_ids: Vec<String> = results
                        .iter()
                        .filter(|r| r.resolution == "claim")
                        .filter_map(|r| parent_section_id(&r.content_id).map(String::from))
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .filter(|sid| !entry.session.is_delivered(&ContentId(sid.clone())))
                        .collect();
                    drop(reg);

                    if !claim_section_ids.is_empty() {
                        let mut prefetch = self.prefetch.lock().await;
                        // Filter out sections already in the prefetch cache
                        let ids_to_fetch: Vec<String> = claim_section_ids
                            .into_iter()
                            .filter(|sid| prefetch.cache().peek(sid).is_none())
                            .collect();

                        if !ids_to_fetch.is_empty() {
                            let mut sections = Vec::new();
                            let mut claims_counts = std::collections::HashMap::new();
                            for sid in &ids_to_fetch {
                                let section_id = SectionId(sid.clone());
                                if let Ok(Some(record)) =
                                    self.service.storage().get_section(&section_id).await
                                {
                                    if let Ok(claims) =
                                        self.service.storage().list_claims(&section_id).await
                                    {
                                        claims_counts.insert(sid.clone(), claims.len());
                                    }
                                    sections.push(record);
                                }
                            }
                            prefetch.prefetch_survey_expand(sections, &claims_counts);
                        }
                    }

                    // Agent intent: record survey result section IDs as predicted next reads.
                    // Top results are likely to be read next by the agent.
                    {
                        let survey_section_ids: Vec<String> = results
                            .iter()
                            .filter(|r| r.resolution == "section")
                            .take(MAX_INTENT_PREFETCH_SURVEY)
                            .map(|r| r.content_id.clone())
                            .collect();
                        if !survey_section_ids.is_empty() {
                            let mut prefetch = self.prefetch.lock().await;
                            prefetch.record_tool_call("ministr_survey", &params.query);
                            prefetch.record_survey_results(survey_section_ids.clone());

                            // Fetch and pre-warm the predicted sections
                            let mut sections = Vec::new();
                            for sid in &survey_section_ids {
                                if prefetch.cache().peek(sid).is_some() {
                                    continue;
                                }
                                let section_id = SectionId(sid.clone());
                                if let Ok(Some(record)) =
                                    self.service.storage().get_section(&section_id).await
                                {
                                    sections.push(record);
                                }
                            }
                            if !sections.is_empty() {
                                prefetch.prefetch_from_intent(sections);
                            }
                        }
                    }

                    self.persist_session().await;

                    // Suggest a follow-up on the top hit when it's confidently above noise.
                    // Symbol-resolution hits route to ministr_definition; everything else
                    // (section / claim / summary) routes to ministr_read on the section.
                    let extra_actions = top_hit_next_action(&results);

                    let response = self
                        .build_response_with(
                            SurveyResponse {
                                results,
                                deduplicated_count,
                            },
                            usage_status,
                            extra_actions,
                        )
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "ministr_survey failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Read the full text of a section by its hierarchical ID.
    ///
    /// Returns the complete section content with heading path and
    /// the number of claims available for extraction. Handles three cases:
    /// 1. **New content** — delivers full text.
    /// 2. **Already delivered, unchanged** — returns short "already delivered" message.
    ///    If the agent re-requests unchanged content, this is treated as a
    ///    fault-based eviction signal and the window estimate is corrected.
    /// 3. **Already delivered, changed** — returns a line-level delta instead
    ///    of re-delivering the full text.
    #[tool(
        name = "ministr_read",
        description = "Full content of a section by ID. On a repeat request it returns only what changed since it last showed you this section (or a short stub if nothing changed). Call ministr_extract instead if you only need atomic claims.",
        output_schema = tool_output_schema::<ToolResponse<ReadOutputData>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn read(
        &self,
        Parameters(params): Parameters<ReadParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_read", section_id = %params.section_id);

        async {
            debug!(section_id = %params.section_id, "ministr_read request");

            // Check prefetch cache for a warm hit
            let warm_detail = {
                let mut prefetch = self.prefetch.lock().await;
                prefetch.try_serve(&params.section_id).map(|entry| {
                    ministr_core::service::SectionDetail {
                        section_id: entry.content_id.clone(),
                        heading_path: entry.heading_path.clone().unwrap_or_default(),
                        text: entry.text.clone(),
                        summary: entry.summary.clone(),
                        claims_available: entry.claims_available,
                    }
                })
            };

            let read_result = if let Some(detail) = warm_detail {
                debug!(section_id = %params.section_id, "ministr_read: warm cache hit");
                Ok(detail)
            } else {
                self.service.read_section(&params.section_id).await
            };

            match read_result {
                Ok(detail) => {
                    let current_hash = content_hash(&detail.text);
                    let content_id = ContentId(params.section_id.clone());

                    // Check deduplication against session shadow
                    let mut reg = self.registry.lock().await;
                    let entry = self.ensure_session_mut(&mut reg);
                    let already_delivered = entry.session.is_delivered(&content_id);
                    let has_changed = entry.session.has_changed(&content_id, &current_hash);
                    let is_re_request = entry.session.is_re_request(&content_id, &current_hash);

                    // Case 2: Already delivered and unchanged — skip re-delivery
                    if already_delivered && !has_changed {
                        debug!(
                            section_id = %params.section_id,
                            "ministr_read: already delivered, skipping re-delivery"
                        );

                        entry.session.record_dedup_hit();

                        // If agent re-requests content it should still have,
                        // treat as a fault-based eviction signal.
                        if is_re_request {
                            entry.budget.force_evict(&params.section_id);
                        }

                        let usage_status = entry.budget.usage_status();
                        drop(reg);

                        let skip = AlreadyDeliveredResponse {
                            section_id: params.section_id.clone(),
                            status: "already_delivered",
                            claims_available: detail.claims_available,
                        };
                        let response = self.build_response(skip, usage_status).await;
                        return structured_result(&response);
                    }

                    // Track delta updates when content has changed since last delivery.
                    if already_delivered && has_changed {
                        entry.session.record_delta_update();
                    }

                    // Case 1: New content (or changed) — deliver full text
                    drop(reg);
                    let usage_status = self
                        .record_section_delivery(&params.section_id, &detail.text, current_hash)
                        .await;
                    self.record_analytics_access(&params.section_id).await;
                    self.trigger_prefetch(&params.section_id).await;

                    let response = self.build_response(detail, usage_status).await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, section_id = %params.section_id, "ministr_read failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Extract atomic claims from a specific section.
    ///
    /// Returns claim-level factual statements, optionally filtered
    /// by relevance to a query.
    #[tool(
        name = "ministr_extract",
        description = "Atomic claims from a section, optionally query-filtered. Cheaper than ministr_read when you don't need full prose.",
        output_schema = tool_output_schema::<ToolResponse<ExtractResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn extract(
        &self,
        Parameters(params): Parameters<ExtractParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!(
            "ministr_extract",
            section_id = %params.section_id,
            has_query = params.query.is_some()
        );

        async {
            debug!(
                section_id = %params.section_id,
                query = params.query.as_deref().unwrap_or("<none>"),
                "ministr_extract request"
            );

            match self
                .service
                .extract_claims(&params.section_id, params.query.as_deref())
                .await
            {
                Ok(claims) => {
                    debug!(
                        section_id = %params.section_id,
                        claim_count = claims.len(),
                        "ministr_extract success"
                    );

                    // Record each claim delivery in session and budget
                    let mut reg = self.registry.lock().await;
                    let entry = self.ensure_session_mut(&mut reg);
                    let turn = entry.session.current_turn() + 1;
                    for c in &claims {
                        let token_count = count_tokens(&c.text);
                        let hash = content_hash(&c.text);
                        entry.session.record_delivery(
                            &ContentId(c.claim_id.clone()),
                            Resolution::Claim,
                            token_count,
                            turn,
                            hash,
                        );
                        let _ = entry.budget.record_tokens(&c.claim_id, token_count);
                    }
                    let usage_status = entry.budget.usage_status();
                    drop(reg);

                    self.persist_session().await;

                    let response = self
                        .build_response(ExtractResponse { claims }, usage_status)
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, section_id = %params.section_id, "ministr_extract failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Follow dependency chains between claims.
    ///
    /// Given a claim ID, returns other claims that reference, depend on,
    /// contradict, or update it. Enables the agent to trace reasoning chains
    /// across documents.
    #[tool(
        name = "ministr_related",
        description = "Follow relationship edges (references, contradicts, depends_on, updates) from a claim. Use when one claim's truth depends on another.",
        output_schema = tool_output_schema::<ToolResponse<RelatedResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn related(
        &self,
        Parameters(params): Parameters<RelatedParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_related", claim_id = %params.claim_id);

        async {
            debug!(claim_id = %params.claim_id, "ministr_related request");

            // Parse relation type filters
            let relation_types: Option<Vec<RelationType>> =
                params.relation_types.as_ref().map(|types| {
                    types
                        .iter()
                        .filter_map(|t| RelationType::parse(t))
                        .collect()
                });

            match self
                .service
                .related_claims(&params.claim_id, relation_types.as_deref())
                .await
            {
                Ok(related) => {
                    debug!(
                        claim_id = %params.claim_id,
                        related_count = related.len(),
                        "ministr_related success"
                    );

                    // Record each related claim delivery in session and budget
                    let mut reg = self.registry.lock().await;
                    let entry = self.ensure_session_mut(&mut reg);
                    let turn = entry.session.current_turn() + 1;
                    for r in &related {
                        let token_count = count_tokens(&r.text);
                        let hash = content_hash(&r.text);
                        entry.session.record_delivery(
                            &ContentId(r.claim_id.clone()),
                            Resolution::Claim,
                            token_count,
                            turn,
                            hash,
                        );
                        let _ = entry.budget.record_tokens(&r.claim_id, token_count);
                    }
                    let usage_status = entry.budget.usage_status();
                    drop(reg);

                    self.persist_session().await;

                    let response = self
                        .build_response(RelatedResponse { related }, usage_status)
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, claim_id = %params.claim_id, "ministr_related failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Signal that content has been evicted from the agent's context window.
    ///
    /// Accepts a list of content IDs that the agent has dropped. This feedback
    /// updates the session shadow and window estimator, improving the accuracy
    /// of budget tracking and deduplication for subsequent requests.
    #[tool(
        name = "ministr_dropped",
        description = "Call immediately after dropping content you previously received. Keeps ministr's view of what you still have accurate; without this, future ministr_read calls on dropped IDs return short 'already delivered' stubs instead of the full text.",
        output_schema = tool_output_schema::<ToolResponse<DroppedResponse>>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn dropped(
        &self,
        Parameters(params): Parameters<DroppedParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_dropped", count = params.content_ids.len());

        async {
            debug!(content_ids = ?params.content_ids, "ministr_dropped request");

            let mut dropped = Vec::new();
            let mut not_found = Vec::new();

            let mut reg = self.registry.lock().await;
            let entry = self.ensure_session_mut(&mut reg);

            for id_str in &params.content_ids {
                let content_id = ContentId(id_str.clone());
                if entry.session.remove_delivered(&content_id).is_some() {
                    entry.budget.force_evict(id_str);
                    dropped.push(id_str.clone());
                } else {
                    not_found.push(id_str.clone());
                }
            }

            let usage_status = entry.budget.usage_status();
            drop(reg);

            self.persist_session().await;

            debug!(
                dropped_count = dropped.len(),
                not_found_count = not_found.len(),
                "ministr_dropped complete"
            );

            let response = self
                .build_response(DroppedResponse { dropped, not_found }, usage_status)
                .await;
            structured_result(&response)
        }
        .instrument(span)
        .await
    }

    /// Get the current context budget status and eviction recommendations.
    ///
    /// Returns the total budget, estimated usage, pressure level, and a
    /// ranked list of eviction candidates when under pressure. Use this
    /// to understand budget health and decide what to evict.
    #[tool(
        name = "ministr_usage",
        description = "Internal ministr accounting (a rough token estimate of what it has delivered so far). Advisory only and anchored to a configured window, not your real model context window — do NOT use it to decide you are low on context or to stop work. Safe to ignore.",
        output_schema = tool_output_schema::<UsageResponse>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn usage(&self) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_usage");

        async {
            debug!("ministr_usage request");

            let mut reg = self.registry.lock().await;
            let entry = self.ensure_session_mut(&mut reg);
            let prefetch = self.prefetch.lock().await;

            let status = entry.budget.usage_status();
            let candidates = entry
                .budget
                .drop_candidates(&entry.session, 5, Some(&entry.memory));
            let prefetch_metrics = prefetch.metrics();
            let alerts = entry.session.drain_alerts();
            let metrics = entry.session.metrics().clone();

            drop(prefetch);
            drop(reg);

            let level_str = match status.level {
                UsageLevel::Normal => "normal",
                UsageLevel::Elevated => "elevated",
                UsageLevel::Critical => "critical",
            };

            debug!(
                level = level_str,
                used = status.tokens_used,
                remaining = status.tokens_remaining,
                candidate_count = candidates.len(),
                "ministr_usage complete"
            );

            // When usage is elevated, try eliciting which sections to drop.
            let mut elicitation_evicted = Vec::new();
            if status.level != UsageLevel::Normal && !candidates.is_empty() {
                let candidate_list: String = candidates
                    .iter()
                    .map(|c| format!("  - {} ({} tokens)", c.content_id, c.tokens_recoverable))
                    .collect::<Vec<_>>()
                    .join("\n");
                let message = format!(
                    "Usage level is {level_str}. These sections are drop candidates:\n\
                     {candidate_list}\n\n\
                     Which content_ids would you like to evict? \
                     (provide comma-separated content_ids, or decline to skip)"
                );

                let peer_guard = self.peer.lock().await;
                if let Some(peer) = peer_guard.clone() {
                    drop(peer_guard);
                    if let Some(choice) = crate::elicitation::try_elicit::<
                        crate::elicitation::DropChoice,
                    >(&peer, &message)
                    .await
                    {
                        let ids = choice.ids();
                        if !ids.is_empty() {
                            let mut reg = self.registry.lock().await;
                            let entry = reg
                                .get_session_mut(&self.active_session_id)
                                .expect("active session exists");
                            for id_str in &ids {
                                let content_id = ContentId(id_str.clone());
                                if entry.session.remove_delivered(&content_id).is_some() {
                                    entry.budget.force_evict(id_str);
                                    elicitation_evicted.push(id_str.clone());
                                }
                            }
                            drop(reg);
                            self.persist_session().await;
                            debug!(
                                evicted = ?elicitation_evicted,
                                "evicted via budget elicitation"
                            );
                        }
                    }
                } else {
                    drop(peer_guard);
                }
            }

            let (schema_tokens, tool_count) = self.schema_token_overhead();

            let response = UsageResponse {
                total_budget: status.tokens_used + status.tokens_remaining,
                estimated_used: status.tokens_used,
                estimated_remaining: status.tokens_remaining,
                level: level_str.to_string(),
                drop_candidates: candidates,
                prefetch_metrics,
                session_metrics: SessionMetricsResponse {
                    total_deliveries: metrics.total_deliveries,
                    cumulative_tokens_delivered: metrics.cumulative_tokens_delivered,
                    total_evictions: metrics.total_evictions,
                    cumulative_tokens_evicted: metrics.cumulative_tokens_evicted,
                    total_compressions: metrics.total_compressions,
                    cumulative_tokens_compressed: metrics.cumulative_tokens_compressed,
                    total_tokens_saved: metrics.total_tokens_saved(),
                    compression_ratio: metrics.compression_ratio(),
                    delta_updates: metrics.delta_updates,
                    dedup_hits: metrics.dedup_hits,
                },
                schema_tokens,
                tool_count,
                coherence_alerts: alerts,
                elicitation_evicted,
            };
            structured_result(&response)
        }
        .instrument(span)
        .await
    }

    /// Generate compressed summaries for content the agent wants to evict.
    ///
    /// Uses extractive TF-IDF summarization for 60–80% token reduction
    /// with no extra cost.
    #[tool(
        name = "ministr_compress",
        description = "Extractive TF-IDF summaries (roughly 60-80% shorter) for sections you want to keep referenceable without their full text. Pair with ministr_dropped after dropping the originals.",
        output_schema = tool_output_schema::<ToolResponse<CompressResponse>>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn compress(
        &self,
        Parameters(params): Parameters<CompressParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_compress", count = params.content_ids.len());

        async {
            debug!(content_ids = ?params.content_ids, "ministr_compress request");

            // Always use extractive (TF-IDF) compression — fast, no extra cost,
            // and doesn't require MCP sampling support from the client.
            let result = self.service.compress_content(&params.content_ids).await;

            match result {
                Ok(summaries) => {
                    debug!(summary_count = summaries.len(), "ministr_compress success");

                    let total_original: usize = summaries.iter().map(|s| s.original_tokens).sum();
                    let total_compressed: usize =
                        summaries.iter().map(|s| s.compressed_tokens).sum();
                    let ratio = if total_original > 0 {
                        total_compressed as f64 / total_original as f64
                    } else {
                        0.0
                    };

                    let mut reg = self.registry.lock().await;
                    let usage_status = self.ensure_session_mut(&mut reg).budget.usage_status();
                    drop(reg);

                    let compress_resp = CompressResponse {
                        summaries,
                        total_original_tokens: total_original,
                        total_compressed_tokens: total_compressed,
                        compression_ratio: ratio,
                    };
                    let response = self.build_response(compress_resp, usage_status).await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "ministr_compress failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Return a structural table of contents for the indexed corpus.
    ///
    /// Lists all documents and their sections as metadata-only entries
    /// (no text content). Includes corpus-level statistics for quick
    /// orientation. Optionally filtered to a single document.
    #[tool(
        name = "ministr_toc",
        description = "Structural overview (table of contents) of the indexed corpus. Use to orient on an unfamiliar codebase before drilling in.",
        output_schema = tool_output_schema::<ToolResponse<TocResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn toc(
        &self,
        Parameters(params): Parameters<TocParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_toc", document_id = ?params.document_id);

        async {
            debug!(document_id = ?params.document_id, "ministr_toc request");

            match self.service.toc(params.document_id.as_deref()).await {
                Ok(entries) => {
                    let total_sections = entries.len();
                    let total_claims: usize = entries.iter().map(|e| e.claims_available).sum();

                    // Count unique document IDs
                    let mut doc_ids: Vec<&str> =
                        entries.iter().map(|e| e.document_id.as_ref()).collect();
                    doc_ids.sort_unstable();
                    doc_ids.dedup();
                    let total_documents = doc_ids.len();

                    // Apply pagination
                    let offset = params.offset.unwrap_or(0);
                    let limit = params.limit.unwrap_or(100);
                    let paginated: Vec<_> = entries.into_iter().skip(offset).take(limit).collect();
                    let returned = paginated.len();

                    debug!(
                        total_documents,
                        total_sections, total_claims, offset, returned, "ministr_toc success"
                    );

                    let mut reg = self.registry.lock().await;
                    let usage_status = self.ensure_session_mut(&mut reg).budget.usage_status();
                    drop(reg);

                    // Report ingestion status when corpus is empty to help
                    // diagnose "0 documents" scenarios.
                    let ingestion_status = match self.ingestion_progress.status() {
                        0 if total_documents == 0 => Some("pending".to_string()),
                        1 => Some("running".to_string()),
                        _ => None, // Don't clutter the response when complete
                    };

                    // Include corpus roots when not filtered to a single document.
                    let roots = if params.document_id.is_none() {
                        self.service.list_corpus_roots().await.unwrap_or_default()
                    } else {
                        Vec::new()
                    };

                    let response = self
                        .build_response(
                            TocResponse {
                                corpus_stats: CorpusStatsHeader {
                                    documents: total_documents,
                                    sections: total_sections,
                                    claims: total_claims,
                                    offset,
                                    returned,
                                    ingestion_status,
                                },
                                roots,
                                entries: paginated,
                            },
                            usage_status,
                        )
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "ministr_toc failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Fetch web content by URL and add it to the indexed corpus.
    ///
    /// Automatically selects the best strategy: tries `llms-full.txt` and
    /// `llms.txt` first, then falls back to direct page fetch. Fetched
    /// content is parsed, indexed with embeddings, and immediately
    /// searchable via `ministr_survey`.
    #[tool(
        name = "ministr_fetch",
        description = "Fetch a URL from the web and index its content. Tries llms.txt first, falls back to direct page fetch.",
        output_schema = tool_output_schema::<FetchOutputData>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true),
        execution(task_support = "optional")
    )]
    async fn fetch(
        &self,
        Parameters(params): Parameters<FetchParams>,
        ct: CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_fetch", url = %params.url);

        async {
            debug!(
                url = %params.url,
                depth = params.depth,
                max_pages = params.max_pages,
                path_filter = ?params.path_filter,
                "ministr_fetch request"
            );

            let Some(ref web_fetcher) = self.web_fetcher else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_fetch is not available: web fetcher not configured. \
                     Start ministr with a data directory to enable web fetching."
                        .to_string(),
                )]));
            };

            let Some(ref embedder) = self.embedder else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_fetch is not available: embedder not configured.".to_string(),
                )]));
            };

            let Some(ref index) = self.index else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_fetch is not available: vector index not configured.".to_string(),
                )]));
            };

            let Some(ref storage) = self.storage else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_fetch is not available: storage not configured.".to_string(),
                )]));
            };

            // When invoked as an MCP Task, rmcp's enqueue_task handles the
            // async lifecycle. This code path always runs synchronously.
            match web_fetcher
                .fetch_and_ingest_with_embeddings(
                    &params.url,
                    &self.ingestion_pipeline,
                    storage.as_ref(),
                    embedder.as_ref(),
                    index.as_ref(),
                    Some(&ct),
                )
                .await
            {
                Ok(result) => {
                    debug!(
                        url = %params.url,
                        pages = result.pages_fetched(),
                        sections = result.sections_indexed,
                        claims = result.claims_extracted,
                        tokens = result.tokens_added,
                        strategy = %result.strategy,
                        "ministr_fetch success"
                    );

                    let mut reg = self.registry.lock().await;
                    let usage_status = self.ensure_session_mut(&mut reg).budget.usage_status();
                    drop(reg);

                    let response = self
                        .build_response(
                            FetchResponse {
                                pages_fetched: result.pages_fetched(),
                                sections_indexed: result.sections_indexed,
                                claims_extracted: result.claims_extracted,
                                tokens_added: result.tokens_added,
                                strategy_used: result.strategy.to_string(),
                            },
                            usage_status,
                        )
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, url = %params.url, "ministr_fetch failed");
                    Ok(CallToolResult::error(vec![Content::text(format!(
                        "fetch failed: {e}"
                    ))]))
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Check cached web sources for staleness and re-fetch changed content.
    ///
    /// If a URL is provided, checks only that URL. If omitted, checks all
    /// cached web sources. For each stale URL, re-fetches the content,
    /// re-indexes it with embeddings, and reports what was updated.
    #[tool(
        name = "ministr_refresh",
        description = "Check cached web and git sources for staleness and re-fetch changed content.",
        output_schema = tool_output_schema::<ToolResponse<RefreshResponse>>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn refresh(
        &self,
        Parameters(params): Parameters<RefreshParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_refresh", url = ?params.url);

        async {
            debug!(url = ?params.url, "ministr_refresh request");

            let Some(ref embedder) = self.embedder else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_refresh is not available: embedder not configured.".to_string(),
                )]));
            };

            let Some(ref index) = self.index else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_refresh is not available: vector index not configured.".to_string(),
                )]));
            };

            let Some(ref storage) = self.storage else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_refresh is not available: storage not configured.".to_string(),
                )]));
            };

            self.refresh_all_sources(&params, storage, embedder.as_ref(), index.as_ref())
                .await
        }
        .instrument(span)
        .await
    }

    /// Clone a git repository and index its content into the corpus.
    ///
    /// Uses `GitFetcher` for efficient shallow cloning with optional sparse
    /// checkout. If the repository is already cached and the remote HEAD
    /// matches, the cached clone is reused. Cloned content is parsed,
    /// embedded, and immediately searchable via `ministr_survey`.
    #[tool(
        name = "ministr_clone",
        description = "Clone a git repository and index its content. Supports sparse checkout. Cached clones are reused.",
        output_schema = tool_output_schema::<CloneOutputData>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true),
        execution(task_support = "optional")
    )]
    async fn clone_repo(
        &self,
        Parameters(params): Parameters<CloneParams>,
        ct: CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_clone", repo = %params.repo);

        async {
            debug!(
                repo = %params.repo,
                paths = ?params.paths,
                branch = ?params.branch,
                "ministr_clone request"
            );

            let Some(ref git_fetcher) = self.git_fetcher else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_clone is not available: git fetcher not configured. \
                     Start ministr with a data directory to enable git cloning."
                        .to_string(),
                )]));
            };

            let Some(ref embedder) = self.embedder else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_clone is not available: embedder not configured.".to_string(),
                )]));
            };

            let Some(ref index) = self.index else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_clone is not available: vector index not configured.".to_string(),
                )]));
            };

            let Some(ref storage) = self.storage else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "ministr_clone is not available: storage not configured.".to_string(),
                )]));
            };

            // When invoked as an MCP Task, rmcp's enqueue_task handles the
            // async lifecycle. This code path always runs synchronously.
            self.clone_and_ingest(
                &params,
                git_fetcher,
                embedder.as_ref(),
                index.as_ref(),
                storage.as_ref(),
                Some(&ct),
            )
            .await
        }
        .instrument(span)
        .await
    }

    /// Poll the status of a background task.
    ///
    /// Delegates to the MCP task manager. Prefer using the protocol-native
    /// `tasks/get` and `tasks/result` methods instead. This tool is retained
    /// for backward compatibility.
    #[tool(
        name = "ministr_task",
        description = "Poll a background task status. Deprecated: prefer MCP tasks/get.",
        output_schema = tool_output_schema::<TaskStatusResponse>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn task_status(
        &self,
        Parameters(params): Parameters<TaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_task", task_id = %params.task_id);

        async {
            debug!(task_id = %params.task_id, "ministr_task request");

            match self.task_manager.get_task(&params.task_id).await {
                Some(task) => structured_result(&task),
                None => Ok(CallToolResult::error(vec![Content::text(format!(
                    "unknown task ID: {}. Tasks expire 5 minutes after completion.",
                    params.task_id
                ))])),
            }
        }
        .instrument(span)
        .await
    }

    /// Search the symbol index for code symbols.
    ///
    /// Returns matching symbols with their file location, signature, and
    /// doc comment preview. Use the returned symbol IDs with `ministr_definition`
    /// and `ministr_references`.
    #[tool(
        name = "ministr_symbols",
        description = "Find code symbols (functions, structs, traits, etc.) by name, kind, module, or visibility. Pair with ministr_definition for source and ministr_references before modifying.",
        output_schema = tool_output_schema::<ToolResponse<SymbolsResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn symbols(
        &self,
        Parameters(params): Parameters<SymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_symbols", query = ?params.query, kind = ?params.kind);

        async {
            debug!(?params.query, ?params.kind, ?params.module, ?params.visibility, "ministr_symbols request");

            // Track query for task-aware salience scoring
            if let Some(ref q) = params.query {
                let mut reg = self.registry.lock().await;
                if let Some(entry) = reg.get_session_mut(&self.active_session_id) {
                    entry.session.record_query(q);
                }
            }

            let filter = SymbolFilter {
                name: params.query,
                name_exact: None,
                kind: params.kind,
                visibility: params.visibility,
                module: params.module,
                file_path: None,
            };

            match self.service.search_symbols(&filter).await {
                Ok(symbols) => {
                    let total = symbols.len();

                    // Compute transitive caller counts for all result symbols
                    let symbol_ids: Vec<_> = symbols.iter().map(|s| s.id.clone()).collect();
                    let caller_counts = self
                        .service
                        .transitive_caller_counts(&symbol_ids)
                        .await
                        .unwrap_or_default();

                    let summaries: Vec<SymbolSummary> = symbols
                        .into_iter()
                        .map(|s| {
                            let cc = caller_counts.get(&s.id).copied();
                            SymbolSummary {
                                id: s.id.0,
                                name: s.name,
                                kind: s.kind,
                                file: s.file_path,
                                line: s.line_start,
                                signature: s.signature,
                                doc_preview: s.doc_comment.map(|d| {
                                    d.lines().next().unwrap_or("").to_string()
                                }),
                                complexity: s.cyclomatic_complexity,
                                caller_count: cc,
                            }
                        })
                        .collect();

                    // Apply pagination
                    let offset = params.offset.unwrap_or(0);
                    let limit = params.limit.unwrap_or(100);
                    let paginated: Vec<_> =
                        summaries.into_iter().skip(offset).take(limit).collect();

                    debug!(total, offset, returned = paginated.len(), "ministr_symbols success");

                    let mut reg = self.registry.lock().await;
                    let usage_status = self.ensure_session_mut(&mut reg).budget.usage_status();
                    drop(reg);

                    // When there's exactly one match, suggest fetching its definition —
                    // the agent almost always wants the source next.
                    let extra_actions = single_symbol_next_action(&paginated);

                    let response = self
                        .build_response_with(
                            SymbolsResponse { symbols: paginated, total, offset },
                            usage_status,
                            extra_actions,
                        )
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "ministr_symbols failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Get the full definition of a code symbol.
    ///
    /// Returns the symbol's source code with 3 lines of surrounding context,
    /// module hierarchy, and all metadata.
    #[tool(
        name = "ministr_definition",
        description = "Full source of a code symbol by ID. Call ministr_references first if you intend to modify or delete the symbol.",
        output_schema = tool_output_schema::<ToolResponse<ministr_core::service::SymbolDefinition>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn definition(
        &self,
        Parameters(params): Parameters<DefinitionParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_definition", symbol_id = %params.symbol_id);

        async {
            debug!(symbol_id = %params.symbol_id, "ministr_definition request");

            match self.service.get_symbol_definition(&params.symbol_id).await {
                Ok(def) => {
                    let token_count = count_tokens(&def.source_context);
                    let mut reg = self.registry.lock().await;
                    let entry = self.ensure_session_mut(&mut reg);
                    let _ = entry.budget.record_tokens(&params.symbol_id, token_count);
                    let usage_status = entry.budget.usage_status();
                    drop(reg);

                    debug!(symbol_id = %params.symbol_id, token_count, "ministr_definition success");

                    let response = self.build_response(def, usage_status).await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, symbol_id = %params.symbol_id, "ministr_definition failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }

    /// Find all references to a code symbol.
    ///
    /// Returns callers, implementors, importers, and users of the symbol,
    /// with source locations.
    #[tool(
        name = "ministr_references",
        description = "All callers, implementors, and importers of a code symbol. Call before deleting or significantly modifying any non-trivial public symbol — zero references means safe to delete.",
        output_schema = tool_output_schema::<ToolResponse<ReferencesResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn references(
        &self,
        Parameters(params): Parameters<ReferencesParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_references", symbol_id = %params.symbol_id);

        async {
            debug!(symbol_id = %params.symbol_id, ref_kind = ?params.ref_kind, "ministr_references request");

            let ref_kind = params
                .ref_kind
                .as_deref()
                .and_then(RefKind::parse);

            match self
                .service
                .get_symbol_references(&params.symbol_id, ref_kind)
                .await
            {
                Ok(refs) => {
                    let total = refs.len();

                    // Apply pagination
                    let offset = params.offset.unwrap_or(0);
                    let limit = params.limit.unwrap_or(100);
                    let paginated: Vec<_> =
                        refs.into_iter().skip(offset).take(limit).collect();

                    debug!(symbol_id = %params.symbol_id, total, offset, returned = paginated.len(), "ministr_references success");

                    let mut reg = self.registry.lock().await;
                    let usage_status = self.ensure_session_mut(&mut reg).budget.usage_status();
                    drop(reg);

                    let response = self
                        .build_response(
                            ReferencesResponse {
                                references: paginated,
                                total,
                                offset,
                            },
                            usage_status,
                        )
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, symbol_id = %params.symbol_id, "ministr_references failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }
    /// Query cross-language bridge links.
    ///
    /// Returns bridge links (export↔import pairs) with optional filtering by
    /// search query, bridge kind, language, or file path.
    #[tool(
        name = "ministr_bridge",
        description = "Cross-language bridge links (Tauri commands, NAPI exports, PyO3 functions, FFI, HTTP routes, etc.). Call before modifying any IPC or FFI boundary so you see every cross-language call site.",
        output_schema = tool_output_schema::<ToolResponse<BridgeResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn bridge(
        &self,
        Parameters(params): Parameters<BridgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("ministr_bridge", query = ?params.query, kind = ?params.bridge_kind);

        async {
            debug!(?params.query, ?params.bridge_kind, ?params.language, ?params.file_path, "ministr_bridge request");

            match self
                .service
                .query_bridges(
                    params.query.as_deref(),
                    params.bridge_kind.as_deref(),
                    params.language.as_deref(),
                    params.file_path.as_deref(),
                )
                .await
            {
                Ok(links) => {
                    let total = links.len();

                    let summaries: Vec<BridgeLinkSummary> = links
                        .into_iter()
                        .map(|l| BridgeLinkSummary {
                            kind: l.kind,
                            confidence: l.confidence,
                            export: BridgeEndpointSummary {
                                binding_key: l.export_binding_key,
                                symbol_name: l.export_symbol,
                                file: l.export_file,
                                line: l.export_line,
                                language: l.export_language,
                            },
                            import: BridgeEndpointSummary {
                                binding_key: l.import_binding_key,
                                symbol_name: l.import_symbol,
                                file: l.import_file,
                                line: l.import_line,
                                language: l.import_language,
                            },
                        })
                        .collect();

                    debug!(total, "ministr_bridge success");

                    let mut reg = self.registry.lock().await;
                    let usage_status = self.ensure_session_mut(&mut reg).budget.usage_status();
                    drop(reg);

                    let response = self
                        .build_response(BridgeResponse { links: summaries, total }, usage_status)
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "ministr_bridge failed");
                    Ok(CallToolResult::error(vec![Content::text(
                        format_query_error(&e),
                    )]))
                }
            }
        }
        .instrument(span)
        .await
    }
}

// ── Prompt Router ────────────────────────────────────────────────────

#[prompt_router]
impl MinistrServer {
    /// Summarize the current session: sections read, budget state, and activity.
    ///
    /// Returns a structured overview of what has been delivered in this session,
    /// the current budget utilization and pressure level, and any pending
    /// coherence alerts or stale content.
    #[prompt(
        name = "session-summary",
        description = "Summarize sections read and session activity"
    )]
    async fn session_summary(&self) -> Result<GetPromptResult, McpError> {
        let reg = self.registry.lock().await;
        let entry = reg
            .get_session(&self.active_session_id)
            .expect("active session exists");
        let status = entry.budget.usage_status();
        let prefetch = self.prefetch.lock().await;
        let metrics = prefetch.metrics();

        let delivered_count = entry.session.delivered_count();
        let total_tokens = entry.session.total_delivered_tokens();
        let trajectory_len = entry.session.trajectory().len();
        let stale_ids = entry.session.stale_content_ids();
        let has_alerts = entry.session.has_pending_alerts();
        let elapsed = entry.session.elapsed();

        let mut summary = format!(
            "## Session Summary\n\n\
             **Delivered:** {delivered_count} sections ({total_tokens} tokens)\n\
             **Trajectory:** {trajectory_len} access steps\n\
             **Duration:** {elapsed:.0?}\n\n\
             ### Budget\n\
             - Utilization: {:.1}%\n\
             - Pressure: {:?}\n\
             - Remaining: {} tokens\n",
            status.utilization * 100.0,
            status.level,
            status.tokens_remaining,
        );

        if !stale_ids.is_empty() {
            let _ = write!(
                summary,
                "\n### Stale Content\n{} sections have changed on disk since delivery.\n",
                stale_ids.len()
            );
        }

        if has_alerts {
            summary.push_str(
                "\n### Coherence\nPending coherence alerts — \
                             call `ministr_usage` to review.\n",
            );
        }

        let _ = write!(
            summary,
            "\n### Prefetch\n\
             - Hit rate: {:.1}%\n\
             - Cache entries: {}\n",
            metrics.hit_rate() * 100.0,
            prefetch.cache().len(),
        );

        Ok(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            summary,
        )])
        .with_description("Session summary — sections read, budget state, and activity"))
    }

    /// Recommend unread sections based on access patterns and prefetch state.
    ///
    /// Looks at what the prefetch engine has pre-warmed, co-access patterns
    /// from analytics, and budget headroom to suggest what to read next.
    #[prompt(
        name = "what-next",
        description = "Recommend unread sections based on access patterns and prefetch"
    )]
    async fn what_next(&self) -> Result<GetPromptResult, McpError> {
        let reg = self.registry.lock().await;
        let entry = reg
            .get_session(&self.active_session_id)
            .expect("active session exists");
        let prefetch = self.prefetch.lock().await;
        let status = entry.budget.usage_status();

        let mut recommendations = String::from("## What to Read Next\n\n");

        // 1. Pre-warmed sections not yet delivered
        let cache = prefetch.cache();
        let delivered_ids = entry.session.delivered_ids();
        let prefetched: Vec<&str> = cache
            .keys()
            .filter(|key| !delivered_ids.contains(*key))
            .take(10)
            .collect();

        if !prefetched.is_empty() {
            recommendations.push_str("### Pre-warmed (ready for instant delivery)\n");
            for id in &prefetched {
                let _ = writeln!(recommendations, "- `{id}`");
            }
            recommendations.push('\n');
        }

        // 2. Budget headroom
        let _ = write!(
            recommendations,
            "### Usage Headroom\n\
             {remaining} tokens remaining ({util:.1}% used, level: {level:?})\n\n",
            remaining = status.tokens_remaining,
            util = status.utilization * 100.0,
            level = status.level,
        );

        if matches!(status.level, UsageLevel::Critical) {
            recommendations.push_str(
                "**Warning:** Budget is critical. Consider evicting content \
                 with `ministr_dropped` or compressing with `ministr_compress` before reading more.\n\n",
            );
        }

        if prefetched.is_empty() {
            recommendations.push_str(
                "No pre-warmed sections available. Use `ministr_survey` to search \
                 for relevant content or `ministr_toc` for a structural overview.\n",
            );
        }

        Ok(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            recommendations,
        )])
        .with_description("Recommendations for what to read next"))
    }

    /// Trace claim relationships from a given concept.
    ///
    /// Searches for claims matching the concept, then follows dependency
    /// chains to show how claims relate to each other.
    #[prompt(
        name = "dependency-chain",
        description = "Trace claim relationships from a given concept"
    )]
    async fn dependency_chain(
        &self,
        Parameters(params): Parameters<DependencyChainArgs>,
    ) -> Result<GetPromptResult, McpError> {
        let concept = &params.concept;

        // Search for relevant sections to find claims.
        let results = self.service.survey(concept, 5).await.map_err(|e| {
            McpError::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                format!("survey failed: {e}"),
                None,
            )
        })?;

        if results.is_empty() {
            return Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                PromptMessageRole::User,
                format!("No content found matching concept: '{concept}'"),
            )])
            .with_description(format!("Dependency chain for '{concept}'")));
        }

        let mut output = format!("## Dependency Chain: {concept}\n\n");

        // Extract claims from top results and follow relationships.
        for result in results.iter().take(3) {
            let claims = self
                .service
                .extract_claims(&result.content_id, Some(concept))
                .await
                .unwrap_or_default();

            if claims.is_empty() {
                continue;
            }

            let _ = writeln!(output, "### {}", result.content_id);

            for claim in claims.iter().take(5) {
                let _ = writeln!(output, "- **{}**: {}", claim.claim_id, claim.text);

                // Follow relationships one level deep.
                let relations = self
                    .service
                    .related_claims(&claim.claim_id, None)
                    .await
                    .unwrap_or_default();

                for rel in relations.iter().take(3) {
                    let _ = writeln!(
                        output,
                        "  - {} → `{}`: {}",
                        rel.relation_type, rel.claim_id, rel.text,
                    );
                }
            }
            output.push('\n');
        }

        Ok(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            output,
        )])
        .with_description(format!("Dependency chain for '{concept}'")))
    }
}

/// Arguments for the `dependency-chain` prompt.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DependencyChainArgs {
    /// The concept or topic to trace dependency chains for.
    #[schemars(description = "The concept to trace claim dependency chains for")]
    pub concept: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ministr_core::embedding::Embedder;
    use ministr_core::error::IndexError;
    use ministr_core::index::{HnswIndex, VectorIndex};
    use ministr_core::service::QueryError;
    use ministr_core::session::UsageConfig;
    use ministr_core::storage::{SqliteStorage, Storage};
    use ministr_core::types::{Claim, ClaimId, ContentId, DocumentTree, Section, SectionId};
    use rmcp::model::{ProtocolVersion, ResourceContents};
    use serde::Serialize;

    use crate::server::helpers::{
        MAX_RESPONSE_BYTES, apply_response_size_guard, has_code_files_in_dir,
    };

    /// Extract the text string from the first Content item.
    fn extract_text(content: &[Content]) -> &str {
        content[0]
            .raw
            .as_text()
            .expect("expected text content")
            .text
            .as_str()
    }

    // ── Steering surface: instructions + next_action helpers ──────────

    #[test]
    fn default_instructions_do_not_push_usage_protocol() {
        // Regression: the playbook must NOT advertise a budget protocol or
        // tell agents to react to pressure — that made agents wrongly
        // conclude they were low on context. It should instead explicitly
        // state that ministr does not surface budget pressure.
        assert!(!DEFAULT_INSTRUCTIONS.contains("ministr_usage"));
        assert!(!DEFAULT_INSTRUCTIONS.contains("Budget protocol"));
        assert!(!DEFAULT_INSTRUCTIONS.contains("drop_suggestions"));
        assert!(
            DEFAULT_INSTRUCTIONS
                .contains("do not treat ministr as a signal that you are low on room")
        );
    }

    #[test]
    fn default_instructions_covers_pre_mutation_checks() {
        assert!(DEFAULT_INSTRUCTIONS.contains("ministr_references"));
        assert!(DEFAULT_INSTRUCTIONS.contains("ministr_bridge"));
    }

    #[test]
    fn default_instructions_lists_decision_tree_entry_points() {
        // Every starting point in the workflow tree must be reachable.
        for tool in [
            "ministr_survey",
            "ministr_symbols",
            "ministr_definition",
            "ministr_toc",
            "ministr_read",
        ] {
            assert!(
                DEFAULT_INSTRUCTIONS.contains(tool),
                "instructions missing entry-point reference to {tool}"
            );
        }
    }

    #[test]
    fn top_hit_next_action_empty_when_no_results() {
        let actions = top_hit_next_action(&[]);
        assert!(actions.is_empty());
    }

    #[test]
    fn top_hit_next_action_empty_when_top_score_below_threshold() {
        let results = vec![ministr_core::service::SurveyResult {
            content_id: "docs/a.md#x".into(),
            resolution: "section".into(),
            score: 0.3,
            text: "noisy match".into(),
            heading_path: None,
        }];
        let actions = top_hit_next_action(&results);
        assert!(actions.is_empty());
    }

    #[test]
    fn top_hit_next_action_section_resolution_emits_read() {
        let results = vec![ministr_core::service::SurveyResult {
            content_id: "docs/a.md#x".into(),
            resolution: "section".into(),
            score: 0.85,
            text: "confident match".into(),
            heading_path: None,
        }];
        let actions = top_hit_next_action(&results);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action, "ministr_read");
        assert_eq!(actions[0].args["section_id"], "docs/a.md#x");
    }

    #[test]
    fn top_hit_next_action_symbol_resolution_emits_definition() {
        let results = vec![ministr_core::service::SurveyResult {
            content_id: "sym-foo::bar".into(),
            resolution: "symbol_full".into(),
            score: 0.9,
            text: "sym".into(),
            heading_path: None,
        }];
        let actions = top_hit_next_action(&results);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action, "ministr_definition");
        assert_eq!(actions[0].args["symbol_id"], "sym-foo::bar");
    }

    #[test]
    fn single_symbol_next_action_emits_definition_for_lone_match() {
        let symbols = vec![SymbolSummary {
            id: "sym-foo".into(),
            name: "foo".into(),
            kind: "function".into(),
            file: "src/lib.rs".into(),
            line: 10,
            signature: "fn foo()".into(),
            doc_preview: None,
            complexity: None,
            caller_count: None,
        }];
        let actions = single_symbol_next_action(&symbols);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action, "ministr_definition");
        assert_eq!(actions[0].args["symbol_id"], "sym-foo");
        assert!(actions[0].reason.contains("foo"));
    }

    #[test]
    fn single_symbol_next_action_empty_when_zero_or_many() {
        assert!(single_symbol_next_action(&[]).is_empty());

        let two = vec![
            SymbolSummary {
                id: "sym-a".into(),
                name: "a".into(),
                kind: "function".into(),
                file: "src/a.rs".into(),
                line: 1,
                signature: String::new(),
                doc_preview: None,
                complexity: None,
                caller_count: None,
            },
            SymbolSummary {
                id: "sym-b".into(),
                name: "b".into(),
                kind: "function".into(),
                file: "src/b.rs".into(),
                line: 1,
                signature: String::new(),
                doc_preview: None,
                complexity: None,
                caller_count: None,
            },
        ];
        assert!(single_symbol_next_action(&two).is_empty());
    }

    /// Deterministic mock embedder for testing.
    struct MockEmbedder {
        dim: usize,
    }

    impl Embedder for MockEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += f32::from(b) / 255.0;
                    }
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if norm > 0.0 {
                        for x in &mut v {
                            *x /= norm;
                        }
                    }
                    v
                })
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    fn make_test_doc() -> DocumentTree {
        DocumentTree {
            id: ContentId("docs/auth.md".into()),
            title: "Authentication Guide".into(),
            source_path: "docs/auth.md".into(),
            sections: vec![Section {
                id: SectionId("docs/auth.md#tokens".into()),
                heading_path: vec!["Authentication".into(), "Tokens".into()],
                depth: 2,
                text: "JWT tokens use RS256 signing. Tokens expire after 24 hours.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![
                    Claim {
                        id: ClaimId("c1".into()),
                        text: "JWT tokens use RS256 signing algorithm.".into(),
                        section_id: SectionId("docs/auth.md#tokens".into()),
                    },
                    Claim {
                        id: ClaimId("c2".into()),
                        text: "Tokens expire after 24 hours by default.".into(),
                        section_id: SectionId("docs/auth.md#tokens".into()),
                    },
                ],
                summary: Some("Token authentication details.".into()),
            }],
            summary: Some("Complete authentication reference.".into()),
        }
    }

    async fn setup_server() -> MinistrServer {
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let doc = make_test_doc();
        storage.insert_document(&doc).await.unwrap();

        let texts_and_ids = [
            (
                "doc-summary::docs/auth.md",
                "Complete authentication reference.",
            ),
            (
                "sec-summary::docs/auth.md#tokens",
                "Token authentication details.",
            ),
            (
                "section::docs/auth.md#tokens",
                "JWT tokens use RS256 signing. Tokens expire after 24 hours.",
            ),
            ("claim::c1", "JWT tokens use RS256 signing algorithm."),
            ("claim::c2", "Tokens expire after 24 hours by default."),
        ];
        for (id, text) in &texts_and_ids {
            let vecs = embedder.embed(&[*text]).unwrap();
            index.insert(id, &vecs[0]).unwrap();
        }

        let service = Arc::new(QueryService::new(storage, embedder, index));
        MinistrServer::new(service)
    }

    /// Wrap an `MinistrServer` into an in-process MCP client for testing.
    ///
    /// Returns both client and server handle — the server handle must stay
    /// alive or the server shuts down and the client hangs.
    type TestClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
    type TestServerHandle = rmcp::service::RunningService<RoleServer, MinistrServer>;
    async fn wrap_test_client(server: MinistrServer) -> (TestClient, TestServerHandle) {
        use rmcp::ServiceExt;
        let (c2s_w, c2s_r) = tokio::io::duplex(65_536);
        let (s2c_w, s2c_r) = tokio::io::duplex(65_536);
        // Spawn server in a separate task — serve().await blocks until the
        // client sends `initialize`, so both must progress concurrently.
        let server_task = tokio::spawn(async move { server.serve((c2s_r, s2c_w)).await.unwrap() });
        let client = ().serve((s2c_r, c2s_w)).await.unwrap();
        let server_handle = server_task.await.unwrap();
        (client, server_handle)
    }

    /// Sync helper for non-async tests — creates a minimal server.
    fn setup_server_sync() -> MinistrServer {
        let dim = 8;
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
        let index: Arc<dyn ministr_core::index::VectorIndex> =
            Arc::new(HnswIndex::new(dim, 100).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();
        let service = Arc::new(QueryService::new(storage, embedder, index));
        MinistrServer::new(service)
    }

    // --- Session isolation tests (subagent collision fix) ---

    /// Two HTTP-facing forks of the same primary `MinistrServer` MUST get
    /// distinct session ids. This is the core property that prevents the
    /// subagent dedup-leak: if Claude Code's parent and subagent both
    /// reach the same primary daemon's HTTP listener, the registry must
    /// see them as two separate sessions, not one shared shadow.
    #[test]
    fn fork_for_new_session_assigns_distinct_session_ids() {
        let primary = setup_server_sync();
        let original_id = primary.active_session_id().to_string();
        let fork_a = primary.fork_for_new_session();
        let fork_b = primary.fork_for_new_session();

        assert_ne!(fork_a.active_session_id(), original_id);
        assert_ne!(fork_b.active_session_id(), original_id);
        assert_ne!(fork_a.active_session_id(), fork_b.active_session_id());
        // Forks share the registry Arc so the daemon sees all sessions.
        assert!(Arc::ptr_eq(&fork_a.registry_arc(), &fork_b.registry_arc()));
    }

    // --- ServerInfo tests ---

    #[test]
    fn server_info_has_correct_name_and_version() {
        let server = setup_server_sync();
        let info = server.get_info();

        assert_eq!(info.server_info.name, "ministr");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn server_info_enables_tools_capability() {
        let server = setup_server_sync();
        let info = server.get_info();

        assert!(
            info.capabilities.tools.is_some(),
            "tools capability should be enabled"
        );
    }

    #[test]
    fn server_info_has_instructions() {
        let server = setup_server_sync();
        let info = server.get_info();

        let instructions = info.instructions.expect("instructions should be set");
        assert!(instructions.contains("ministr_survey"));
        assert!(instructions.contains("ministr_read"));
        assert!(instructions.contains("ministr_extract"));
    }

    #[test]
    fn server_info_uses_latest_protocol() {
        let server = setup_server_sync();
        let info = server.get_info();

        assert_eq!(info.protocol_version, ProtocolVersion::LATEST);
    }

    // --- Extension declaration tests ---

    #[test]
    fn server_info_advertises_ministr_extensions() {
        let server = setup_server_sync();
        let info = server.get_info();

        let extensions = info
            .capabilities
            .extensions
            .as_ref()
            .expect("extensions capability should be set");

        assert!(
            extensions.contains_key(EXT_USAGE_PROTOCOL),
            "should advertise budget-protocol extension"
        );
        assert!(
            extensions.contains_key(EXT_COHERENCE),
            "should advertise coherence extension"
        );
        assert!(
            extensions.contains_key(EXT_COMPRESSION),
            "should advertise compression extension"
        );
    }

    #[test]
    fn extension_usage_protocol_has_version() {
        let server = setup_server_sync();
        let info = server.get_info();
        let extensions = info.capabilities.extensions.as_ref().unwrap();
        let budget = &extensions[EXT_USAGE_PROTOCOL];
        assert_eq!(budget["version"], serde_json::json!("1"));
    }

    #[test]
    fn extension_compression_has_tiers() {
        let server = setup_server_sync();
        let info = server.get_info();
        let extensions = info.capabilities.extensions.as_ref().unwrap();
        let compression = &extensions[EXT_COMPRESSION];
        assert_eq!(
            compression["tiers"],
            serde_json::json!(["summary", "claims", "full"])
        );
    }

    // --- Extension negotiation tests ---

    #[test]
    fn negotiation_with_no_client_extensions_yields_all_false() {
        let result = NegotiatedExtensions::negotiate(None);
        assert!(!result.usage_protocol);
        assert!(!result.coherence);
        assert!(!result.compression);
    }

    #[test]
    fn negotiation_with_empty_client_extensions_yields_all_false() {
        let empty = ExtensionCapabilities::new();
        let result = NegotiatedExtensions::negotiate(Some(&empty));
        assert!(!result.usage_protocol);
        assert!(!result.coherence);
        assert!(!result.compression);
    }

    #[test]
    fn negotiation_with_matching_client_extensions() {
        let mut client_ext = ExtensionCapabilities::new();
        client_ext.insert(EXT_USAGE_PROTOCOL.to_string(), serde_json::Map::new());
        client_ext.insert(EXT_COHERENCE.to_string(), serde_json::Map::new());
        client_ext.insert(EXT_COMPRESSION.to_string(), serde_json::Map::new());

        let result = NegotiatedExtensions::negotiate(Some(&client_ext));
        assert!(result.usage_protocol);
        assert!(result.coherence);
        assert!(result.compression);
    }

    #[test]
    fn negotiation_partial_match() {
        let mut client_ext = ExtensionCapabilities::new();
        client_ext.insert(EXT_USAGE_PROTOCOL.to_string(), serde_json::Map::new());
        // Client does NOT advertise coherence or compression.

        let result = NegotiatedExtensions::negotiate(Some(&client_ext));
        assert!(result.usage_protocol);
        assert!(!result.coherence);
        assert!(!result.compression);
    }

    #[test]
    fn negotiation_ignores_unknown_client_extensions() {
        let mut client_ext = ExtensionCapabilities::new();
        client_ext.insert("io.example/unknown".to_string(), serde_json::Map::new());

        let result = NegotiatedExtensions::negotiate(Some(&client_ext));
        assert!(!result.usage_protocol);
        assert!(!result.coherence);
        assert!(!result.compression);
    }

    // --- Budget-hint suppression tests ---
    //
    // Budget is tracked internally (so compression/dedup keep working and
    // `ministr_usage` can still report it on explicit request) but is
    // never injected into ordinary tool responses — the per-call numbers
    // made agents wrongly believe they were almost out of context.

    /// Pull `estimated_used` out of a `ministr_usage` tool result. The
    /// budget tool serializes `UsageResponse` directly (no `ToolResponse`
    /// wrapper), so the field is top-level.
    fn extract_estimated_used(result: &rmcp::model::CallToolResult) -> u64 {
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("ministr_usage should be valid JSON: {e}\n{text}"));
        parsed["estimated_used"]
            .as_u64()
            .unwrap_or_else(|| panic!("ministr_usage should report estimated_used: {parsed}"))
    }

    /// The serialized fields an agent must never see in a normal response.
    fn assert_no_budget_hints(parsed: &serde_json::Value) {
        assert!(
            parsed.get("usage_status").is_none() || parsed["usage_status"].is_null(),
            "usage_status must not be surfaced to the agent, got: {parsed}"
        );
        assert!(
            parsed.get("drop_suggestions").is_none() || parsed["drop_suggestions"].is_null(),
            "drop_suggestions must not be surfaced to the agent, got: {parsed}"
        );
    }

    #[tokio::test]
    async fn survey_response_omits_usage_status() {
        let server = setup_server().await;
        let params = SurveyParams {
            query: "JWT authentication tokens".to_string(),
            top_k: Some(5),
        };
        let result = server.survey(Parameters(params)).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("should be valid JSON: {e}\n{text}"));

        assert_no_budget_hints(&parsed);
        // The actual payload is still present.
        assert!(parsed["result"]["results"].is_array());
    }

    #[tokio::test]
    async fn read_response_omits_usage_status() {
        let server = setup_server().await;
        let params = ReadParams {
            section_id: "docs/auth.md#tokens".to_string(),
        };
        let result = server.read(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_no_budget_hints(&parsed);
        assert!(parsed["result"]["text"].is_string());
    }

    #[tokio::test]
    async fn extract_response_omits_usage_status() {
        let server = setup_server().await;
        let params = ExtractParams {
            section_id: "docs/auth.md#tokens".to_string(),
            query: None,
        };
        let result = server.extract(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_no_budget_hints(&parsed);
    }

    /// Internal accumulation still works — it's just only observable via
    /// the explicit `ministr_usage` tool, not injected into every reply.
    #[tokio::test]
    async fn budget_accumulates_internally_across_tool_calls() {
        let server = setup_server().await;

        server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();
        let after_read = extract_estimated_used(&server.usage().await.unwrap());
        assert!(after_read > 0, "should track tokens after read");

        server
            .extract(Parameters(ExtractParams {
                section_id: "docs/auth.md#tokens".to_string(),
                query: None,
            }))
            .await
            .unwrap();
        let after_extract = extract_estimated_used(&server.usage().await.unwrap());
        assert!(
            after_extract > after_read,
            "internal budget should accumulate: {after_extract} > {after_read}"
        );
    }

    // --- Deduplication tests ---

    #[tokio::test]
    async fn survey_deduplicates_already_delivered_content() {
        let server = setup_server().await;

        // First survey — delivers results
        let result1 = server
            .survey(Parameters(SurveyParams {
                query: "JWT authentication tokens".to_string(),
                top_k: Some(10),
            }))
            .await
            .unwrap();
        let text1 = extract_text(&result1.content);
        let parsed1: serde_json::Value = serde_json::from_str(text1).unwrap();
        let first_count = parsed1["result"]["results"].as_array().unwrap().len();
        assert!(first_count > 0, "first survey should return results");

        // Second survey with same query — should filter out delivered content
        let result2 = server
            .survey(Parameters(SurveyParams {
                query: "JWT authentication tokens".to_string(),
                top_k: Some(10),
            }))
            .await
            .unwrap();
        let text2 = extract_text(&result2.content);
        let parsed2: serde_json::Value = serde_json::from_str(text2).unwrap();
        let second_count = parsed2["result"]["results"].as_array().unwrap().len();
        let dedup_count = parsed2["result"]["deduplicated_count"].as_u64().unwrap();

        assert!(
            second_count < first_count,
            "second survey should have fewer results: {second_count} < {first_count}"
        );
        assert!(
            dedup_count > 0,
            "should report deduplicated items: {dedup_count}"
        );
    }

    #[tokio::test]
    async fn read_re_request_skips_unchanged_content() {
        let server = setup_server().await;

        // First read — delivers content
        let result1 = server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();
        let text1 = extract_text(&result1.content);
        let parsed1: serde_json::Value = serde_json::from_str(text1).unwrap();
        assert!(
            parsed1["result"]["text"].is_string(),
            "first read should return full text"
        );

        // Second read — already delivered and unchanged, should skip re-delivery
        let result2 = server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();
        let text2 = extract_text(&result2.content);
        let parsed2: serde_json::Value = serde_json::from_str(text2).unwrap();
        assert_eq!(
            parsed2["result"]["status"], "already_delivered",
            "re-request should return already_delivered status"
        );
        assert!(
            parsed2["result"]["text"].is_null(),
            "re-request should not include full text"
        );
        assert_no_budget_hints(&parsed2);
        assert!(
            parsed2["result"]["claims_available"].is_number(),
            "response should include claims_available"
        );
    }

    // --- Survey response format tests ---

    #[tokio::test]
    async fn survey_response_has_results_array() {
        let server = setup_server().await;
        let params = SurveyParams {
            query: "JWT authentication tokens".to_string(),
            top_k: Some(5),
        };
        let result = server.survey(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(
            parsed["result"]["results"].is_array(),
            "should have results array"
        );
        assert!(
            parsed["result"]["deduplicated_count"].is_number(),
            "should have deduplicated_count"
        );
    }

    // --- Read response format tests ---

    #[tokio::test]
    async fn read_returns_section_without_usage_status() {
        let server = setup_server().await;
        let params = ReadParams {
            section_id: "docs/auth.md#tokens".to_string(),
        };
        let result = server.read(Parameters(params)).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["result"]["section_id"], "docs/auth.md#tokens");
        assert_eq!(parsed["result"]["claims_available"], 2);
        assert!(
            parsed["result"]["text"]
                .as_str()
                .unwrap()
                .contains("JWT tokens")
        );
        assert_no_budget_hints(&parsed);
    }

    #[tokio::test]
    async fn read_not_found_returns_error() {
        let server = setup_server().await;
        let params = ReadParams {
            section_id: "nonexistent#section".to_string(),
        };
        let result = server.read(Parameters(params)).await.unwrap();

        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn extract_not_found_returns_error() {
        let server = setup_server().await;
        let params = ExtractParams {
            section_id: "nonexistent#section".to_string(),
            query: None,
        };
        let result = server.extract(Parameters(params)).await.unwrap();

        assert_eq!(result.is_error, Some(true));
    }

    // --- Error formatting tests ---

    #[test]
    fn format_section_not_found_includes_id_and_hint() {
        let err = QueryError::SectionNotFound {
            id: "docs/missing.md#intro".into(),
        };
        let msg = format_query_error(&err);
        assert!(
            msg.contains("docs/missing.md#intro"),
            "should include section ID"
        );
        assert!(
            msg.contains("ministr_survey"),
            "should suggest using ministr_survey"
        );
    }

    #[test]
    fn format_index_error_includes_details() {
        let err = QueryError::Index(ministr_core::error::IndexError::EmbeddingFailed {
            reason: "model not loaded".into(),
        });
        let msg = format_query_error(&err);
        assert!(
            msg.contains("model not loaded"),
            "should include original reason"
        );
        assert!(msg.contains("index"), "should mention index");
    }

    #[test]
    fn format_storage_error_includes_details() {
        let err = QueryError::Storage(ministr_core::error::StorageError::NotFound {
            entity: "section".into(),
            id: "test-id".into(),
        });
        let msg = format_query_error(&err);
        assert!(msg.contains("test-id"), "should include original details");
        assert!(msg.contains("Storage error"), "should mention storage");
    }

    #[tokio::test]
    async fn read_not_found_error_message_is_user_friendly() {
        let server = setup_server().await;
        let params = ReadParams {
            section_id: "nonexistent#section".to_string(),
        };
        let result = server.read(Parameters(params)).await.unwrap();

        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result.content);
        assert!(
            text.contains("Section not found"),
            "error should start with 'Section not found', got: {text}"
        );
        assert!(
            text.contains("ministr_survey"),
            "error should suggest ministr_survey, got: {text}"
        );
    }

    #[tokio::test]
    async fn extract_not_found_error_message_is_user_friendly() {
        let server = setup_server().await;
        let params = ExtractParams {
            section_id: "nonexistent#section".to_string(),
            query: None,
        };
        let result = server.extract(Parameters(params)).await.unwrap();

        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result.content);
        assert!(
            text.contains("Section not found"),
            "error should be user-friendly, got: {text}"
        );
    }

    // --- Helper function tests ---

    #[test]
    fn content_hash_is_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn content_hash_differs_for_different_input() {
        let h1 = content_hash("hello");
        let h2 = content_hash("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn parse_resolution_handles_all_variants() {
        assert_eq!(parse_resolution("summary"), Resolution::Summary);
        assert_eq!(parse_resolution("claim"), Resolution::Claim);
        // "section" and unknown strings both map to Section (default)
        assert_eq!(parse_resolution("section"), Resolution::Section);
        assert_eq!(parse_resolution("unknown"), Resolution::Section);
    }

    // --- ministr_dropped tests ---

    #[tokio::test]
    async fn evicted_removes_delivered_content() {
        let server = setup_server().await;

        // First deliver some content
        server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();

        // Evict it
        let result = server
            .dropped(Parameters(DroppedParams {
                content_ids: vec!["docs/auth.md#tokens".to_string()],
            }))
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["dropped"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["result"]["not_found"].as_array().unwrap().len(), 0);
        assert_no_budget_hints(&parsed);
        // The eviction really happened — confirmed via the explicit
        // budget tool, not via a field injected into the evict reply.
        assert_eq!(
            extract_estimated_used(&server.usage().await.unwrap()),
            0,
            "internal budget should be zero after evicting all content"
        );
    }

    #[tokio::test]
    async fn evicted_reports_not_found_for_unknown_ids() {
        let server = setup_server().await;

        let result = server
            .dropped(Parameters(DroppedParams {
                content_ids: vec!["nonexistent".to_string()],
            }))
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["dropped"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["result"]["not_found"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn evicted_handles_mixed_known_and_unknown() {
        let server = setup_server().await;

        // Deliver content
        server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();

        // Evict a mix of known and unknown
        let result = server
            .dropped(Parameters(DroppedParams {
                content_ids: vec!["docs/auth.md#tokens".to_string(), "nonexistent".to_string()],
            }))
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["dropped"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["result"]["not_found"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn evicted_empty_list_is_noop() {
        let server = setup_server().await;

        let result = server
            .dropped(Parameters(DroppedParams {
                content_ids: vec![],
            }))
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["dropped"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["result"]["not_found"].as_array().unwrap().len(), 0);
    }

    // --- Fault-based correction tests ---

    #[tokio::test]
    async fn re_request_corrects_window_estimate() {
        let server = setup_server().await;

        // First read — delivers and records in budget
        let result1 = server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();
        let _ = extract_text(&result1.content);
        let used_after_first = extract_estimated_used(&server.usage().await.unwrap());
        assert!(used_after_first > 0, "first read should use tokens");

        // Second read (re-request) — triggers fault correction (force_evict)
        // and returns already_delivered without re-recording
        let result2 = server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();
        let text2 = extract_text(&result2.content);
        let parsed2: serde_json::Value = serde_json::from_str(text2).unwrap();
        assert_eq!(
            parsed2["result"]["status"], "already_delivered",
            "re-request should skip re-delivery"
        );
        assert_no_budget_hints(&parsed2);

        // After fault correction the internal budget is back to 0 —
        // force_evict removed the entry and no re-delivery occurred.
        // Observed via the explicit budget tool, not an injected field.
        assert_eq!(
            extract_estimated_used(&server.usage().await.unwrap()),
            0,
            "internal budget should be zero after fault correction"
        );
    }

    /// The instructions must NOT push a budget protocol at the agent.
    /// `ministr_usage` stays callable, but the prose no longer advertises
    /// it or instructs agents to react to pressure — that's what made
    /// agents wrongly conclude they were low on context.
    #[test]
    fn server_instructions_do_not_advertise_usage_protocol() {
        let server = setup_server_sync();
        let info = server.get_info();
        let instructions = info.instructions.unwrap();

        assert!(
            !instructions.contains("ministr_usage"),
            "instructions must not advertise ministr_usage"
        );
        assert!(
            !instructions.contains("Budget protocol"),
            "instructions must not contain a budget protocol section"
        );
        assert!(
            !instructions.contains("drop_suggestions"),
            "instructions must not tell the agent to act on drop_suggestions"
        );
        // It should explicitly tell the agent ministr is not a
        // low-context signal.
        assert!(
            instructions.contains("do not treat ministr as a signal that you are low on room"),
            "instructions should state ministr is not a low-context signal"
        );
    }

    // --- ministr_usage tests ---

    #[tokio::test]
    async fn budget_returns_status_with_zero_usage() {
        let server = setup_server().await;
        let result = server.usage().await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert!(parsed["total_budget"].is_number());
        assert_eq!(parsed["estimated_used"].as_u64().unwrap(), 0);
        assert_eq!(parsed["level"], "normal");
        assert!(parsed["drop_candidates"].is_array());
        assert!(
            parsed["drop_candidates"].as_array().unwrap().is_empty(),
            "no candidates under normal pressure"
        );
    }

    #[tokio::test]
    async fn budget_returns_candidates_under_pressure() {
        // Use a small budget to easily trigger elevated pressure
        let dim = 8;
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();
        let doc = make_test_doc();
        storage.insert_document(&doc).await.unwrap();

        let texts_and_ids = [(
            "section::docs/auth.md#tokens",
            "JWT tokens use RS256 signing. Tokens expire after 24 hours.",
        )];
        for (id, text) in &texts_and_ids {
            let vecs = embedder.embed(&[*text]).unwrap();
            index.insert(id, &vecs[0]).unwrap();
        }

        let service = Arc::new(QueryService::new(storage, embedder, index));
        let budget_config = UsageConfig {
            max_context_tokens: 20, // Very small — any delivery triggers pressure
            pressure_threshold: 0.5,
            critical_threshold: 0.9,
            ..UsageConfig::default()
        };
        let server = MinistrServer::with_budget_config(service, budget_config);

        // Read a section to fill the budget
        server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();

        // Now check budget — should be under pressure with candidates
        let result = server.usage().await.unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_ne!(parsed["level"], "normal", "should be under pressure");
        assert!(
            !parsed["drop_candidates"].as_array().unwrap().is_empty(),
            "should have eviction candidates under pressure"
        );
    }

    #[tokio::test]
    async fn budget_after_tool_calls_shows_usage() {
        let server = setup_server().await;

        // Read a section to accumulate budget
        server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();

        let result = server.usage().await.unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert!(
            parsed["estimated_used"].as_u64().unwrap() > 0,
            "should show non-zero usage after read"
        );
        assert!(
            parsed["estimated_remaining"].as_u64().unwrap() > 0,
            "should show remaining budget"
        );
    }

    // --- ministr_compress tests ---

    #[tokio::test]
    async fn compress_returns_summaries_for_known_sections() {
        let server = setup_server().await;
        let params = CompressParams {
            content_ids: vec!["docs/auth.md#tokens".to_string()],
        };
        let result = server.compress(Parameters(params)).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let summaries = parsed["result"]["summaries"].as_array().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0]["original_id"], "docs/auth.md#tokens");
        assert!(summaries[0]["summary"].is_string());
        assert!(summaries[0]["original_tokens"].is_number());
        assert!(summaries[0]["compressed_tokens"].is_number());
    }

    #[tokio::test]
    async fn compress_skips_unknown_content_ids() {
        let server = setup_server().await;
        let params = CompressParams {
            content_ids: vec!["nonexistent#section".to_string()],
        };
        let result = server.compress(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let summaries = parsed["result"]["summaries"].as_array().unwrap();
        assert!(
            summaries.is_empty(),
            "unknown content IDs should be silently skipped"
        );
    }

    #[tokio::test]
    async fn compress_omits_usage_status() {
        let server = setup_server().await;
        let params = CompressParams {
            content_ids: vec!["docs/auth.md#tokens".to_string()],
        };
        let result = server.compress(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_no_budget_hints(&parsed);
    }

    #[tokio::test]
    async fn compress_summary_is_shorter_than_original() {
        let server = setup_server().await;
        let params = CompressParams {
            content_ids: vec!["docs/auth.md#tokens".to_string()],
        };
        let result = server.compress(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let summaries = parsed["result"]["summaries"].as_array().unwrap();
        if !summaries.is_empty() {
            let original = summaries[0]["original_tokens"].as_u64().unwrap();
            let compressed = summaries[0]["compressed_tokens"].as_u64().unwrap();
            assert!(
                compressed <= original,
                "compressed ({compressed}) should be <= original ({original})"
            );
        }
    }

    #[tokio::test]
    async fn compress_mixed_known_and_unknown() {
        let server = setup_server().await;

        // Compress a mix of known and unknown section IDs
        let params = CompressParams {
            content_ids: vec![
                "docs/auth.md#tokens".to_string(),
                "nonexistent#missing".to_string(),
            ],
        };
        let result = server.compress(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let summaries = parsed["result"]["summaries"].as_array().unwrap();
        assert_eq!(
            summaries.len(),
            1,
            "should only return summary for the known section"
        );
    }

    // --- Resource tests ---

    #[test]
    fn server_info_enables_resources_capability() {
        let server = setup_server_sync();
        let info = server.get_info();

        assert!(
            info.capabilities.resources.is_some(),
            "resources capability should be enabled"
        );
    }

    #[tokio::test]
    async fn list_resources_returns_status_resource() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client.peer().list_resources(None).await.unwrap();

        assert_eq!(result.resources.len(), 2);
        let status = result
            .resources
            .iter()
            .find(|r| r.uri == "ministr://status")
            .expect("should include ministr://status resource");
        assert_eq!(status.name, "ministr status");
        assert!(status.description.is_some());
        assert_eq!(status.mime_type.as_deref(), Some("application/json"));
    }

    #[tokio::test]
    async fn list_resource_templates_returns_corpus_template() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client.peer().list_resource_templates(None).await.unwrap();

        assert_eq!(result.resource_templates.len(), 1);
        assert_eq!(
            result.resource_templates[0].uri_template,
            "ministr://corpus/{path}"
        );
        assert_eq!(result.resource_templates[0].name, "corpus document");
    }

    #[tokio::test]
    async fn read_status_resource_returns_index_and_session_info() {
        let server = setup_server().await;
        let result = server.read_status_resource().await.unwrap();

        assert_eq!(result.contents.len(), 1);
        let text = match &result.contents[0] {
            ResourceContents::TextResourceContents { uri, text, .. } => {
                assert_eq!(uri, "ministr://status");
                text
            }
            ResourceContents::BlobResourceContents { .. } => {
                panic!("expected text resource contents")
            }
        };

        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(parsed["index"]["vector_count"].is_number());
        assert!(parsed["index"]["dimension"].is_number());
        assert!(parsed["session"]["id"].is_string());
        assert!(parsed["session"]["delivered_count"].is_number());
        assert!(parsed["budget"].is_object());
    }

    #[tokio::test]
    async fn read_corpus_resource_returns_document_metadata() {
        let server = setup_server().await;
        let result = server.read_corpus_resource("docs/auth.md").await.unwrap();

        assert_eq!(result.contents.len(), 1);
        let text = match &result.contents[0] {
            ResourceContents::TextResourceContents { uri, text, .. } => {
                assert_eq!(uri, "ministr://corpus/docs/auth.md");
                text
            }
            ResourceContents::BlobResourceContents { .. } => {
                panic!("expected text resource contents")
            }
        };

        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["id"], "docs/auth.md");
        assert_eq!(parsed["title"], "Authentication Guide");
        assert_eq!(parsed["source_path"], "docs/auth.md");
        assert_eq!(parsed["section_count"], 1);
    }

    #[tokio::test]
    async fn read_corpus_resource_unknown_path_returns_error() {
        let server = setup_server().await;
        let result = server.read_corpus_resource("nonexistent.md").await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("document not found"),
            "error should mention document not found: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn read_resource_dispatches_status_uri() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .read_resource(ReadResourceRequestParams::new("ministr://status"))
            .await
            .unwrap();

        assert_eq!(result.contents.len(), 1);
    }

    #[tokio::test]
    async fn read_resource_dispatches_corpus_uri() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .read_resource(ReadResourceRequestParams::new(
                "ministr://corpus/docs/auth.md",
            ))
            .await
            .unwrap();

        assert_eq!(result.contents.len(), 1);
    }

    #[tokio::test]
    async fn read_resource_unknown_uri_returns_error() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .read_resource(ReadResourceRequestParams::new("ministr://unknown"))
            .await;

        assert!(result.is_err());
    }

    // --- ministr_clone tests ---

    #[tokio::test]
    async fn clone_without_git_fetcher_returns_error() {
        let server = setup_server().await;
        let result = server
            .clone_repo(
                Parameters(CloneParams {
                    repo: "https://github.com/octocat/Hello-World.git".to_string(),
                    paths: None,
                    branch: None,
                }),
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert!(result.is_error.unwrap_or(false));
        let text = extract_text(&result.content);
        assert!(
            text.contains("not available"),
            "should say not available: {text}"
        );
    }

    #[tokio::test]
    async fn clone_with_git_fetcher_invalid_repo_returns_error() {
        let dim = 8;
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 100).unwrap());
        let storage = Arc::new(SqliteStorage::open_in_memory().unwrap());
        let service = Arc::new(QueryService::new(
            (*storage).clone(),
            Arc::clone(&embedder),
            Arc::clone(&index),
        ));

        let git_config = ministr_core::git::GitFetcherConfig {
            remote_dir: std::path::PathBuf::from("/tmp/ministr-test-clone"),
            ..ministr_core::git::GitFetcherConfig::default()
        };
        let git_fetcher = GitFetcher::new(git_config);

        let server =
            MinistrServer::with_persistence(service, UsageConfig::default(), storage, None)
                .await
                .with_git_fetcher(git_fetcher, embedder, index);

        let result = server
            .clone_repo(
                Parameters(CloneParams {
                    repo: String::new(),
                    paths: None,
                    branch: None,
                }),
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert!(result.is_error.unwrap_or(false));
        let text = extract_text(&result.content);
        assert!(
            text.contains("clone failed"),
            "should report clone failure: {text}"
        );
    }

    #[test]
    fn with_git_fetcher_sets_field() {
        let dim = 8;
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 100).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();
        let service = Arc::new(QueryService::new(
            storage,
            Arc::clone(&embedder),
            Arc::clone(&index),
        ));
        let server = MinistrServer::new(service);

        assert!(server.git_fetcher.is_none());

        let git_fetcher = GitFetcher::with_defaults();
        let server = server.with_git_fetcher(git_fetcher, embedder, index);

        assert!(server.git_fetcher.is_some());
    }

    // Progress notification tests removed — Peer::new() is pub(crate) in rmcp 0.16.
    // Progress behavior is exercised by the e2e tests through the MCP protocol layer.

    // --- Budget-pressure suppression tests ---
    //
    // These used to assert that drop_suggestions + usage_status
    // appeared in responses under pressure. The new contract is the
    // opposite: even with a tiny budget that is provably saturated, none
    // of it is surfaced to the agent. Internal tracking still runs (the
    // `ministr_usage` tool can still report real numbers on request).

    /// Helper: create a server with a tiny budget so any delivery triggers pressure.
    async fn setup_pressured_server() -> MinistrServer {
        let dim = 8;
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 100).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let doc = make_test_doc();
        storage.insert_document(&doc).await.unwrap();

        let texts_and_ids = [(
            "section::docs/auth.md#tokens",
            "JWT tokens use RS256 signing. Tokens expire after 24 hours.",
        )];
        for (id, text) in &texts_and_ids {
            let vecs = embedder.embed(&[*text]).unwrap();
            index.insert(id, &vecs[0]).unwrap();
        }

        let service = Arc::new(QueryService::new(storage, embedder, index));
        let budget_config = UsageConfig {
            max_context_tokens: 20, // Very small — any delivery triggers pressure
            pressure_threshold: 0.5,
            critical_threshold: 0.9,
            ..UsageConfig::default()
        };
        MinistrServer::with_budget_config(service, budget_config)
    }

    #[tokio::test]
    async fn no_budget_hints_at_normal_pressure() {
        let server = setup_server().await;

        let result = server
            .toc(Parameters(TocParams {
                document_id: None,
                offset: None,
                limit: None,
            }))
            .await
            .unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_no_budget_hints(&parsed);
    }

    /// The key regression: a provably-saturated budget must STILL leak
    /// nothing to the agent. Internal pressure is real here (tiny 20-token
    /// window, content delivered) — yet the response carries no
    /// `usage_status` and no `drop_suggestions`.
    #[tokio::test]
    async fn saturated_budget_still_leaks_no_hints_via_toc() {
        let server = setup_pressured_server().await;

        server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();

        let result = server
            .toc(Parameters(TocParams {
                document_id: None,
                offset: None,
                limit: None,
            }))
            .await
            .unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_no_budget_hints(&parsed);

        // Internal tracking is genuinely under pressure — confirm via the
        // explicit budget tool so this isn't a false-negative from the
        // budget simply not being exercised.
        let budget = server.usage().await.unwrap();
        let btext = extract_text(&budget.content);
        let bparsed: serde_json::Value = serde_json::from_str(btext).unwrap();
        assert_ne!(
            bparsed["level"], "normal",
            "internal budget should actually be under pressure"
        );
    }

    #[tokio::test]
    async fn saturated_budget_still_leaks_no_hints_via_survey() {
        let server = setup_pressured_server().await;

        let result = server
            .survey(Parameters(SurveyParams {
                query: "JWT tokens".to_string(),
                top_k: Some(5),
            }))
            .await
            .unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_no_budget_hints(&parsed);
    }

    // --- Resource subscription tests ---

    #[tokio::test]
    async fn subscribe_adds_uri_to_set() {
        let server = setup_server().await;
        let subs = server.subscriptions.lock().await;
        assert!(subs.is_empty());
        drop(subs);

        // Manually add to subscriptions (simulating subscribe handler).
        server
            .subscriptions
            .lock()
            .await
            .insert("ministr://status".to_string());
        let subs = server.subscriptions.lock().await;
        assert!(subs.contains("ministr://status"));
    }

    #[tokio::test]
    async fn subscribe_is_idempotent() {
        let server = setup_server().await;
        let subs = &server.subscriptions;
        subs.lock().await.insert("ministr://status".to_string());
        subs.lock().await.insert("ministr://status".to_string());
        assert_eq!(subs.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn unsubscribe_removes_uri() {
        let server = setup_server().await;
        server
            .subscriptions
            .lock()
            .await
            .insert("ministr://status".to_string());
        server.subscriptions.lock().await.remove("ministr://status");
        assert!(server.subscriptions.lock().await.is_empty());
    }

    #[tokio::test]
    async fn unsubscribe_nonexistent_is_noop() {
        let server = setup_server().await;
        server
            .subscriptions
            .lock()
            .await
            .remove("ministr://nonexistent");
        assert!(server.subscriptions.lock().await.is_empty());
    }

    #[tokio::test]
    async fn capabilities_include_subscribe() {
        let server = setup_server().await;
        let info = server.get_info();
        let resources = info.capabilities.resources.expect("resources capability");
        assert_eq!(resources.subscribe, Some(true));
    }

    #[tokio::test]
    async fn set_coherence_receiver_stores_receiver() {
        let server = setup_server().await;
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<String>>();
        server.set_coherence_receiver(rx);
        let guard = server.coherence_rx.lock().await;
        assert!(
            guard.is_some(),
            "coherence_rx should be set after set_coherence_receiver"
        );
    }

    #[tokio::test]
    async fn subscription_notifier_sends_when_subscribed() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<String>>();
        let peer_lock: Arc<Mutex<Option<Peer<RoleServer>>>> = Arc::new(Mutex::new(None));
        let subscriptions = Arc::new(Mutex::new(HashSet::new()));

        // Without a peer, the notifier should exit cleanly when it receives events.
        subscriptions
            .lock()
            .await
            .insert("ministr://status".to_string());

        let subs_clone = Arc::clone(&subscriptions);
        let peer_clone = Arc::clone(&peer_lock);
        let handle = tokio::spawn(async move {
            run_subscription_notifier(rx, peer_clone, subs_clone).await;
        });

        // Send a coherence event.
        tx.send(vec!["docs/auth.md#tokens".to_string()]).unwrap();

        // The notifier should exit because there's no peer.
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(
            result.is_ok(),
            "notifier should exit when no peer is available"
        );
    }

    #[tokio::test]
    async fn subscription_notifier_skips_when_no_subscriptions() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<String>>();
        let peer_lock: Arc<Mutex<Option<Peer<RoleServer>>>> = Arc::new(Mutex::new(None));
        let subscriptions = Arc::new(Mutex::new(HashSet::new()));
        // No subscriptions — notifier should skip the event and wait for more.

        let subs_clone = Arc::clone(&subscriptions);
        let peer_clone = Arc::clone(&peer_lock);
        let handle = tokio::spawn(async move {
            run_subscription_notifier(rx, peer_clone, subs_clone).await;
        });

        // Send event with no subscriptions active — should be skipped.
        tx.send(vec!["docs/auth.md#tokens".to_string()]).unwrap();

        // Drop sender to close the channel, causing notifier to exit.
        drop(tx);

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "notifier should exit when channel closes");
    }

    // --- ministr_task tests ---

    #[tokio::test]
    async fn task_status_unknown_id_returns_error() {
        let server = setup_server().await;
        let result = server
            .task_status(Parameters(TaskParams {
                task_id: "nonexistent-task-id".to_string(),
            }))
            .await
            .unwrap();

        assert!(result.is_error.unwrap_or(false));
        let text = extract_text(&result.content);
        assert!(
            text.contains("unknown task ID"),
            "should say unknown task: {text}"
        );
    }

    #[tokio::test]
    async fn task_status_returns_working_task() {
        let server = setup_server().await;
        let task = server.task_manager.create("test task", None, None).await;

        let result = server
            .task_status(Parameters(TaskParams {
                task_id: task.task_id.clone(),
            }))
            .await
            .unwrap();

        assert!(result.is_error.is_none() || !result.is_error.unwrap());
        let text = extract_text(&result.content);
        assert!(text.contains("working"), "should show working: {text}");
        assert!(text.contains("test task"), "should show message: {text}");
    }

    #[tokio::test]
    async fn task_status_returns_completed_task() {
        let server = setup_server().await;
        let task = server.task_manager.create("test task", None, None).await;
        let tool_result = CallToolResult::success(vec![Content::text(
            serde_json::json!({"pages": 5}).to_string(),
        )]);
        server
            .task_manager
            .complete(&task.task_id, tool_result)
            .await;

        let result = server
            .task_status(Parameters(TaskParams {
                task_id: task.task_id.clone(),
            }))
            .await
            .unwrap();

        assert!(result.is_error.is_none() || !result.is_error.unwrap());
        let text = extract_text(&result.content);
        assert!(text.contains("completed"), "should show completed: {text}");
    }

    #[tokio::test]
    async fn task_status_returns_failed_task() {
        let server = setup_server().await;
        let task = server.task_manager.create("test task", None, None).await;
        server
            .task_manager
            .fail(&task.task_id, "something broke")
            .await;

        let result = server
            .task_status(Parameters(TaskParams {
                task_id: task.task_id.clone(),
            }))
            .await
            .unwrap();

        assert!(result.is_error.is_none() || !result.is_error.unwrap());
        let text = extract_text(&result.content);
        assert!(text.contains("failed"), "should show failed: {text}");
        assert!(
            text.contains("something broke"),
            "should include error: {text}"
        );
    }

    // -- Structured output and annotation tests --

    /// Verify that `structured_result` produces both `structured_content` and text fallback.
    #[test]
    fn structured_result_sets_both_content_and_structured_content() {
        #[derive(Serialize)]
        struct Demo {
            value: u32,
        }
        let result = structured_result(&Demo { value: 42 }).unwrap();

        // structured_content must be present
        let sc = result
            .structured_content
            .as_ref()
            .expect("structured_content missing");
        assert_eq!(sc["value"], 42);

        // text fallback must also be present for backward compatibility
        let text = extract_text(&result.content);
        assert!(
            text.contains("42"),
            "text fallback should contain the value"
        );

        // isError should be false
        assert_eq!(result.is_error, Some(false));
    }

    /// Verify that all 15 tool definitions have output schemas.
    #[tokio::test]
    async fn all_tools_have_output_schema() {
        let server = setup_server().await;
        let (client, _server_handle) = wrap_test_client(server).await;

        let tools = client.list_all_tools().await.unwrap();
        assert!(
            tools.len() >= 15,
            "expected at least 15 tools, got {}",
            tools.len()
        );

        let missing: Vec<&str> = tools
            .iter()
            .filter(|t| t.output_schema.is_none())
            .map(|t| t.name.as_ref())
            .collect();

        assert!(
            missing.is_empty(),
            "tools missing output_schema: {missing:?}"
        );
    }

    /// Verify that all 15 tool definitions have annotations.
    #[tokio::test]
    async fn all_tools_have_annotations() {
        let server = setup_server().await;
        let (client, _server_handle) = wrap_test_client(server).await;

        let tools = client.list_all_tools().await.unwrap();

        let missing: Vec<&str> = tools
            .iter()
            .filter(|t| t.annotations.is_none())
            .map(|t| t.name.as_ref())
            .collect();

        assert!(missing.is_empty(), "tools missing annotations: {missing:?}");
    }

    /// Verify specific annotation values for read-only vs mutating tools.
    #[tokio::test]
    async fn read_only_tools_have_correct_annotations() {
        let server = setup_server().await;
        let (client, _server_handle) = wrap_test_client(server).await;

        let tools = client.list_all_tools().await.unwrap();

        let read_only_tools = [
            "ministr_survey",
            "ministr_read",
            "ministr_extract",
            "ministr_related",
            "ministr_usage",
            "ministr_toc",
            "ministr_task",
            "ministr_symbols",
            "ministr_definition",
            "ministr_references",
        ];

        let mutating_tools = [
            "ministr_dropped",
            "ministr_compress",
            "ministr_fetch",
            "ministr_refresh",
            "ministr_clone",
        ];

        for tool in &tools {
            let name = tool.name.as_ref();
            let ann = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("missing annotations for {name}"));

            if read_only_tools.contains(&name) {
                assert_eq!(ann.read_only_hint, Some(true), "{name} should be read-only");
            } else if mutating_tools.contains(&name) {
                assert_eq!(
                    ann.read_only_hint,
                    Some(false),
                    "{name} should not be read-only"
                );
            }
        }
    }

    /// Verify `open_world_hint`: fetch/refresh/clone are open, others closed.
    #[tokio::test]
    async fn open_world_tools_have_correct_hint() {
        let server = setup_server().await;
        let (client, _server_handle) = wrap_test_client(server).await;

        let tools = client.list_all_tools().await.unwrap();

        let open_world = ["ministr_fetch", "ministr_refresh", "ministr_clone"];

        for tool in &tools {
            let name = tool.name.as_ref();
            let ann = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("missing annotations for {name}"));

            if open_world.contains(&name) {
                assert_eq!(
                    ann.open_world_hint,
                    Some(true),
                    "{name} should be open-world"
                );
            } else {
                assert_eq!(
                    ann.open_world_hint,
                    Some(false),
                    "{name} should be closed-world"
                );
            }
        }
    }

    /// Verify that `ministr_survey` returns structured content.
    #[tokio::test]
    async fn survey_returns_structured_content() {
        let server = setup_server().await;
        let (client, _server_handle) = wrap_test_client(server).await;

        let args: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({"query": "authentication"})).unwrap();
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("ministr_survey").with_arguments(args),
            )
            .await
            .unwrap();

        // Must have structured_content
        let sc = result
            .structured_content
            .as_ref()
            .expect("ministr_survey should return structured_content");

        // The structured content wraps the payload under `result`; the
        // budget fields are deliberately absent.
        assert!(
            sc.get("result").is_some(),
            "structured_content should contain result, got: {sc:?}"
        );
        assert!(
            sc.get("usage_status").is_none(),
            "structured_content must not surface usage_status, got: {sc:?}"
        );

        // Must also have text fallback
        assert!(
            !result.content.is_empty(),
            "should have text fallback content"
        );
    }

    /// Verify that the output schema for `ToolResponse<SurveyResponse>` is valid JSON Schema.
    #[test]
    fn tool_output_schema_generates_valid_schema() {
        let schema = tool_output_schema::<ToolResponse<SurveyResponse>>();

        // Must be a JSON object with "type": "object"
        assert_eq!(
            schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "output schema should be type object"
        );

        // Should have properties or allOf (due to flatten)
        assert!(
            schema.contains_key("properties") || schema.contains_key("allOf"),
            "output schema should define properties: {schema:?}"
        );
    }

    /// Verify that `TaskStatusResponse` generates a valid schema.
    #[test]
    fn task_status_schema_is_valid() {
        let schema = tool_output_schema::<TaskStatusResponse>();
        assert!(
            schema.contains_key("properties"),
            "TaskStatusResponse schema should have properties: {schema:?}"
        );
    }

    /// Verify that tasks capability is declared.
    #[tokio::test]
    async fn capabilities_include_tasks() {
        let server = setup_server().await;
        let info = server.get_info();
        assert!(
            info.capabilities.tasks.is_some(),
            "server should declare tasks capability"
        );
    }

    // --- MCP Task manager integration tests ---

    #[tokio::test]
    async fn mcp_task_list_returns_active_tasks() {
        let server = setup_server().await;
        server.task_manager.create("task 1", None, None).await;
        server.task_manager.create("task 2", None, None).await;

        let task_list = server.task_manager.list_tasks().await;
        assert_eq!(task_list.tasks.len(), 2);
    }

    #[tokio::test]
    async fn mcp_task_get_returns_metadata() {
        let server = setup_server().await;
        let task = server.task_manager.create("indexing…", None, None).await;

        let info = server.task_manager.get_task(&task.task_id).await.unwrap();

        assert_eq!(info.status, rmcp::model::TaskStatus::Working);
        assert_eq!(info.status_message.as_deref(), Some("indexing…"));
        assert!(info.poll_interval.is_some());
    }

    #[tokio::test]
    async fn mcp_task_result_available_after_completion() {
        let server = setup_server().await;
        let task = server.task_manager.create("test", None, None).await;
        let tool_result = CallToolResult::success(vec![Content::text("hello")]);
        server
            .task_manager
            .complete(&task.task_id, tool_result)
            .await;

        let result = server.task_manager.get_result(&task.task_id).await;
        assert!(
            result.is_some(),
            "result should be available after completion"
        );
    }

    #[tokio::test]
    async fn mcp_task_result_unavailable_when_working() {
        let server = setup_server().await;
        let task = server.task_manager.create("test", None, None).await;

        let result = server.task_manager.get_result(&task.task_id).await;
        assert!(result.is_none(), "result should be None while working");
    }

    #[tokio::test]
    async fn mcp_task_cancel_transitions_state() {
        let server = setup_server().await;
        let handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        let task = server
            .task_manager
            .create("long op", Some(handle), None)
            .await;

        let cancelled = server.task_manager.cancel(&task.task_id).await.unwrap();
        assert_eq!(cancelled.status, rmcp::model::TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn fetch_tool_has_optional_task_support() {
        let server = setup_server().await;
        let fetch_tool = server
            .get_tool("ministr_fetch")
            .expect("ministr_fetch tool not found");
        assert_eq!(
            fetch_tool.task_support(),
            rmcp::model::TaskSupport::Optional,
            "ministr_fetch should support optional task mode"
        );
    }

    #[tokio::test]
    async fn clone_tool_has_optional_task_support() {
        let server = setup_server().await;
        let clone_tool = server
            .get_tool("ministr_clone")
            .expect("ministr_clone tool not found");
        assert_eq!(
            clone_tool.task_support(),
            rmcp::model::TaskSupport::Optional,
            "ministr_clone should support optional task mode"
        );
    }

    // --- Prompts tests ---

    #[test]
    fn server_info_enables_prompts_capability() {
        let server = setup_server_sync();
        let info = server.get_info();
        assert!(
            info.capabilities.prompts.is_some(),
            "prompts capability should be enabled"
        );
    }

    #[test]
    fn server_info_enables_completions_capability() {
        let server = setup_server_sync();
        let info = server.get_info();
        assert!(
            info.capabilities.completions.is_some(),
            "completions capability should be enabled"
        );
    }

    #[tokio::test]
    async fn list_prompts_returns_three_prompts() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client.peer().list_prompts(None).await.unwrap();
        assert_eq!(result.prompts.len(), 3, "should have 3 prompts");
        let names: Vec<&str> = result.prompts.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"session-summary"));
        assert!(names.contains(&"what-next"));
        assert!(names.contains(&"dependency-chain"));
    }

    #[tokio::test]
    async fn get_prompt_session_summary_returns_messages() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .get_prompt(GetPromptRequestParams::new("session-summary"))
            .await
            .unwrap();
        assert!(
            !result.messages.is_empty(),
            "should return at least one message"
        );
        assert!(result.description.is_some(), "should have a description");
    }

    #[tokio::test]
    async fn get_prompt_what_next_returns_messages() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .get_prompt(GetPromptRequestParams::new("what-next"))
            .await
            .unwrap();
        assert!(
            !result.messages.is_empty(),
            "should return at least one message"
        );
    }

    #[tokio::test]
    async fn get_prompt_dependency_chain_returns_messages() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let mut args = serde_json::Map::new();
        args.insert("concept".into(), serde_json::json!("JWT"));
        let result = client
            .peer()
            .get_prompt(GetPromptRequestParams::new("dependency-chain").with_arguments(args))
            .await
            .unwrap();
        assert!(
            !result.messages.is_empty(),
            "should return at least one message"
        );
    }

    #[tokio::test]
    async fn get_prompt_dependency_chain_no_results() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let mut args = serde_json::Map::new();
        args.insert("concept".into(), serde_json::json!("nonexistent-topic-xyz"));
        let result = client
            .peer()
            .get_prompt(GetPromptRequestParams::new("dependency-chain").with_arguments(args))
            .await
            .unwrap();
        assert!(!result.messages.is_empty());
    }

    #[tokio::test]
    async fn get_prompt_unknown_name_returns_error() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .get_prompt(GetPromptRequestParams::new("nonexistent"))
            .await;
        assert!(result.is_err(), "should error for unknown prompt");
    }

    // --- Completions tests ---

    #[tokio::test]
    async fn complete_section_ids_matches() {
        let server = setup_server().await;
        let results = server.complete_section_ids("auth").await;
        assert!(
            results.iter().any(|v| v.contains("auth")),
            "should complete section IDs matching 'auth', got: {results:?}"
        );
    }

    #[tokio::test]
    async fn complete_corpus_paths_matches() {
        let server = setup_server().await;
        let results = server.complete_corpus_paths("auth").await;
        assert!(
            results.iter().any(|v| v.contains("auth")),
            "should complete corpus paths matching 'auth', got: {results:?}"
        );
    }

    #[tokio::test]
    async fn complete_unknown_ref_returns_empty() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .complete(CompleteRequestParams::new(
                Reference::for_prompt("unknown"),
                rmcp::model::ArgumentInfo {
                    name: "arg".into(),
                    value: "val".into(),
                },
            ))
            .await
            .unwrap();
        assert!(
            result.completion.values.is_empty(),
            "should return empty for unknown prompt"
        );
    }

    // --- Server Card tests (SEP-1649) ---

    #[test]
    fn server_card_has_required_schema_fields() {
        let server = setup_server_sync();
        let card = server.build_server_card();

        assert_eq!(
            card["$schema"],
            "https://static.modelcontextprotocol.io/schemas/mcp-server-card/v1.json"
        );
        assert_eq!(card["version"], "1.0");
        assert!(
            card["protocolVersion"].is_string(),
            "protocolVersion should be a string"
        );
    }

    #[test]
    fn server_card_has_server_info() {
        let server = setup_server_sync();
        let card = server.build_server_card();

        assert_eq!(card["serverInfo"]["name"], "ministr");
        assert_eq!(card["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(
            card["serverInfo"]["description"].is_string(),
            "description should be present"
        );
    }

    #[test]
    fn server_card_has_capabilities() {
        let server = setup_server_sync();
        let card = server.build_server_card();

        let caps = &card["capabilities"];
        assert!(
            caps["tools"].is_object(),
            "tools capability should be present"
        );
        assert!(
            caps["resources"].is_object(),
            "resources capability should be present"
        );
        assert!(
            caps["prompts"].is_object(),
            "prompts capability should be present"
        );
        assert!(
            caps["tasks"].is_object(),
            "tasks capability should be present"
        );
        assert!(
            caps["completions"].is_object(),
            "completions capability should be present"
        );
    }

    #[test]
    fn server_card_includes_ministr_extensions() {
        let server = setup_server_sync();
        let card = server.build_server_card();

        let extensions = &card["capabilities"]["extensions"];
        assert!(
            extensions[EXT_USAGE_PROTOCOL].is_object(),
            "should include budget-protocol extension"
        );
        assert!(
            extensions[EXT_COHERENCE].is_object(),
            "should include coherence extension"
        );
        assert!(
            extensions[EXT_COMPRESSION].is_object(),
            "should include compression extension"
        );
    }

    #[test]
    fn server_card_includes_full_tool_catalog() {
        let server = setup_server_sync();
        let card = server.build_server_card();

        let tools = card["tools"].as_array().expect("tools should be an array");
        // All 15 ministr tools should be listed.
        assert!(
            tools.len() >= 15,
            "should have at least 15 tools, got {}",
            tools.len()
        );

        let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        for expected in &[
            "ministr_survey",
            "ministr_read",
            "ministr_extract",
            "ministr_related",
            "ministr_dropped",
            "ministr_usage",
            "ministr_compress",
            "ministr_toc",
            "ministr_fetch",
            "ministr_refresh",
            "ministr_clone",
            "ministr_task",
            "ministr_symbols",
            "ministr_definition",
            "ministr_references",
        ] {
            assert!(
                tool_names.contains(expected),
                "tool catalog should include {expected}"
            );
        }

        // Every tool should have a description.
        for tool in tools {
            assert!(
                tool["description"].is_string(),
                "tool {} should have a description",
                tool["name"]
            );
        }
    }

    #[tokio::test]
    async fn list_resources_includes_server_card() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client.peer().list_resources(None).await.unwrap();

        let uris: Vec<&str> = result.resources.iter().map(|r| r.uri.as_str()).collect();
        assert!(
            uris.contains(&"mcp://server-card.json"),
            "list_resources should include mcp://server-card.json, got: {uris:?}"
        );
    }

    #[tokio::test]
    async fn read_resource_dispatches_server_card_uri() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .read_resource(ReadResourceRequestParams::new("mcp://server-card.json"))
            .await
            .unwrap();

        assert_eq!(result.contents.len(), 1);
        let text = match &result.contents[0] {
            ResourceContents::TextResourceContents { uri, text, .. } => {
                assert_eq!(uri, "mcp://server-card.json");
                text
            }
            ResourceContents::BlobResourceContents { .. } => {
                panic!("expected text resource contents")
            }
        };

        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["serverInfo"]["name"], "ministr");
        assert!(parsed["tools"].is_array());
    }

    #[test]
    fn server_card_resource_is_valid_json() {
        let server = setup_server_sync();
        let result = server.read_server_card_resource();

        let text = match &result.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text,
            ResourceContents::BlobResourceContents { .. } => {
                panic!("expected text resource contents")
            }
        };

        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["version"], "1.0");
    }

    #[tokio::test]
    async fn toc_pagination_defaults_to_100() {
        let server = setup_server().await;
        let result = server
            .toc(Parameters(TocParams {
                document_id: None,
                offset: None,
                limit: None,
            }))
            .await
            .unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        // Our test corpus has 1 section — verify pagination metadata
        assert_eq!(parsed["result"]["corpus_stats"]["sections"], 1);
        assert_eq!(parsed["result"]["corpus_stats"]["offset"], 0);
        assert_eq!(parsed["result"]["corpus_stats"]["returned"], 1);
        assert_eq!(parsed["result"]["entries"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn toc_pagination_with_offset_and_limit() {
        let server = setup_server().await;

        // Offset past all entries → empty page
        let result = server
            .toc(Parameters(TocParams {
                document_id: None,
                offset: Some(100),
                limit: Some(10),
            }))
            .await
            .unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["corpus_stats"]["sections"], 1);
        assert_eq!(parsed["result"]["corpus_stats"]["offset"], 100);
        assert_eq!(parsed["result"]["corpus_stats"]["returned"], 0);
        assert!(parsed["result"]["entries"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn symbols_pagination_defaults() {
        let server = setup_server().await;
        let result = server
            .symbols(Parameters(SymbolsParams {
                query: None,
                kind: None,
                module: None,
                visibility: None,
                offset: None,
                limit: None,
            }))
            .await
            .unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        // Test corpus has no symbols — verify pagination metadata defaults
        assert_eq!(parsed["result"]["total"], 0);
        assert_eq!(parsed["result"]["offset"], 0);
        assert!(parsed["result"]["symbols"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn symbols_pagination_with_offset_and_limit() {
        let server = setup_server().await;
        let result = server
            .symbols(Parameters(SymbolsParams {
                query: None,
                kind: None,
                module: None,
                visibility: None,
                offset: Some(50),
                limit: Some(10),
            }))
            .await
            .unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["total"], 0);
        assert_eq!(parsed["result"]["offset"], 50);
        assert!(parsed["result"]["symbols"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn references_pagination_defaults() {
        let server = setup_server().await;
        // Non-existent symbol returns an error (no symbol table in test corpus).
        // Verify the handler doesn't panic and returns a valid CallToolResult.
        let result = server
            .references(Parameters(ReferencesParams {
                symbol_id: "nonexistent::symbol".into(),
                ref_kind: None,
                offset: None,
                limit: None,
            }))
            .await
            .unwrap();
        // Error path returns plain text, not JSON — just verify it completes.
        assert!(!result.content.is_empty());
    }

    #[test]
    fn references_response_includes_offset_and_total() {
        let response = ReferencesResponse {
            references: vec![],
            total: 42,
            offset: 10,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["total"], 42);
        assert_eq!(json["offset"], 10);
        assert!(json["references"].as_array().unwrap().is_empty());
    }

    #[test]
    fn symbols_response_includes_offset_and_total() {
        let response = SymbolsResponse {
            symbols: vec![],
            total: 99,
            offset: 20,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["total"], 99);
        assert_eq!(json["offset"], 20);
        assert!(json["symbols"].as_array().unwrap().is_empty());
    }

    #[test]
    fn response_size_guard_injects_warning_for_large_payloads() {
        // Create a JSON object larger than MAX_RESPONSE_BYTES
        let big_string = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let v = serde_json::json!({ "data": big_string });
        let result = apply_response_size_guard(v);
        assert!(
            result.get("_truncation_warning").is_some(),
            "expected _truncation_warning for large payload"
        );
        let warning = &result["_truncation_warning"];
        assert!(warning["response_bytes"].as_u64().unwrap() > MAX_RESPONSE_BYTES as u64);
        assert_eq!(
            warning["threshold_bytes"].as_u64().unwrap(),
            MAX_RESPONSE_BYTES as u64
        );
    }

    #[test]
    fn response_size_guard_skips_small_payloads() {
        let v = serde_json::json!({ "data": "small" });
        let result = apply_response_size_guard(v);
        assert!(
            result.get("_truncation_warning").is_none(),
            "should not inject warning for small payload"
        );
    }

    #[test]
    fn tool_response_prefix_stability() {
        use ministr_core::session::{UsageLevel, UsageStatus};

        // Two different tool result types with identical budget status
        let budget = UsageStatus {
            tokens_used: 5000,
            tokens_remaining: 95_000,
            level: UsageLevel::Normal,
            utilization: 0.05,
        };

        let resp_a = ToolResponse {
            usage_status: budget.clone(),
            coherence_alerts: Vec::new(),
            indexing_in_progress: false,
            indexing_message: None,
            drop_suggestions: Vec::new(),
            next_actions: Vec::new(),
            result: serde_json::json!({"results": [1, 2, 3]}),
        };
        let resp_b = ToolResponse {
            usage_status: budget,
            coherence_alerts: Vec::new(),
            indexing_in_progress: false,
            indexing_message: None,
            drop_suggestions: Vec::new(),
            next_actions: Vec::new(),
            result: serde_json::json!({"symbols": ["foo", "bar"]}),
        };

        let json_a = serde_json::to_string(&resp_a).unwrap();
        let json_b = serde_json::to_string(&resp_b).unwrap();

        // Both should start with identical usage_status prefix
        let prefix_a = &json_a[..json_a.find("\"result\"").unwrap()];
        let prefix_b = &json_b[..json_b.find("\"result\"").unwrap()];
        assert_eq!(
            prefix_a, prefix_b,
            "stable prefix should be byte-identical across different tool responses"
        );
    }

    #[test]
    fn tool_response_result_not_flattened() {
        use ministr_core::session::{UsageLevel, UsageStatus};

        let resp = ToolResponse {
            usage_status: UsageStatus {
                tokens_used: 0,
                tokens_remaining: 100_000,
                level: UsageLevel::Normal,
                utilization: 0.0,
            },
            coherence_alerts: Vec::new(),
            indexing_in_progress: false,
            indexing_message: None,
            drop_suggestions: Vec::new(),
            next_actions: Vec::new(),
            result: serde_json::json!({"items": [1]}),
        };

        let v = serde_json::to_value(&resp).unwrap();
        let obj = v.as_object().unwrap();

        // `result` should be a nested object, not flattened
        assert!(obj.contains_key("result"), "should have 'result' key");
        // usage_status is tracked internally but must not be serialized.
        assert!(
            !obj.contains_key("usage_status"),
            "usage_status must not be serialized to the agent"
        );
        // Flattened fields should NOT appear at top level
        assert!(
            !obj.contains_key("items"),
            "'items' should be inside 'result', not flattened"
        );
    }

    #[test]
    fn tool_response_skips_empty_optional_fields() {
        use ministr_core::session::{UsageLevel, UsageStatus};

        let resp = ToolResponse {
            usage_status: UsageStatus {
                tokens_used: 0,
                tokens_remaining: 100_000,
                level: UsageLevel::Normal,
                utilization: 0.0,
            },
            coherence_alerts: Vec::new(),
            indexing_in_progress: false,
            indexing_message: None,
            drop_suggestions: Vec::new(),
            next_actions: Vec::new(),
            result: serde_json::json!({}),
        };

        let v = serde_json::to_value(&resp).unwrap();
        let obj = v.as_object().unwrap();

        // skip_serializing_if should suppress empty/false fields
        assert!(
            !obj.contains_key("coherence_alerts"),
            "empty alerts should be skipped"
        );
        assert!(
            !obj.contains_key("indexing_in_progress"),
            "false indexing should be skipped"
        );
        assert!(
            !obj.contains_key("indexing_message"),
            "None message should be skipped"
        );
        assert!(
            !obj.contains_key("drop_suggestions"),
            "drop_suggestions must never be serialized"
        );
        assert!(
            !obj.contains_key("usage_status"),
            "usage_status must never be serialized"
        );

        // Only `result` remains — budget hints are gone entirely.
        assert_eq!(obj.len(), 1, "should only have result");
    }

    // --- Lazy tool registration (prune_tools) tests ---

    #[test]
    fn prune_tools_removes_web_tools_when_no_fetcher() {
        let mut server = setup_server_sync();
        assert!(server.web_fetcher.is_none());
        assert!(server.git_fetcher.is_none());

        // Before pruning: all tools present
        assert!(server.tool_router.has_route("ministr_fetch"));
        assert!(server.tool_router.has_route("ministr_refresh"));

        server.prune_tools(&[]);

        // Web tools removed
        assert!(!server.tool_router.has_route("ministr_fetch"));
        assert!(!server.tool_router.has_route("ministr_refresh"));
    }

    #[test]
    fn prune_tools_removes_git_tools_when_no_fetcher() {
        let mut server = setup_server_sync();
        assert!(server.git_fetcher.is_none());

        assert!(server.tool_router.has_route("ministr_clone"));
        server.prune_tools(&[]);
        assert!(!server.tool_router.has_route("ministr_clone"));
    }

    #[test]
    fn prune_tools_removes_task_when_no_web_or_git() {
        let mut server = setup_server_sync();
        assert!(server.tool_router.has_route("ministr_task"));
        server.prune_tools(&[]);
        assert!(!server.tool_router.has_route("ministr_task"));
    }

    #[test]
    fn prune_tools_keeps_core_tools() {
        let mut server = setup_server_sync();
        server.prune_tools(&[]);

        // Core context tools should always remain
        for name in &[
            "ministr_survey",
            "ministr_read",
            "ministr_extract",
            "ministr_related",
            "ministr_usage",
            "ministr_compress",
            "ministr_dropped",
            "ministr_toc",
        ] {
            assert!(
                server.tool_router.has_route(name),
                "core tool {name} should not be pruned"
            );
        }
    }

    #[test]
    fn prune_tools_removes_code_tools_for_non_code_dir() {
        let mut server = setup_server_sync();
        let tmp = tempfile::tempdir().unwrap();

        // Create a docs-only corpus (markdown files only)
        std::fs::write(tmp.path().join("README.md"), "# Hello").unwrap();
        std::fs::write(tmp.path().join("guide.txt"), "text").unwrap();

        server.prune_tools(&[tmp.path().to_path_buf()]);

        assert!(!server.tool_router.has_route("ministr_symbols"));
        assert!(!server.tool_router.has_route("ministr_definition"));
        assert!(!server.tool_router.has_route("ministr_references"));
        assert!(!server.tool_router.has_route("ministr_bridge"));
    }

    #[test]
    fn prune_tools_keeps_code_tools_for_code_dir() {
        let mut server = setup_server_sync();
        let tmp = tempfile::tempdir().unwrap();

        // Create a corpus with a Rust file
        std::fs::write(tmp.path().join("main.rs"), "fn main() {}").unwrap();

        server.prune_tools(&[tmp.path().to_path_buf()]);

        assert!(server.tool_router.has_route("ministr_symbols"));
        assert!(server.tool_router.has_route("ministr_definition"));
        assert!(server.tool_router.has_route("ministr_references"));
    }

    #[test]
    fn prune_tools_generates_custom_instructions() {
        let mut server = setup_server_sync();
        assert!(server.custom_instructions.is_none());

        server.prune_tools(&[]);

        let instructions = server.custom_instructions.as_ref().unwrap();
        // Instructions should mention core tools
        assert!(instructions.contains("ministr_survey"));
        assert!(instructions.contains("ministr_read"));
        assert!(instructions.contains("ministr_toc"));

        // Instructions should NOT mention pruned tools
        assert!(!instructions.contains("ministr_fetch"));
        assert!(!instructions.contains("ministr_clone"));
        assert!(!instructions.contains("ministr_task"));
    }

    #[test]
    fn prune_tools_instructions_used_in_get_info() {
        let mut server = setup_server_sync();
        server.prune_tools(&[]);

        let info = server.get_info();
        let instructions = info.instructions.unwrap();
        // Should use the custom instructions (without fetch/clone/task)
        assert!(!instructions.contains("ministr_fetch"));
        assert!(instructions.contains("ministr_survey"));
    }

    #[test]
    fn has_code_files_detects_rust() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("lib.rs"), "pub fn foo() {}").unwrap();
        assert!(has_code_files_in_dir(tmp.path()));
    }

    #[test]
    fn has_code_files_detects_nested_python() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("src");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("main.py"), "print('hello')").unwrap();
        assert!(has_code_files_in_dir(tmp.path()));
    }

    #[test]
    fn has_code_files_returns_false_for_docs_only() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), "# hello").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "notes").unwrap();
        assert!(!has_code_files_in_dir(tmp.path()));
    }

    #[test]
    fn has_code_files_skips_hidden_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let hidden = tmp.path().join(".hidden");
        std::fs::create_dir(&hidden).unwrap();
        std::fs::write(hidden.join("main.rs"), "fn main() {}").unwrap();
        // Only a hidden dir with code — should not be found
        assert!(!has_code_files_in_dir(tmp.path()));
    }

    #[test]
    fn has_code_files_skips_node_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let nm = tmp.path().join("node_modules");
        std::fs::create_dir(&nm).unwrap();
        std::fs::write(nm.join("index.js"), "module.exports = {}").unwrap();
        assert!(!has_code_files_in_dir(tmp.path()));
    }
}
