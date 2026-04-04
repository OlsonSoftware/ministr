//! MCP server implementation for iris.
//!
//! Implements the rmcp `ServerHandler` trait with `#[tool]` macro-based
//! tool registration. The server exposes iris tools (`iris_survey`,
//! `iris_read`, `iris_extract`, `iris_related`, `iris_evicted`,
//! `iris_budget`, `iris_compress`, `iris_toc`, `iris_fetch`,
//! `iris_refresh`, `iris_clone`, `iris_task`) over the MCP protocol.
//!
//! Every tool response includes a `budget_status` object with the current
//! token budget state. Survey and read responses are deduplicated against
//! the session shadow to avoid re-delivering content the agent already has.
//!
//! When the agent re-requests unchanged content, iris treats it as a
//! fault-based eviction signal — the agent's context window dropped the
//! content before our estimator predicted. The window estimate is corrected
//! and the content is re-delivered. The `iris_evicted` tool accepts explicit
//! eviction feedback from the agent.

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
    ListResourceTemplatesResult, ListResourcesResult, ListTasksResult, NumberOrString,
    PaginatedRequestParams, ProgressNotificationParam, PromptMessage, PromptMessageRole,
    RawResource, RawResourceTemplate, ReadResourceRequestParams, ReadResourceResult, Reference,
    Resource, ResourceContents, ResourceTemplate, ResourceUpdatedNotificationParam,
    ServerCapabilities, ServerInfo, SubscribeRequestParams, UnsubscribeRequestParams,
};
use rmcp::schemars;
use rmcp::service::{NotificationContext, Peer, RequestContext};
use rmcp::{prompt, prompt_handler, prompt_router, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug, info_span, warn};

use iris_core::analytics::Analytics;
use iris_core::code::package_graph::PackageGraph;
use iris_core::embedding::Embedder;
use iris_core::git::GitFetcher;
use iris_core::index::VectorIndex;
use iris_core::ingestion::{IngestionPipeline, IngestionProgress};
use iris_core::service::{
    CompressedItem, QueryError, QueryService, RelatedClaimResult, SurveyResult, SymbolRefResult,
};
use iris_core::session::eviction::EvictionCandidate;
use iris_core::session::prefetch::PrefetchEngine;
use iris_core::session::{
    AccessMode, BudgetConfig, BudgetStatus, CoherenceAlert, PressureLevel, SessionId,
    SessionRegistry,
};
use iris_core::storage::{SqliteStorage, Storage, SymbolFilter};
use iris_core::token::count_tokens;
use iris_core::types::{
    ContentId, RefKind, RelationType, Resolution, SectionId, parent_section_id,
};
use iris_core::web::fetcher::WebFetcher;

use crate::task::{McpTaskManager, task_to_cancel_result, task_to_get_result};

// ── iris MCP extension identifiers (SEP-1724) ──────────────────────────

/// Extension identifier for the iris budget protocol.
///
/// Advertises that the server provides per-response budget status snapshots,
/// proactive eviction recommendations at elevated pressure, and context-window
/// token accounting.
pub const EXT_BUDGET_PROTOCOL: &str = "dev.iris/budget-protocol";

/// Extension identifier for iris coherence notifications.
///
/// Advertises that the server emits `notifications/resources/updated` when
/// underlying corpus files change, enabling agents to refresh stale context.
pub const EXT_COHERENCE: &str = "dev.iris/coherence";

/// Extension identifier for iris multi-tier compression.
///
/// Advertises that the server supports compressing context at multiple
/// granularity levels: full text, atomic claims, and summaries.
pub const EXT_COMPRESSION: &str = "dev.iris/compression";

/// Build the `ExtensionCapabilities` map advertising all iris extensions.
fn iris_extension_capabilities() -> ExtensionCapabilities {
    let mut extensions = ExtensionCapabilities::new();
    extensions.insert(
        EXT_BUDGET_PROTOCOL.to_string(),
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
    pub budget_protocol: bool,
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
            budget_protocol: client.contains_key(EXT_BUDGET_PROTOCOL),
            coherence: client.contains_key(EXT_COHERENCE),
            compression: client.contains_key(EXT_COMPRESSION),
        }
    }
}

/// MCP server that exposes iris context-cache tools to LLM agents.
///
/// `IrisServer` adapts the [`QueryService`] to the MCP protocol.
/// It handles tool registration, request routing, and response formatting.
/// Tracks session state for deduplication and budget management.
#[derive(Clone)]
pub struct IrisServer {
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
    /// server-advertised iris extensions with the client's declared support.
    negotiated_extensions: Arc<Mutex<NegotiatedExtensions>>,
    /// Macro-generated tool router for dispatching tool calls.
    tool_router: ToolRouter<Self>,
    /// Macro-generated prompt router for dispatching prompt requests.
    prompt_router: PromptRouter<Self>,
}

/// Tool response wrapper that includes budget status alongside the result data.
///
/// Every tool response is serialized as a JSON object with a `data` field
/// containing the tool-specific result and a `budget_status` field with
/// the current token budget snapshot.
///
/// Fields are ordered for KV-cache prefix stability: stable metadata first,
/// varying tool-specific payload last. LLM providers cache KV tensors for
/// matching prompt prefixes, so consecutive tool calls with the same budget
/// status share a prefix up to the `result` field.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ToolResponse<T: Serialize + schemars::JsonSchema> {
    /// Current budget status snapshot (stable across consecutive calls).
    budget_status: BudgetStatus,
    /// Pending coherence alerts (present when underlying content has changed).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    coherence_alerts: Vec<CoherenceAlert>,
    /// True when background corpus ingestion is still running.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    indexing_in_progress: bool,
    /// Human-readable ingestion status message (e.g. "Checking 12/42 files").
    #[serde(skip_serializing_if = "Option::is_none")]
    indexing_message: Option<String>,
    /// Proactive eviction recommendations when budget pressure is elevated or critical.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    eviction_recommendations: Vec<EvictionCandidate>,
    /// The tool-specific result data (varying — placed last for prefix stability).
    result: T,
}

/// Wrapper for survey responses that includes both results and dedup metadata.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SurveyResponse {
    /// The survey results after deduplication.
    results: Vec<SurveyResult>,
    /// Number of results filtered out by deduplication.
    deduplicated_count: usize,
}

/// Wrapper for extract responses.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ExtractResponse {
    /// The extracted claims.
    claims: Vec<iris_core::service::ClaimResult>,
}

/// Response when a section has already been delivered and is unchanged.
///
/// Returned instead of full text to avoid wasting context tokens on
/// content the agent already has.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct AlreadyDeliveredResponse {
    /// The requested section ID.
    section_id: String,
    /// Always `"already_delivered"`.
    status: &'static str,
    /// Number of claims available for extraction.
    claims_available: usize,
}

/// Response from the `iris_evicted` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct EvictedResponse {
    /// Content IDs that were successfully removed.
    evicted: Vec<String>,
    /// Content IDs that were not found in the session.
    not_found: Vec<String>,
}

/// Parameters for the `iris_survey` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SurveyParams {
    /// Natural language query to search for relevant content.
    #[schemars(description = "Natural language query to search the corpus")]
    pub query: String,

    /// Maximum number of results to return (default: 10).
    #[schemars(description = "Maximum number of results to return")]
    pub top_k: Option<usize>,
}

/// Parameters for the `iris_read` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadParams {
    /// Hierarchical section ID (e.g. `docs/auth.md#error-handling`).
    #[schemars(description = "Section ID to read (e.g. 'docs/auth.md#error-handling')")]
    pub section_id: String,
}

/// Parameters for the `iris_evicted` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EvictedParams {
    /// Content IDs that the agent has dropped from its context window.
    #[schemars(description = "Content IDs that have been evicted from the agent's context window")]
    pub content_ids: Vec<String>,
}

/// Parameters for the `iris_extract` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExtractParams {
    /// Section ID to extract claims from.
    #[schemars(description = "Section ID to extract claims from")]
    pub section_id: String,

    /// Optional query to filter claims by relevance.
    #[schemars(description = "Optional query to filter claims by relevance")]
    pub query: Option<String>,
}

/// Parameters for the `iris_compress` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompressParams {
    /// Content IDs to generate compressed summaries for.
    #[schemars(
        description = "Content IDs (section IDs) to generate compressed summaries for eviction"
    )]
    pub content_ids: Vec<String>,
}

/// Response from the `iris_budget` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct BudgetResponse {
    /// Total context window budget in tokens.
    total_budget: usize,
    /// Estimated tokens currently used.
    estimated_used: usize,
    /// Estimated tokens remaining.
    estimated_remaining: usize,
    /// Current pressure level.
    pressure_level: String,
    /// Recommended eviction candidates (empty under normal pressure).
    eviction_candidates: Vec<EvictionCandidate>,
    /// Prefetch cache hit/miss metrics by strategy.
    prefetch_metrics: iris_core::session::PrefetchMetrics,
    /// Pending coherence alerts (present when underlying content has changed).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    coherence_alerts: Vec<CoherenceAlert>,
    /// Content IDs evicted via interactive elicitation (empty if elicitation
    /// was unavailable, declined, or pressure was normal).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    elicitation_evicted: Vec<String>,
}

/// Parameters for the `iris_related` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RelatedParams {
    /// Claim ID to find related claims for.
    #[schemars(description = "Claim ID to find related claims for")]
    pub claim_id: String,

    /// Optional filter for specific relation types.
    #[schemars(
        description = "Optional filter: 'references', 'contradicts', 'depends_on', 'updates'"
    )]
    pub relation_types: Option<Vec<String>>,
}

/// Response from the `iris_related` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct RelatedResponse {
    /// Related claims with relationship metadata.
    related: Vec<RelatedClaimResult>,
}

/// Response from the `iris_compress` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct CompressResponse {
    /// Compressed summaries for the requested content.
    summaries: Vec<CompressedItem>,
}

/// Parameters for the `iris_toc` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TocParams {
    /// Optional document ID filter — returns only sections from this document.
    #[schemars(
        description = "Optional document ID to filter the table of contents to a single document"
    )]
    pub document_id: Option<String>,

    /// Number of entries to skip (default: 0). Use with `limit` for pagination.
    #[schemars(description = "Number of entries to skip (default: 0)")]
    pub offset: Option<usize>,

    /// Maximum number of entries to return (default: 100).
    #[schemars(description = "Maximum number of entries to return (default: 100)")]
    pub limit: Option<usize>,
}

/// Parameters for the `iris_fetch` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FetchParams {
    /// URL to fetch content from.
    #[schemars(description = "URL to fetch content from (e.g. 'https://docs.example.com/')")]
    pub url: String,

    /// Crawl depth for following links (default: 0 = single page).
    /// Depth 1+ follows same-domain links up to this depth.
    #[schemars(description = "Crawl depth for following links (default: 0 = single page only)")]
    pub depth: Option<u32>,

    /// Maximum number of pages to fetch when crawling (default: 50).
    #[schemars(description = "Maximum number of pages to fetch when crawling (default: 50)")]
    pub max_pages: Option<usize>,

    /// Only fetch URLs whose path starts with this prefix (e.g. '/docs/').
    #[schemars(description = "Only fetch URLs whose path starts with this prefix (e.g. '/docs/')")]
    pub path_filter: Option<String>,
}

/// Response from the `iris_fetch` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct FetchResponse {
    /// Number of pages successfully fetched.
    pages_fetched: usize,
    /// Total sections indexed across all fetched pages.
    sections_indexed: usize,
    /// Total claims extracted across all fetched pages.
    claims_extracted: usize,
    /// Total tokens added to the corpus.
    tokens_added: usize,
    /// The fetch strategy that was used.
    strategy_used: String,
}

/// Parameters for the `iris_refresh` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RefreshParams {
    /// Optional URL or repo URL to refresh. If omitted, checks all cached web
    /// and git sources.
    #[schemars(
        description = "Optional URL (web) or repo URL (git) to check for staleness. If omitted, checks all cached sources."
    )]
    pub url: Option<String>,
}

/// Per-URL refresh detail for the response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct RefreshUrlDetailResponse {
    /// The URL that was checked.
    url: String,
    /// The outcome: "unchanged", "updated", or "failed: <reason>".
    status: String,
}

/// Per-repo refresh detail for the response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct RefreshGitDetailResponse {
    /// The repository URL that was checked.
    repo_url: String,
    /// The outcome: "unchanged", "updated", or "failed: <reason>".
    status: String,
}

/// Response from the `iris_refresh` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct RefreshResponse {
    /// Number of web URLs checked.
    urls_checked: usize,
    /// Number of web URLs that had new content and were re-indexed.
    urls_refreshed: usize,
    /// Number of web URLs that were unchanged.
    urls_unchanged: usize,
    /// Number of web URLs where the check failed.
    urls_failed: usize,
    /// Per-URL details for web sources.
    details: Vec<RefreshUrlDetailResponse>,
    /// Number of git repos checked.
    #[serde(skip_serializing_if = "is_zero")]
    git_repos_checked: usize,
    /// Number of git repos that had new commits and were re-indexed.
    #[serde(skip_serializing_if = "is_zero")]
    git_repos_refreshed: usize,
    /// Number of git repos that were unchanged.
    #[serde(skip_serializing_if = "is_zero")]
    git_repos_unchanged: usize,
    /// Number of git repos where the check failed.
    #[serde(skip_serializing_if = "is_zero")]
    git_repos_failed: usize,
    /// Per-repo details for git sources.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    git_details: Vec<RefreshGitDetailResponse>,
}

/// Helper for `skip_serializing_if` on zero counts.
///
/// Must take `&usize` (not `usize`) because serde's `skip_serializing_if`
/// passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(val: &usize) -> bool {
    *val == 0
}

/// Parameters for the `iris_clone` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CloneParams {
    /// Remote git repository URL (HTTPS or SSH).
    #[schemars(
        description = "Remote git repository URL to clone (e.g. 'https://github.com/owner/repo.git')"
    )]
    pub repo: String,

    /// Optional list of paths for sparse checkout — only these directories/files
    /// will be checked out and indexed.
    #[schemars(
        description = "Optional paths for sparse checkout (e.g. ['docs', 'src']). Omit for full checkout."
    )]
    pub paths: Option<Vec<String>>,

    /// Optional branch to clone (defaults to the repository's default branch).
    #[schemars(description = "Optional branch to clone (defaults to repository default)")]
    pub branch: Option<String>,
}

/// Response from the `iris_clone` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct CloneResponse {
    /// Number of files discovered in the clone checkout.
    files_discovered: usize,
    /// Number of files that were indexed (parsed and stored).
    files_indexed: usize,
    /// Total sections extracted across all indexed files.
    sections_extracted: usize,
    /// Time spent cloning the repository in milliseconds.
    clone_time_ms: u64,
    /// Time spent running the ingestion pipeline in milliseconds.
    index_time_ms: u64,
    /// Whether the clone was served from cache.
    from_cache: bool,
    /// Number of cross-references linked from local code to the cloned dependency.
    dependency_refs_linked: usize,
}

/// Parameters for the `iris_task` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskParams {
    /// Task ID to check status for.
    #[schemars(description = "Task ID to check status for")]
    pub task_id: String,
}

/// Output schema for the `iris_task` tool response.
///
/// Mirrors the MCP Task object shape for schema generation purposes.
/// The actual response is serialized from `rmcp::model::Task`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TaskStatusResponse {
    /// Unique task identifier.
    task_id: String,
    /// Current status: `working`, `completed`, `failed`, `cancelled`, or `input_required`.
    status: String,
    /// Human-readable status message.
    #[serde(skip_serializing_if = "Option::is_none")]
    status_message: Option<String>,
    /// ISO-8601 creation timestamp.
    created_at: String,
    /// ISO-8601 timestamp for the most recent status change.
    last_updated_at: String,
    /// Retention window in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<u64>,
    /// Suggested polling interval in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    poll_interval: Option<u64>,
}

/// Parameters for the `iris_symbols` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolsParams {
    /// Fuzzy name search (case-insensitive substring match).
    #[schemars(description = "Fuzzy symbol name search (case-insensitive substring match)")]
    pub query: Option<String>,

    /// Exact kind filter (e.g. "function", "struct", "trait", "enum", "impl").
    #[schemars(
        description = "Exact kind filter: 'function', 'struct', 'trait', 'enum', 'impl', 'const', 'static', 'type', 'mod'"
    )]
    pub kind: Option<String>,

    /// Module path prefix filter (e.g. "config" matches `config::sub`).
    #[schemars(description = "Module path prefix filter (e.g. 'config' matches config::sub)")]
    pub module: Option<String>,

    /// Exact visibility filter (e.g. "pub", "pub(crate)", "").
    #[schemars(description = "Exact visibility filter: 'pub', 'pub(crate)', 'pub(super)', ''")]
    pub visibility: Option<String>,

    /// Number of entries to skip (default: 0). Use with `limit` for pagination.
    #[schemars(description = "Number of entries to skip (default: 0)")]
    pub offset: Option<usize>,

    /// Maximum number of entries to return (default: 100).
    #[schemars(description = "Maximum number of entries to return (default: 100)")]
    pub limit: Option<usize>,
}

/// Parameters for the `iris_definition` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DefinitionParams {
    /// The symbol ID to look up.
    #[schemars(description = "Symbol ID to get the definition for (from iris_symbols results)")]
    pub symbol_id: String,
}

/// Parameters for the `iris_references` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReferencesParams {
    /// The symbol ID to find references for.
    #[schemars(description = "Symbol ID to find references for (from iris_symbols results)")]
    pub symbol_id: String,

    /// Optional reference kind filter.
    #[schemars(
        description = "Optional reference kind filter: 'calls', 'implements', 'imports', 'uses', 'bridge'"
    )]
    pub ref_kind: Option<String>,

    /// Number of entries to skip (default: 0). Use with `limit` for pagination.
    #[schemars(description = "Number of entries to skip (default: 0)")]
    pub offset: Option<usize>,

    /// Maximum number of entries to return (default: 100).
    #[schemars(description = "Maximum number of entries to return (default: 100)")]
    pub limit: Option<usize>,
}

/// Response from the `iris_symbols` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SymbolsResponse {
    /// Matching symbols.
    symbols: Vec<SymbolSummary>,
    /// Total number of matches (before pagination).
    total: usize,
    /// Offset of the first returned entry within the full list.
    offset: usize,
}

/// A compact symbol summary for search results.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SymbolSummary {
    /// Symbol ID (use with `iris_definition` / `iris_references`).
    id: String,
    /// Symbol name.
    name: String,
    /// Symbol kind.
    kind: String,
    /// Source file path.
    file: String,
    /// Start line number.
    line: u32,
    /// Declaration signature.
    signature: String,
    /// First line of doc comment, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_preview: Option<String>,
    /// Cyclomatic complexity (functions only).
    #[serde(skip_serializing_if = "Option::is_none")]
    complexity: Option<u32>,
    /// Transitive caller count (impact analysis).
    #[serde(skip_serializing_if = "Option::is_none")]
    caller_count: Option<u32>,
}

/// Response from the `iris_references` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ReferencesResponse {
    /// Cross-references for the symbol.
    references: Vec<SymbolRefResult>,
    /// Total number of references (before pagination).
    total: usize,
    /// Offset of the first returned entry within the full list.
    offset: usize,
}

/// Parameters for the `iris_bridge` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct BridgeParams {
    /// Optional search query to filter bridge links by binding key or symbol name.
    #[schemars(
        description = "Search query to filter by binding key or symbol name (case-insensitive substring match)"
    )]
    pub query: Option<String>,

    /// Optional bridge kind filter.
    #[schemars(
        description = "Filter by bridge kind: 'tauri_command', 'tauri_event', 'napi', 'wasm_bindgen', 'pyo3', 'http_route', 'ffi'"
    )]
    pub bridge_kind: Option<String>,

    /// Optional language filter.
    #[schemars(
        description = "Filter links involving this language (e.g. 'rust', 'typescript', 'javascript', 'python')"
    )]
    pub language: Option<String>,

    /// Optional file path filter.
    #[schemars(description = "Filter links where either endpoint is in this file path")]
    pub file_path: Option<String>,
}

/// Response from the `iris_bridge` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct BridgeResponse {
    /// Matched bridge links.
    links: Vec<BridgeLinkSummary>,
    /// Total number of links returned.
    total: usize,
}

/// A compact bridge link summary for search results.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct BridgeLinkSummary {
    /// Bridge mechanism kind (e.g. `"tauri_command"`).
    kind: String,
    /// Combined confidence score.
    confidence: f32,
    /// Export (definition) side.
    export: BridgeEndpointSummary,
    /// Import (call site) side.
    import: BridgeEndpointSummary,
}

/// One side of a bridge link in the response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct BridgeEndpointSummary {
    /// Binding key.
    binding_key: String,
    /// Symbol name.
    symbol_name: String,
    /// Source file path.
    file: String,
    /// Source line number.
    line: u32,
    /// Language.
    language: String,
}

/// Corpus-level statistics returned in the `iris_toc` response header.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct CorpusStatsHeader {
    /// Number of documents in the corpus.
    documents: usize,
    /// Total number of sections across all documents (before pagination).
    sections: usize,
    /// Number of claims across all sections.
    claims: usize,
    /// Offset of the first returned entry within the full list.
    offset: usize,
    /// Number of entries returned in this page.
    returned: usize,
    /// Ingestion state: `"pending"`, `"running"`, or `"complete"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    ingestion_status: Option<String>,
}

/// Response from the `iris_toc` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
struct TocResponse {
    /// Corpus-level statistics for quick orientation.
    corpus_stats: CorpusStatsHeader,
    /// Registered corpus roots with per-directory metadata and language stats.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    roots: Vec<iris_core::types::CorpusRoot>,
    /// Table of contents entries (metadata only, no text).
    entries: Vec<iris_core::types::TocEntry>,
}

// -- Union output types for tools that return different response shapes --

/// Output data for `iris_read`: either full section detail or an "already delivered" skip.
///
/// Used only for output schema generation via `schemars::JsonSchema`.
#[expect(dead_code)]
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
enum ReadOutputData {
    /// Section was already delivered and is unchanged.
    AlreadyDelivered(AlreadyDeliveredResponse),
    /// Full section detail (new or changed content).
    Detail(iris_core::service::SectionDetail),
}

/// Output data for `iris_fetch`.
///
/// Used for output schema generation via `schemars::JsonSchema`.
/// When invoked as an MCP Task, the result is delivered via `tasks/result`.
type FetchOutputData = ToolResponse<FetchResponse>;

/// Output data for `iris_clone`.
///
/// Used for output schema generation via `schemars::JsonSchema`.
/// When invoked as an MCP Task, the result is delivered via `tasks/result`.
type CloneOutputData = ToolResponse<CloneResponse>;

/// Generate the output schema `Arc<JsonObject>` for a tool response type.
///
/// Used in `#[tool(output_schema = ...)]` macro attributes to provide
/// static output schemas derived from the response types' `JsonSchema` impls.
///
/// Handles types with `#[serde(flatten)]` (which produce `allOf` schemas without
/// a root `type: "object"`) by injecting the required `type` field so the schema
/// passes rmcp's MCP spec validation.
fn tool_output_schema<T: schemars::JsonSchema + 'static>() -> std::sync::Arc<rmcp::model::JsonObject>
{
    // Try the standard rmcp path first (works for non-flattened types).
    if let Ok(schema) = rmcp::handler::server::tool::schema_for_output::<T>() {
        return schema;
    }
    // Flattened types produce allOf without a root "type" — generate the raw
    // schema and add `"type": "object"` so it conforms to the MCP spec.
    let mut schema = (*rmcp::handler::server::tool::schema_for_type::<T>()).clone();
    schema
        .entry("type")
        .or_insert_with(|| serde_json::Value::String("object".into()));
    std::sync::Arc::new(schema)
}

/// Maximum serialized response size in bytes before the guard injects a warning.
const MAX_RESPONSE_BYTES: usize = 100_000;

/// Maximum number of survey results to prefetch via agent intent prediction.
const MAX_INTENT_PREFETCH_SURVEY: usize = 5;

/// Serialize a value into a `CallToolResult` with structured content.
///
/// Sets both `structured_content` (JSON object) and `content` (text fallback)
/// for backward compatibility with clients that don't support structured output.
///
/// Includes a response size guard: if the serialized JSON exceeds
/// [`MAX_RESPONSE_BYTES`], a `_truncation_warning` is injected into the
/// response object advising the caller to use pagination parameters.
fn structured_result(value: &impl Serialize) -> Result<CallToolResult, McpError> {
    let v = serde_json::to_value(value)
        .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;

    // Response size guard: warn (but still deliver) when the payload is large.
    let v = apply_response_size_guard(v);

    Ok(CallToolResult::structured(v))
}

/// If the serialized JSON exceeds [`MAX_RESPONSE_BYTES`], inject a
/// `_truncation_warning` field advising the caller to paginate.
fn apply_response_size_guard(mut v: serde_json::Value) -> serde_json::Value {
    // Only measure object responses (all tool responses are objects).
    if let Some(obj) = v.as_object_mut() {
        // Fast byte-length estimate: serde_json::to_string length.
        let size = serde_json::to_string(obj).map_or(0, |s| s.len());
        if size > MAX_RESPONSE_BYTES {
            obj.insert(
                "_truncation_warning".to_string(),
                serde_json::json!({
                    "message": "Response exceeds size threshold. Use offset/limit parameters to paginate.",
                    "response_bytes": size,
                    "threshold_bytes": MAX_RESPONSE_BYTES,
                }),
            );
        }
    }
    v
}

#[tool_handler]
#[prompt_handler]
impl ServerHandler for IrisServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_subscribe()
                .enable_tasks()
                .enable_prompts()
                .enable_completions()
                .enable_extensions_with(iris_extension_capabilities())
                .build(),
        )
        .with_server_info(
            Implementation::new("iris", env!("CARGO_PKG_VERSION")).with_description(
                "A context cache controller for LLM agents — session tracking, \
                     predictive prefetching, budget management, and coherence.",
            ),
        )
        .with_instructions(
            "iris is a context cache controller for LLM agents. Use iris_toc to get \
             a structural overview of the indexed corpus, iris_survey to search for \
             relevant content, iris_read to retrieve full section text, iris_extract \
             to get atomic claims from a section, iris_related to follow dependency \
             chains between claims, iris_budget to check context budget status and \
             get eviction recommendations, iris_compress to generate compressed \
             summaries of content you want to evict, iris_evicted to signal when \
             content has been dropped from your context window, iris_fetch to \
             fetch web content by URL and add it to the corpus, iris_refresh \
             to check cached web sources for staleness and re-fetch changed content, \
             iris_clone to clone a git repository and index its content, \
             iris_task to poll background fetch/clone tasks (deprecated — prefer MCP tasks/get), \
             iris_symbols to search the code symbol index, \
             iris_definition to get the full source definition of a symbol, \
             and iris_references to find all references to a symbol.",
        )
    }

    // ── Extension Negotiation (SEP-1724) ─────────────────────────────

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        // Negotiate iris extensions by intersecting our capabilities with the
        // client's declared extension support.
        let negotiated = NegotiatedExtensions::negotiate(request.capabilities.extensions.as_ref());
        tracing::info!(
            budget = negotiated.budget_protocol,
            coherence = negotiated.coherence,
            compression = negotiated.compression,
            "extension negotiation complete"
        );
        *self.negotiated_extensions.lock().await = negotiated;

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
                             version metadata, and iris extension declarations"
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
                        uri: "iris://status".to_string(),
                        name: "iris status".to_string(),
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
                    uri_template: "iris://corpus/{path}".to_string(),
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
        } else if uri == "iris://status" {
            self.read_status_resource().await
        } else if let Some(path) = uri.strip_prefix("iris://corpus/") {
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
        // Only iris://status supports subscriptions for now.
        if uri != "iris://status" {
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
                // Complete iris://corpus/{path} resource URIs.
                if resource_ref.uri.starts_with("iris://corpus/") {
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
impl IrisServer {
    /// Search the corpus for sections relevant to your query.
    ///
    /// Returns ranked summaries with relevance scores across all resolution
    /// levels (document summaries, section text, atomic claims).
    /// Results that were already delivered in this session are filtered out.
    #[tool(
        name = "iris_survey",
        description = "Search the indexed corpus for sections relevant to a natural language query. Returns ranked summaries with relevance scores. Already-delivered content is filtered out.",
        output_schema = tool_output_schema::<ToolResponse<SurveyResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn survey(
        &self,
        Parameters(params): Parameters<SurveyParams>,
    ) -> Result<CallToolResult, McpError> {
        let top_k = params.top_k.unwrap_or(10);
        let span = info_span!("iris_survey", query_len = params.query.len(), top_k);

        async {
            debug!(query = %params.query, top_k, "iris_survey request");

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
                        deduplicated_count, "iris_survey success"
                    );

                    // Record delivered content in session and budget
                    let mut reg = self.registry.lock().await;
                    let entry = reg
                        .get_session_mut(&self.active_session_id)
                        .expect("active session exists");
                    let turn = entry.session.current_turn() + 1;
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
                    let budget_status = entry.budget.budget_status();

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
                            prefetch.record_tool_call("iris_survey", &params.query);
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

                    let response = self
                        .build_response(
                            SurveyResponse {
                                results,
                                deduplicated_count,
                            },
                            budget_status,
                        )
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "iris_survey failed");
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
        name = "iris_read",
        description = "Read the full text of a section by its hierarchical ID. Returns content with heading path and available claims count. Returns deltas for changed content and skips re-delivery of unchanged content.",
        output_schema = tool_output_schema::<ToolResponse<ReadOutputData>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn read(
        &self,
        Parameters(params): Parameters<ReadParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_read", section_id = %params.section_id);

        async {
            debug!(section_id = %params.section_id, "iris_read request");

            // Check prefetch cache for a warm hit
            let warm_detail = {
                let mut prefetch = self.prefetch.lock().await;
                prefetch.try_serve(&params.section_id).map(|entry| {
                    iris_core::service::SectionDetail {
                        section_id: entry.content_id.clone(),
                        heading_path: entry.heading_path.clone().unwrap_or_default(),
                        text: entry.text.clone(),
                        summary: entry.summary.clone(),
                        claims_available: entry.claims_available,
                    }
                })
            };

            let read_result = if let Some(detail) = warm_detail {
                debug!(section_id = %params.section_id, "iris_read: warm cache hit");
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
                    let entry = reg
                        .get_session_mut(&self.active_session_id)
                        .expect("active session exists");
                    let already_delivered = entry.session.is_delivered(&content_id);
                    let has_changed = entry.session.has_changed(&content_id, &current_hash);
                    let is_re_request = entry.session.is_re_request(&content_id, &current_hash);

                    // Case 2: Already delivered and unchanged — skip re-delivery
                    if already_delivered && !has_changed {
                        debug!(
                            section_id = %params.section_id,
                            "iris_read: already delivered, skipping re-delivery"
                        );

                        // If agent re-requests content it should still have,
                        // treat as a fault-based eviction signal.
                        if is_re_request {
                            entry.budget.force_evict(&params.section_id);
                        }

                        let budget_status = entry.budget.budget_status();
                        drop(reg);

                        let skip = AlreadyDeliveredResponse {
                            section_id: params.section_id.clone(),
                            status: "already_delivered",
                            claims_available: detail.claims_available,
                        };
                        let response = self.build_response(skip, budget_status).await;
                        return structured_result(&response);
                    }

                    // Case 1: New content (or changed) — deliver full text
                    drop(reg);
                    let budget_status = self
                        .record_section_delivery(&params.section_id, &detail.text, current_hash)
                        .await;
                    self.record_analytics_access(&params.section_id).await;
                    self.trigger_prefetch(&params.section_id).await;

                    let response = self.build_response(detail, budget_status).await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, section_id = %params.section_id, "iris_read failed");
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
        name = "iris_extract",
        description = "Extract atomic claims from a section, optionally filtered by relevance to a query.",
        output_schema = tool_output_schema::<ToolResponse<ExtractResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn extract(
        &self,
        Parameters(params): Parameters<ExtractParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!(
            "iris_extract",
            section_id = %params.section_id,
            has_query = params.query.is_some()
        );

        async {
            debug!(
                section_id = %params.section_id,
                query = params.query.as_deref().unwrap_or("<none>"),
                "iris_extract request"
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
                        "iris_extract success"
                    );

                    // Record each claim delivery in session and budget
                    let mut reg = self.registry.lock().await;
                    let entry = reg
                        .get_session_mut(&self.active_session_id)
                        .expect("active session exists");
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
                    let budget_status = entry.budget.budget_status();
                    drop(reg);

                    self.persist_session().await;

                    let response = self
                        .build_response(ExtractResponse { claims }, budget_status)
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, section_id = %params.section_id, "iris_extract failed");
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
        name = "iris_related",
        description = "Follow dependency chains between claims. Given a claim ID, returns related claims with relationship type (references, contradicts, depends_on, updates) and source section.",
        output_schema = tool_output_schema::<ToolResponse<RelatedResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn related(
        &self,
        Parameters(params): Parameters<RelatedParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_related", claim_id = %params.claim_id);

        async {
            debug!(claim_id = %params.claim_id, "iris_related request");

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
                        "iris_related success"
                    );

                    // Record each related claim delivery in session and budget
                    let mut reg = self.registry.lock().await;
                    let entry = reg
                        .get_session_mut(&self.active_session_id)
                        .expect("active session exists");
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
                    let budget_status = entry.budget.budget_status();
                    drop(reg);

                    self.persist_session().await;

                    let response = self
                        .build_response(RelatedResponse { related }, budget_status)
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, claim_id = %params.claim_id, "iris_related failed");
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
        name = "iris_evicted",
        description = "Signal that content IDs have been evicted from the agent's context window. Updates session tracking for accurate budget and deduplication.",
        output_schema = tool_output_schema::<ToolResponse<EvictedResponse>>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn evicted(
        &self,
        Parameters(params): Parameters<EvictedParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_evicted", count = params.content_ids.len());

        async {
            debug!(content_ids = ?params.content_ids, "iris_evicted request");

            let mut evicted = Vec::new();
            let mut not_found = Vec::new();

            let mut reg = self.registry.lock().await;
            let entry = reg
                .get_session_mut(&self.active_session_id)
                .expect("active session exists");

            for id_str in &params.content_ids {
                let content_id = ContentId(id_str.clone());
                if entry.session.remove_delivered(&content_id).is_some() {
                    entry.budget.force_evict(id_str);
                    evicted.push(id_str.clone());
                } else {
                    not_found.push(id_str.clone());
                }
            }

            let budget_status = entry.budget.budget_status();
            drop(reg);

            self.persist_session().await;

            debug!(
                evicted_count = evicted.len(),
                not_found_count = not_found.len(),
                "iris_evicted complete"
            );

            let response = self
                .build_response(EvictedResponse { evicted, not_found }, budget_status)
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
        name = "iris_budget",
        description = "Get the current context budget status: total budget, estimated usage, pressure level, and eviction recommendations. Call this to understand budget health.",
        output_schema = tool_output_schema::<BudgetResponse>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn budget(&self) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_budget");

        async {
            debug!("iris_budget request");

            let mut reg = self.registry.lock().await;
            let entry = reg
                .get_session_mut(&self.active_session_id)
                .expect("active session exists");
            let prefetch = self.prefetch.lock().await;

            let status = entry.budget.budget_status();
            let candidates =
                entry
                    .budget
                    .eviction_candidates(&entry.session, 5, Some(&entry.memory));
            let prefetch_metrics = prefetch.metrics();
            let alerts = entry.session.drain_alerts();

            drop(prefetch);
            drop(reg);

            let pressure_str = match status.pressure_level {
                PressureLevel::Normal => "normal",
                PressureLevel::Elevated => "elevated",
                PressureLevel::Critical => "critical",
            };

            debug!(
                pressure = pressure_str,
                used = status.tokens_used,
                remaining = status.tokens_remaining,
                candidate_count = candidates.len(),
                "iris_budget complete"
            );

            // When under pressure, try eliciting which sections to evict.
            let mut elicitation_evicted = Vec::new();
            if status.pressure_level != PressureLevel::Normal && !candidates.is_empty() {
                let candidate_list: String = candidates
                    .iter()
                    .map(|c| format!("  - {} ({} tokens)", c.content_id, c.tokens_recoverable))
                    .collect::<Vec<_>>()
                    .join("\n");
                let message = format!(
                    "Budget pressure is {pressure_str}. These sections are eviction candidates:\n\
                     {candidate_list}\n\n\
                     Which content_ids would you like to evict? \
                     (provide comma-separated content_ids, or decline to skip)"
                );

                let peer_guard = self.peer.lock().await;
                if let Some(peer) = peer_guard.clone() {
                    drop(peer_guard);
                    if let Some(choice) = crate::elicitation::try_elicit::<
                        crate::elicitation::EvictionChoice,
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

            let response = BudgetResponse {
                total_budget: status.tokens_used + status.tokens_remaining,
                estimated_used: status.tokens_used,
                estimated_remaining: status.tokens_remaining,
                pressure_level: pressure_str.to_string(),
                eviction_candidates: candidates,
                prefetch_metrics,
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
    /// When MCP sampling is available, uses LLM-assisted abstractive
    /// compression for 90%+ token reduction. Falls back to extractive
    /// TF-IDF summarization (60–80% reduction) when sampling is unavailable.
    #[tool(
        name = "iris_compress",
        description = "Generate compressed summaries for sections the agent wants to evict from context. Uses LLM-assisted abstractive compression (90%+ reduction) when sampling is available, falling back to extractive (60-80%). Returns summaries with original/compressed token counts and compression method.",
        output_schema = tool_output_schema::<ToolResponse<CompressResponse>>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn compress(
        &self,
        Parameters(params): Parameters<CompressParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_compress", count = params.content_ids.len());

        async {
            debug!(content_ids = ?params.content_ids, "iris_compress request");

            // Determine compression mode: try eliciting preference, then fall back.
            let result = {
                let peer_guard = self.peer.lock().await;
                if let Some(peer) = peer_guard.clone() {
                    drop(peer_guard);

                    // Elicit compression mode preference before spending sampling tokens.
                    let use_abstractive = {
                        let message = format!(
                            "Compressing {} section(s). Abstractive compression (LLM-assisted) \
                             achieves 90%+ reduction but uses sampling tokens. Extractive \
                             (TF-IDF) achieves 60-80% reduction with no extra cost.\n\n\
                             Set proceed=true and mode='abstractive' or 'extractive'.",
                            params.content_ids.len()
                        );
                        match crate::elicitation::try_elicit::<
                            crate::elicitation::CompressionConfirmation,
                        >(&peer, &message)
                        .await
                        {
                            Some(conf) if conf.proceed && conf.mode == "abstractive" => true,
                            Some(conf) if conf.proceed => false, // any other mode → extractive
                            Some(_) => false,                    // proceed=false → extractive
                            None => true, // no elicitation support → auto-detect (default: abstractive)
                        }
                    };

                    if use_abstractive {
                        let compressor = crate::sampling::SamplingCompressor::new(peer);
                        debug!("attempting abstractive compression via MCP sampling");
                        self.service
                            .compress_content_abstractive(&params.content_ids, &compressor)
                            .await
                    } else {
                        debug!("using extractive compression (agent preference)");
                        self.service.compress_content(&params.content_ids).await
                    }
                } else {
                    drop(peer_guard);
                    debug!("no peer available, using extractive compression");
                    self.service.compress_content(&params.content_ids).await
                }
            };

            match result {
                Ok(summaries) => {
                    debug!(summary_count = summaries.len(), "iris_compress success");

                    let reg = self.registry.lock().await;
                    let budget_status = reg
                        .get_session(&self.active_session_id)
                        .expect("active session exists")
                        .budget
                        .budget_status();
                    drop(reg);

                    let response = self
                        .build_response(CompressResponse { summaries }, budget_status)
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "iris_compress failed");
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
        name = "iris_toc",
        description = "Return a table of contents for the indexed corpus. Lists documents and sections with metadata (heading path, depth, claim count, token count) but no text content. Paginated: returns up to `limit` entries (default 100) starting at `offset` (default 0). Use `corpus_stats.sections` to know the total count. Optionally filter to a single document by ID.",
        output_schema = tool_output_schema::<ToolResponse<TocResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn toc(
        &self,
        Parameters(params): Parameters<TocParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_toc", document_id = ?params.document_id);

        async {
            debug!(document_id = ?params.document_id, "iris_toc request");

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
                        total_sections, total_claims, offset, returned, "iris_toc success"
                    );

                    let reg = self.registry.lock().await;
                    let budget_status = reg
                        .get_session(&self.active_session_id)
                        .expect("active session exists")
                        .budget
                        .budget_status();
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
                            budget_status,
                        )
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "iris_toc failed");
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
    /// searchable via `iris_survey`.
    #[tool(
        name = "iris_fetch",
        description = "Fetch web content by URL and add it to the indexed corpus. Tries llms.txt strategies first, then falls back to direct page fetch. Content is immediately searchable after fetching. Supports MCP Tasks for async execution.",
        output_schema = tool_output_schema::<FetchOutputData>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true),
        execution(task_support = "optional")
    )]
    async fn fetch(
        &self,
        Parameters(params): Parameters<FetchParams>,
        ct: CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_fetch", url = %params.url);

        async {
            debug!(
                url = %params.url,
                depth = params.depth,
                max_pages = params.max_pages,
                path_filter = ?params.path_filter,
                "iris_fetch request"
            );

            let Some(ref web_fetcher) = self.web_fetcher else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_fetch is not available: web fetcher not configured. \
                     Start iris with a data directory to enable web fetching."
                        .to_string(),
                )]));
            };

            let Some(ref embedder) = self.embedder else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_fetch is not available: embedder not configured.".to_string(),
                )]));
            };

            let Some(ref index) = self.index else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_fetch is not available: vector index not configured.".to_string(),
                )]));
            };

            let Some(ref storage) = self.storage else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_fetch is not available: storage not configured.".to_string(),
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
                        "iris_fetch success"
                    );

                    let reg = self.registry.lock().await;
                    let budget_status = reg
                        .get_session(&self.active_session_id)
                        .expect("active session exists")
                        .budget
                        .budget_status();
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
                            budget_status,
                        )
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, url = %params.url, "iris_fetch failed");
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
        name = "iris_refresh",
        description = "Check cached web and git sources for staleness. Re-fetches changed web content and re-clones stale git repos. If url is provided, checks only that source. If omitted, checks all cached sources. Reports what was updated.",
        output_schema = tool_output_schema::<ToolResponse<RefreshResponse>>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = true)
    )]
    async fn refresh(
        &self,
        Parameters(params): Parameters<RefreshParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_refresh", url = ?params.url);

        async {
            debug!(url = ?params.url, "iris_refresh request");

            let Some(ref embedder) = self.embedder else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_refresh is not available: embedder not configured.".to_string(),
                )]));
            };

            let Some(ref index) = self.index else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_refresh is not available: vector index not configured.".to_string(),
                )]));
            };

            let Some(ref storage) = self.storage else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_refresh is not available: storage not configured.".to_string(),
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
    /// embedded, and immediately searchable via `iris_survey`.
    #[tool(
        name = "iris_clone",
        description = "Clone a git repository and index its content. Supports sparse checkout via paths parameter. Cached clones are reused when the remote HEAD hasn't changed. Content is immediately searchable after cloning. Supports MCP Tasks for async execution.",
        output_schema = tool_output_schema::<CloneOutputData>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true),
        execution(task_support = "optional")
    )]
    async fn clone_repo(
        &self,
        Parameters(params): Parameters<CloneParams>,
        ct: CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_clone", repo = %params.repo);

        async {
            debug!(
                repo = %params.repo,
                paths = ?params.paths,
                branch = ?params.branch,
                "iris_clone request"
            );

            let Some(ref git_fetcher) = self.git_fetcher else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_clone is not available: git fetcher not configured. \
                     Start iris with a data directory to enable git cloning."
                        .to_string(),
                )]));
            };

            let Some(ref embedder) = self.embedder else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_clone is not available: embedder not configured.".to_string(),
                )]));
            };

            let Some(ref index) = self.index else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_clone is not available: vector index not configured.".to_string(),
                )]));
            };

            let Some(ref storage) = self.storage else {
                return Ok(CallToolResult::error(vec![Content::text(
                    "iris_clone is not available: storage not configured.".to_string(),
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
        name = "iris_task",
        description = "Poll a background task. Deprecated: prefer the MCP tasks/get protocol method. Returns task status (working, completed, failed, cancelled).",
        output_schema = tool_output_schema::<TaskStatusResponse>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn task_status(
        &self,
        Parameters(params): Parameters<TaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_task", task_id = %params.task_id);

        async {
            debug!(task_id = %params.task_id, "iris_task request");

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
    /// doc comment preview. Use the returned symbol IDs with `iris_definition`
    /// and `iris_references`.
    #[tool(
        name = "iris_symbols",
        description = "Search the code symbol index. Filter by name (fuzzy), kind, module, or visibility. Returns symbol IDs for use with iris_definition and iris_references. Paginated: returns up to `limit` entries (default 100) starting at `offset` (default 0). Use `total` to know the full count.",
        output_schema = tool_output_schema::<ToolResponse<SymbolsResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn symbols(
        &self,
        Parameters(params): Parameters<SymbolsParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_symbols", query = ?params.query, kind = ?params.kind);

        async {
            debug!(?params.query, ?params.kind, ?params.module, ?params.visibility, "iris_symbols request");

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

                    debug!(total, offset, returned = paginated.len(), "iris_symbols success");

                    let reg = self.registry.lock().await;
                    let budget_status = reg
                        .get_session(&self.active_session_id)
                        .expect("active session exists")
                        .budget
                        .budget_status();
                    drop(reg);

                    let response = self
                        .build_response(SymbolsResponse { symbols: paginated, total, offset }, budget_status)
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "iris_symbols failed");
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
        name = "iris_definition",
        description = "Get the full source definition of a code symbol by ID. Returns source code with surrounding context and module hierarchy.",
        output_schema = tool_output_schema::<ToolResponse<iris_core::service::SymbolDefinition>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn definition(
        &self,
        Parameters(params): Parameters<DefinitionParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_definition", symbol_id = %params.symbol_id);

        async {
            debug!(symbol_id = %params.symbol_id, "iris_definition request");

            match self.service.get_symbol_definition(&params.symbol_id).await {
                Ok(def) => {
                    let token_count = count_tokens(&def.source_context);
                    let mut reg = self.registry.lock().await;
                    let entry = reg
                        .get_session_mut(&self.active_session_id)
                        .expect("active session exists");
                    let _ = entry.budget.record_tokens(&params.symbol_id, token_count);
                    let budget_status = entry.budget.budget_status();
                    drop(reg);

                    debug!(symbol_id = %params.symbol_id, token_count, "iris_definition success");

                    let response = self.build_response(def, budget_status).await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, symbol_id = %params.symbol_id, "iris_definition failed");
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
        name = "iris_references",
        description = "Find all references to a code symbol: callers, implementors, importers, and cross-language bridge links. Optionally filter by reference kind. Paginated: returns up to `limit` entries (default 100) starting at `offset` (default 0). Use `total` to know the full count.",
        output_schema = tool_output_schema::<ToolResponse<ReferencesResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn references(
        &self,
        Parameters(params): Parameters<ReferencesParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_references", symbol_id = %params.symbol_id);

        async {
            debug!(symbol_id = %params.symbol_id, ref_kind = ?params.ref_kind, "iris_references request");

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

                    debug!(symbol_id = %params.symbol_id, total, offset, returned = paginated.len(), "iris_references success");

                    let reg = self.registry.lock().await;
                    let budget_status = reg
                        .get_session(&self.active_session_id)
                        .expect("active session exists")
                        .budget
                        .budget_status();
                    drop(reg);

                    let response = self
                        .build_response(
                            ReferencesResponse {
                                references: paginated,
                                total,
                                offset,
                            },
                            budget_status,
                        )
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, symbol_id = %params.symbol_id, "iris_references failed");
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
        name = "iris_bridge",
        description = "Search cross-language bridge links (e.g. Tauri commands, NAPI exports, PyO3 bindings). Filter by query, bridge_kind, language, or file_path. Returns matched export↔import pairs with confidence scores.",
        output_schema = tool_output_schema::<ToolResponse<BridgeResponse>>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn bridge(
        &self,
        Parameters(params): Parameters<BridgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let span = info_span!("iris_bridge", query = ?params.query, kind = ?params.bridge_kind);

        async {
            debug!(?params.query, ?params.bridge_kind, ?params.language, ?params.file_path, "iris_bridge request");

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

                    debug!(total, "iris_bridge success");

                    let reg = self.registry.lock().await;
                    let budget_status = reg
                        .get_session(&self.active_session_id)
                        .expect("active session exists")
                        .budget
                        .budget_status();
                    drop(reg);

                    let response = self
                        .build_response(BridgeResponse { links: summaries, total }, budget_status)
                        .await;
                    structured_result(&response)
                }
                Err(e) => {
                    warn!(error = %e, "iris_bridge failed");
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
impl IrisServer {
    /// Summarize the current session: sections read, budget state, and activity.
    ///
    /// Returns a structured overview of what has been delivered in this session,
    /// the current budget utilization and pressure level, and any pending
    /// coherence alerts or stale content.
    #[prompt(
        name = "session-summary",
        description = "Summarize sections read, budget state, and session activity"
    )]
    async fn session_summary(&self) -> Result<GetPromptResult, McpError> {
        let reg = self.registry.lock().await;
        let entry = reg
            .get_session(&self.active_session_id)
            .expect("active session exists");
        let status = entry.budget.budget_status();
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
            status.pressure_level,
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
                             call `iris_budget` to review.\n",
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
        let status = entry.budget.budget_status();

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
            "### Budget Headroom\n\
             {remaining} tokens remaining ({util:.1}% used, pressure: {pressure:?})\n\n",
            remaining = status.tokens_remaining,
            util = status.utilization * 100.0,
            pressure = status.pressure_level,
        );

        if matches!(status.pressure_level, PressureLevel::Critical) {
            recommendations.push_str(
                "**Warning:** Budget is critical. Consider evicting content \
                 with `iris_evicted` or compressing with `iris_compress` before reading more.\n\n",
            );
        }

        if prefetched.is_empty() {
            recommendations.push_str(
                "No pre-warmed sections available. Use `iris_survey` to search \
                 for relevant content or `iris_toc` for a structural overview.\n",
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
                // Create entry and replace its session with the restored one
                let entry =
                    registry.create_session(&sid, Some(budget_config), AccessMode::ReadWrite);
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
        // Set embedder/index if not already set (web_fetcher may have set them).
        if self.embedder.is_none() {
            self.embedder = Some(embedder);
        }
        if self.index.is_none() {
            self.index = Some(index);
        }
        self
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

    /// Record a section delivery in the session shadow and budget tracker.
    ///
    /// When the delivery causes window eviction, applies bookmark compression
    /// to evicted entries synchronously and spawns background extractive
    /// compression to upgrade bookmarks into summaries.
    ///
    /// Returns the budget status snapshot after recording.
    async fn record_section_delivery(
        &self,
        section_id: &str,
        text: &str,
        content_hash: String,
    ) -> BudgetStatus {
        let token_count = count_tokens(text);
        let content_id = ContentId(section_id.to_string());
        let mut reg = self.registry.lock().await;
        let entry = reg
            .get_session_mut(&self.active_session_id)
            .expect("active session exists");
        let turn = entry.session.current_turn() + 1;
        entry.session.record_delivery(
            &content_id,
            Resolution::Section,
            token_count,
            turn,
            content_hash,
        );
        let evicted_ids = entry.budget.record_tokens(section_id, token_count);

        let status = entry.budget.budget_status();
        drop(reg);

        // Phase 1: bookmark compression for evicted entries.
        // Look up heading paths, then re-acquire lock to apply bookmarks.
        if !evicted_ids.is_empty() {
            let mut heading_paths = Vec::with_capacity(evicted_ids.len());
            for evicted_id in &evicted_ids {
                heading_paths.push(self.service.section_heading_path(evicted_id).await);
            }
            let mut reg = self.registry.lock().await;
            if let Some(entry) = reg.get_session_mut(&self.active_session_id) {
                for (evicted_id, heading_path) in evicted_ids.iter().zip(&heading_paths) {
                    let evicted_cid = ContentId(evicted_id.clone());
                    entry.session.mask_to_bookmark(&evicted_cid, heading_path);
                }
            }
            drop(reg);
        }

        self.persist_session().await;

        // Phase 2: background extractive compression to upgrade bookmarks.
        if !evicted_ids.is_empty() {
            let service = self.service.clone();
            let registry = self.registry.clone();
            let session_id = self.active_session_id.clone();
            tokio::spawn(async move {
                if let Ok(compressed) = service.compress_content(&evicted_ids).await {
                    let mut reg = registry.lock().await;
                    if let Some(entry) = reg.get_session_mut(&session_id) {
                        for item in compressed {
                            let cid = ContentId(item.original_id.clone());
                            entry.session.set_compressed_summary(
                                &cid,
                                item.summary,
                                iris_core::session::CompressionTier::Extractive,
                                item.compressed_tokens,
                            );
                        }
                    }
                }
            });
        }

        status
    }

    /// Build a tool response with budget status and any pending coherence alerts.
    ///
    /// When budget pressure is elevated or critical, proactively includes
    /// eviction recommendations so the agent can free context tokens without
    /// having to call `iris_budget` explicitly.
    async fn build_response<T: Serialize + schemars::JsonSchema>(
        &self,
        data: T,
        budget_status: BudgetStatus,
    ) -> ToolResponse<T> {
        let mut reg = self.registry.lock().await;
        let entry = reg
            .get_session_mut(&self.active_session_id)
            .expect("active session exists");
        let alerts = entry.session.drain_alerts();

        // Compute eviction recommendations when under pressure
        let eviction_recommendations = if budget_status.pressure_level == PressureLevel::Normal {
            Vec::new()
        } else {
            entry
                .budget
                .eviction_candidates(&entry.session, 3, Some(&entry.memory))
        };
        drop(reg);

        let progress = &self.ingestion_progress;
        let indexing = progress.is_running();
        let indexing_message = if indexing {
            let done = progress.files_done();
            let total = progress.files_total();
            Some(format!("Checking {done}/{total} files"))
        } else {
            None
        };

        ToolResponse {
            budget_status,
            coherence_alerts: alerts,
            indexing_in_progress: indexing,
            indexing_message,
            eviction_recommendations,
            result: data,
        }
    }

    /// Execute the clone-and-ingest pipeline for `iris_clone`.
    ///
    /// Separated from the tool handler to satisfy the `too_many_lines` lint.
    #[allow(clippy::too_many_lines)]
    async fn clone_and_ingest(
        &self,
        params: &CloneParams,
        git_fetcher: &GitFetcher,
        embedder: &dyn Embedder,
        index: &dyn VectorIndex,
        storage: &SqliteStorage,
        ct: Option<&CancellationToken>,
    ) -> Result<CallToolResult, McpError> {
        // Phase 1: Clone the repository.
        let clone_start = std::time::Instant::now();
        let clone_result = match GitFetcher::clone(
            git_fetcher,
            &params.repo,
            params.paths.as_deref(),
            params.branch.as_deref(),
            ct,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, repo = %params.repo, "iris_clone: git clone failed");
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "clone failed: {e}"
                ))]));
            }
        };
        let clone_time_ms = elapsed_millis(clone_start);

        // Phase 1b: Register a corpus root for the clone.
        let root_id = iris_core::ingestion::compute_root_id(&clone_result.clone_dir);
        let clone_root = iris_core::types::CorpusRoot {
            id: root_id.clone(),
            path: clone_result.clone_dir.to_string_lossy().to_string(),
            kind: iris_core::types::RootKind::Git,
            display_name: Some(repo_display_name(&params.repo)),
            file_count: 0,
            language_stats: std::collections::HashMap::new(),
            repo_url: Some(params.repo.clone()),
            branch: clone_result.metadata.branch.clone(),
            commit_sha: Some(clone_result.metadata.commit_sha.clone()),
            clone_timestamp: Some(clone_result.metadata.clone_timestamp.clone()),
            sparse_paths: clone_result.metadata.checked_out_paths.clone(),
        };
        if let Err(e) = storage.upsert_corpus_root(&clone_root).await {
            warn!(error = %e, repo = %params.repo, "failed to register clone corpus root");
        }

        // Phase 2: Ingest the cloned content with embeddings (root-scoped).
        let ingest_start = std::time::Instant::now();
        let ingest_result = self
            .ingestion_pipeline
            .ingest_directory_with_embeddings_rooted(
                &clone_result.clone_dir,
                storage,
                embedder,
                index,
                Some(&root_id),
                ct,
            )
            .await;
        let index_time_ms = elapsed_millis(ingest_start);

        match ingest_result {
            Ok(stats) => {
                // Update the root's file count and language stats.
                let lang_stats = compute_language_stats(&clone_result.files);
                let updated_root = iris_core::types::CorpusRoot {
                    file_count: stats.files_indexed,
                    language_stats: lang_stats,
                    ..clone_root
                };
                if let Err(e) = storage.upsert_corpus_root(&updated_root).await {
                    warn!(error = %e, repo = %params.repo, "failed to update clone root stats");
                }

                // Record the clone in the git cache for staleness tracking.
                let git_cache_record = iris_core::storage::GitCacheRecord {
                    repo_url: params.repo.clone(),
                    branch: params.branch.clone(),
                    commit_sha: clone_result.metadata.commit_sha.clone(),
                    clone_timestamp: clone_result.metadata.clone_timestamp.clone(),
                    clone_dir: clone_result.clone_dir.to_string_lossy().to_string(),
                    checked_out_paths: clone_result.metadata.checked_out_paths.clone(),
                };
                if let Err(e) = storage.upsert_git_cache(&git_cache_record).await {
                    warn!(error = %e, repo = %params.repo, "failed to record git cache");
                }

                // Phase 3: Re-resolve local references against newly-indexed dependency.
                let dep_graph = PackageGraph::from_cloned_repo(&clone_result.clone_dir);
                let dep_dir_str = clone_result.clone_dir.to_string_lossy().to_string();
                let dependency_refs_linked = if dep_graph.is_empty() {
                    0
                } else {
                    // Gather local corpus root paths for file resolution.
                    let corpus_roots: Vec<std::path::PathBuf> = self
                        .service
                        .list_corpus_roots()
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|r| r.kind == iris_core::types::RootKind::Local)
                        .map(|r| std::path::PathBuf::from(r.path))
                        .collect();
                    match self
                        .ingestion_pipeline
                        .re_resolve_dependency_refs(
                            &dep_graph,
                            &[dep_dir_str],
                            &corpus_roots,
                            storage,
                        )
                        .await
                    {
                        Ok(count) => count,
                        Err(e) => {
                            warn!(
                                error = %e,
                                repo = %params.repo,
                                "dependency reference re-resolution failed"
                            );
                            0
                        }
                    }
                };

                debug!(
                    repo = %params.repo,
                    files_discovered = clone_result.files.len(),
                    files_indexed = stats.files_indexed,
                    sections = stats.total_sections,
                    dependency_refs_linked,
                    clone_ms = clone_time_ms,
                    index_ms = index_time_ms,
                    from_cache = clone_result.from_cache,
                    "iris_clone success"
                );

                let reg = self.registry.lock().await;
                let budget_status = reg
                    .get_session(&self.active_session_id)
                    .expect("active session exists")
                    .budget
                    .budget_status();
                drop(reg);

                let response = self
                    .build_response(
                        CloneResponse {
                            files_discovered: clone_result.files.len(),
                            files_indexed: stats.files_indexed,
                            sections_extracted: stats.total_sections,
                            clone_time_ms,
                            index_time_ms,
                            from_cache: clone_result.from_cache,
                            dependency_refs_linked,
                        },
                        budget_status,
                    )
                    .await;
                structured_result(&response)
            }
            Err(e) => {
                warn!(error = %e, repo = %params.repo, "iris_clone: ingestion failed");
                Ok(CallToolResult::error(vec![Content::text(format!(
                    "clone succeeded but ingestion failed: {e}"
                ))]))
            }
        }
    }

    /// Execute the refresh pipeline for both web and git sources concurrently.
    ///
    /// Separated from the tool handler to satisfy the `too_many_lines` lint.
    async fn refresh_all_sources(
        &self,
        params: &RefreshParams,
        storage: &Arc<SqliteStorage>,
        embedder: &dyn Embedder,
        index: &dyn VectorIndex,
    ) -> Result<CallToolResult, McpError> {
        // Run web and git refresh concurrently.
        // Run web and git refresh concurrently.
        let (web_result, git_result) = tokio::join!(
            self.refresh_web_sources(params, storage, embedder, index),
            self.refresh_git_sources(params.url.as_deref(), storage, embedder, index),
        );

        // Surface fatal web errors as tool errors.
        let (urls_checked, urls_refreshed, urls_unchanged, urls_failed, web_details) =
            match web_result {
                Ok(tuple) => tuple,
                Err(msg) => {
                    return Ok(CallToolResult::error(vec![Content::text(msg)]));
                }
            };
        let (git_checked, git_refreshed, git_unchanged, git_failed, git_details) = git_result;

        debug!(
            urls_checked,
            urls_refreshed,
            urls_unchanged,
            urls_failed,
            git_checked,
            git_refreshed,
            git_unchanged,
            git_failed,
            "iris_refresh success"
        );

        let reg = self.registry.lock().await;
        let budget_status = reg
            .get_session(&self.active_session_id)
            .expect("active session exists")
            .budget
            .budget_status();
        drop(reg);

        let response = self
            .build_response(
                RefreshResponse {
                    urls_checked,
                    urls_refreshed,
                    urls_unchanged,
                    urls_failed,
                    details: web_details,
                    git_repos_checked: git_checked,
                    git_repos_refreshed: git_refreshed,
                    git_repos_unchanged: git_unchanged,
                    git_repos_failed: git_failed,
                    git_details,
                },
                budget_status,
            )
            .await;
        structured_result(&response)
    }

    /// Refresh cached web URLs, returning aggregate counts and per-URL details.
    ///
    /// Returns `Err(message)` when the web refresh fails fatally (no URL filter),
    /// so the caller can surface the error as a `CallToolResult::error`.
    async fn refresh_web_sources(
        &self,
        params: &RefreshParams,
        storage: &Arc<SqliteStorage>,
        embedder: &dyn Embedder,
        index: &dyn VectorIndex,
    ) -> Result<(usize, usize, usize, usize, Vec<RefreshUrlDetailResponse>), String> {
        let Some(ref web_fetcher) = self.web_fetcher else {
            return Ok((0, 0, 0, 0, Vec::new()));
        };

        match web_fetcher
            .refresh_all(
                params.url.as_deref(),
                &self.ingestion_pipeline,
                storage.as_ref(),
                embedder,
                index,
            )
            .await
        {
            Ok(result) => {
                let details: Vec<RefreshUrlDetailResponse> = result
                    .details
                    .iter()
                    .map(|d| RefreshUrlDetailResponse {
                        url: d.url.clone(),
                        status: d.status.to_string(),
                    })
                    .collect();
                Ok((
                    result.urls_checked,
                    result.urls_refreshed,
                    result.urls_unchanged,
                    result.urls_failed,
                    details,
                ))
            }
            Err(e) => {
                if params.url.is_some() {
                    debug!(error = %e, "web refresh skipped (URL may be git)");
                    Ok((0, 0, 0, 0, Vec::new()))
                } else {
                    warn!(error = %e, "iris_refresh web failed");
                    Err(format!("refresh failed: {e}"))
                }
            }
        }
    }

    /// Refresh all cached git clones, or a single repo if `url_filter` matches.
    ///
    /// Phase 1: check staleness of all repos concurrently (bounded by
    /// `GitFetcherConfig::refresh_concurrency`).
    /// Phase 2: re-ingest stale repos sequentially (disk-bound, not worth parallelising).
    ///
    /// Returns `(checked, refreshed, unchanged, failed, details)`.
    #[allow(clippy::too_many_lines)]
    async fn refresh_git_sources(
        &self,
        url_filter: Option<&str>,
        storage: &Arc<SqliteStorage>,
        embedder: &dyn Embedder,
        index: &dyn VectorIndex,
    ) -> (usize, usize, usize, usize, Vec<RefreshGitDetailResponse>) {
        let Some(ref git_fetcher) = self.git_fetcher else {
            return (0, 0, 0, 0, Vec::new());
        };

        let records = if let Some(url) = url_filter {
            match storage.get_git_cache(url).await {
                Ok(Some(record)) => vec![record],
                Ok(None) => return (0, 0, 0, 0, Vec::new()),
                Err(e) => {
                    warn!(error = %e, "failed to query git cache");
                    return (0, 0, 0, 0, Vec::new());
                }
            }
        } else {
            match storage.list_git_cache().await {
                Ok(records) => records,
                Err(e) => {
                    warn!(error = %e, "failed to list git cache");
                    return (0, 0, 0, 0, Vec::new());
                }
            }
        };

        // Phase 1: concurrent staleness checks + re-clones for stale repos.
        let concurrency = git_fetcher.config().refresh_concurrency;
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let mut handles = Vec::with_capacity(records.len());

        for record in records {
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("locally-owned semaphore is never closed");
            let fetcher = Arc::clone(git_fetcher);
            let paths_opt: Option<Vec<String>> = if record.checked_out_paths.is_empty() {
                None
            } else {
                Some(record.checked_out_paths.clone())
            };

            handles.push(tokio::spawn(async move {
                let result = fetcher
                    .refresh(
                        &record.repo_url,
                        paths_opt.as_deref(),
                        record.branch.as_deref(),
                        &record.commit_sha,
                    )
                    .await;
                drop(permit);
                (record, paths_opt, result)
            }));
        }

        // Phase 2: collect results and re-ingest stale repos sequentially.
        let mut checked = 0;
        let mut refreshed = 0;
        let mut unchanged = 0;
        let mut failed = 0;
        let mut details = Vec::with_capacity(handles.len());

        for handle in handles {
            checked += 1;
            let Ok((record, paths_opt, refresh_result)) = handle.await else {
                failed += 1;
                warn!("git refresh task panicked");
                details.push(RefreshGitDetailResponse {
                    repo_url: String::from("<unknown>"),
                    status: "failed: task panicked".to_string(),
                });
                continue;
            };

            match refresh_result {
                Ok(None) => {
                    unchanged += 1;
                    details.push(RefreshGitDetailResponse {
                        repo_url: record.repo_url.clone(),
                        status: "unchanged".to_string(),
                    });
                }
                Ok(Some(clone_result)) => {
                    let params = CloneParams {
                        repo: record.repo_url.clone(),
                        paths: paths_opt,
                        branch: record.branch.clone(),
                    };
                    match self
                        .clone_and_ingest(
                            &params,
                            git_fetcher,
                            embedder,
                            index,
                            storage.as_ref(),
                            None,
                        )
                        .await
                    {
                        Ok(_) => {
                            refreshed += 1;
                            details.push(RefreshGitDetailResponse {
                                repo_url: record.repo_url.clone(),
                                status: format!(
                                    "updated: {} -> {}",
                                    &record.commit_sha[..7.min(record.commit_sha.len())],
                                    &clone_result.metadata.commit_sha
                                        [..7.min(clone_result.metadata.commit_sha.len())]
                                ),
                            });
                        }
                        Err(e) => {
                            failed += 1;
                            warn!(error = ?e, repo = %record.repo_url, "git refresh re-ingest failed");
                            details.push(RefreshGitDetailResponse {
                                repo_url: record.repo_url.clone(),
                                status: "failed: re-ingest error".to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    warn!(error = %e, repo = %record.repo_url, "git staleness check failed");
                    details.push(RefreshGitDetailResponse {
                        repo_url: record.repo_url.clone(),
                        status: format!("failed: {e}"),
                    });
                }
            }
        }

        (checked, refreshed, unchanged, failed, details)
    }

    /// Trigger all prefetch strategies after a read operation.
    ///
    /// Runs three strategies in sequence:
    /// 1. **Sequential** — next section + parent document summary
    /// 2. **Structural** — sibling sections from the same document
    /// 3. **Topical** — sections nearest to the running topic vector
    async fn trigger_prefetch(&self, section_id: &str) {
        if let Some(ref storage) = self.storage {
            let sid = SectionId(section_id.to_string());

            // --- Sequential prefetch ---
            let next_section = storage.get_next_section(&sid).await.unwrap_or(None);

            let claims_count = if let Some(ref next) = next_section {
                storage.list_claims(&next.id).await.map(|c| c.len()).ok()
            } else {
                None
            };

            let doc_record = storage.get_document_for_section(&sid).await.ok().flatten();
            let doc_summary = doc_record
                .as_ref()
                .and_then(|doc| doc.summary.as_ref().map(|s| (doc.id.0.clone(), s.clone())));

            let mut prefetch = self.prefetch.lock().await;
            prefetch.prefetch_sequential(next_section, doc_summary, claims_count);

            // --- Structural prefetch (sibling sections) ---
            if let Some(ref doc) = doc_record {
                if let Ok(all_sections) = storage.list_sections(&doc.id).await {
                    // Find current section's position to get nearby siblings
                    let current_pos = all_sections.iter().position(|s| s.id.0 == section_id);
                    if let Some(pos) = current_pos {
                        // Collect siblings: up to 2 before and 2 after, excluding current
                        let start = pos.saturating_sub(2);
                        let end = (pos + 3).min(all_sections.len());
                        let siblings: Vec<_> = all_sections[start..end]
                            .iter()
                            .filter(|s| s.id.0 != section_id)
                            .cloned()
                            .collect();

                        // Build claims counts for siblings
                        let mut claims_counts = std::collections::HashMap::new();
                        for s in &siblings {
                            if let Ok(claims) = storage.list_claims(&s.id).await {
                                claims_counts.insert(s.id.0.clone(), claims.len());
                            }
                        }

                        prefetch.prefetch_structural(siblings, &claims_counts);
                    }
                }
            }

            // --- Topical prefetch (similarity to running topic) ---
            // Embed the current section text and record it for topic tracking
            if let Ok(Some(section)) = storage.get_section(&sid).await {
                if let Ok(embeddings) = self.service.embedder().embed(&[&section.text]) {
                    if let Some(embedding) = embeddings.into_iter().next() {
                        prefetch.record_topic_access(embedding);
                    }
                }

                // Query index with topic vector for nearest un-cached sections
                if let Some(topic_vec) = prefetch.topic_vector() {
                    if let Ok(results) = self.service.index().search_knn(&topic_vec, 5) {
                        let mut candidates = Vec::new();
                        for result in results {
                            // Only prefetch section-level results
                            let vid = iris_core::types::VectorId::parse(&result.id);
                            if let Some(vid) = vid {
                                if vid.resolution() == iris_core::types::Resolution::Section {
                                    let cid = vid.content_id();
                                    // Skip if it's the current section
                                    if cid == section_id {
                                        continue;
                                    }
                                    let candidate_sid = SectionId(cid.to_string());
                                    if let Ok(Some(s)) = storage.get_section(&candidate_sid).await {
                                        candidates.push(s);
                                    }
                                }
                            }
                        }

                        let mut claims_counts = std::collections::HashMap::new();
                        for s in &candidates {
                            if let Ok(claims) = storage.list_claims(&s.id).await {
                                claims_counts.insert(s.id.0.clone(), claims.len());
                            }
                        }

                        prefetch.prefetch_topical(candidates, &claims_counts);
                    }
                }
            }

            // --- Cross-session prefetch (frequently co-accessed sections) ---
            if let Some(ref analytics) = self.analytics {
                let sid_ref = SectionId(section_id.to_string());
                if let Ok(co_accessed) = analytics
                    .co_accessed_with(&sid_ref, Analytics::default_co_access_limit())
                    .await
                {
                    let mut candidates = Vec::new();
                    for co in co_accessed {
                        // Skip if already in cache
                        if prefetch.cache().peek(&co.section_id.0).is_some() {
                            continue;
                        }
                        if let Ok(Some(s)) = storage.get_section(&co.section_id).await {
                            candidates.push(s);
                        }
                    }

                    if !candidates.is_empty() {
                        let mut claims_counts = std::collections::HashMap::new();
                        for s in &candidates {
                            if let Ok(claims) = storage.list_claims(&s.id).await {
                                claims_counts.insert(s.id.0.clone(), claims.len());
                            }
                        }
                        prefetch.prefetch_cross_session(candidates, &claims_counts);
                    }
                }
            }
        }
    }

    /// Record a section access in cross-session analytics.
    async fn record_analytics_access(&self, section_id: &str) {
        if let Some(ref analytics) = self.analytics {
            let sid = SectionId(section_id.to_string());
            if let Err(e) = analytics.record_access(&sid).await {
                warn!(error = %e, "failed to record analytics access");
            }
        }
    }

    /// Persist the current session state to storage, if persistence is enabled.
    /// Also flushes co-access patterns from the session trajectory.
    async fn persist_session(&self) {
        if let Some(ref storage) = self.storage {
            let reg = self.registry.lock().await;
            let Some(entry) = reg.get_session(&self.active_session_id) else {
                return;
            };
            if let Err(e) = storage.save_session(&entry.session).await {
                warn!(error = %e, "failed to persist session");
            }
            // Flush co-access patterns from trajectory
            if let Some(ref analytics) = self.analytics {
                let trajectory = entry.session.trajectory();
                let section_ids: Vec<SectionId> = trajectory
                    .iter()
                    .map(|cid| SectionId(cid.0.clone()))
                    .collect();
                drop(reg);
                if let Err(e) = analytics.record_co_accesses(&section_ids).await {
                    warn!(error = %e, "failed to record co-access patterns");
                }
            }
        }
    }

    // ── Completion helpers ────────────────────────────────────────────

    /// Complete section IDs by fuzzy-matching the partial value.
    async fn complete_section_ids(&self, partial: &str) -> Vec<String> {
        let storage = self.service.storage();
        let documents = storage.list_documents().await.unwrap_or_default();
        let lower = partial.to_lowercase();
        let mut results = Vec::new();
        for doc in &documents {
            let sections = storage.list_sections(&doc.id).await.unwrap_or_default();
            for section in sections {
                if section.id.0.to_lowercase().contains(&lower) {
                    results.push(section.id.0);
                    if results.len() >= CompletionInfo::MAX_VALUES {
                        return results;
                    }
                }
            }
        }
        results
    }

    /// Complete corpus document paths for `iris://corpus/{path}` resources.
    async fn complete_corpus_paths(&self, partial: &str) -> Vec<String> {
        let storage = self.service.storage();
        let documents = storage.list_documents().await.unwrap_or_default();
        let lower = partial.to_lowercase();
        documents
            .into_iter()
            .filter(|d| d.source_path.to_lowercase().contains(&lower))
            .take(CompletionInfo::MAX_VALUES)
            .map(|d| d.source_path)
            .collect()
    }

    /// Build the `mcp://server-card.json` server card (SEP-1649).
    ///
    /// Returns a structured metadata document describing this server's identity,
    /// protocol version, capabilities, extensions, and full tool catalog. Clients
    /// can read this resource to discover server features without completing the
    /// initialization handshake.
    fn build_server_card(&self) -> serde_json::Value {
        let info = self.get_info();

        // Build the tool catalog from the macro-generated tool router.
        let tools: Vec<serde_json::Value> = self
            .tool_router
            .list_all()
            .into_iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                })
            })
            .collect();

        // Build capabilities object mirroring the initialization result.
        let mut capabilities = serde_json::json!({});
        if info.capabilities.tools.is_some() {
            capabilities["tools"] = serde_json::json!({ "listChanged": true });
        }
        if let Some(ref res) = info.capabilities.resources {
            capabilities["resources"] = serde_json::json!({
                "subscribe": res.subscribe.unwrap_or(false),
                "listChanged": res.list_changed.unwrap_or(false),
            });
        }
        if info.capabilities.prompts.is_some() {
            capabilities["prompts"] = serde_json::json!({ "listChanged": true });
        }
        if info.capabilities.tasks.is_some() {
            capabilities["tasks"] = serde_json::json!({});
        }
        if info.capabilities.completions.is_some() {
            capabilities["completions"] = serde_json::json!({});
        }
        if let Some(ref extensions) = info.capabilities.extensions {
            capabilities["extensions"] = serde_json::to_value(extensions).unwrap_or_default();
        }

        serde_json::json!({
            "$schema": "https://static.modelcontextprotocol.io/schemas/mcp-server-card/v1.json",
            "version": "1.0",
            "protocolVersion": info.protocol_version.to_string(),
            "serverInfo": {
                "name": info.server_info.name,
                "version": info.server_info.version,
                "description": info.server_info.description,
            },
            "capabilities": capabilities,
            "tools": tools,
        })
    }

    /// Read the `mcp://server-card.json` resource content.
    fn read_server_card_resource(&self) -> ReadResourceResult {
        let card = self.build_server_card();
        let text = serde_json::to_string_pretty(&card).unwrap_or_default();
        ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
            meta: None,
            uri: "mcp://server-card.json".to_string(),
            mime_type: Some("application/json".to_string()),
            text,
        }])
    }

    /// Build the `iris://status` resource content.
    async fn read_status_resource(&self) -> Result<ReadResourceResult, McpError> {
        let index = self.service.index();
        let reg = self.registry.lock().await;
        let entry = reg
            .get_session(&self.active_session_id)
            .expect("active session exists");

        let analytics_stats = if let Some(ref analytics) = self.analytics {
            analytics.corpus_stats().await.ok()
        } else {
            None
        };

        let mut status = serde_json::json!({
            "index": {
                "vector_count": index.len(),
                "dimension": index.dimension(),
            },
            "session": {
                "id": entry.session.id.to_string(),
                "delivered_count": entry.session.delivered_count(),
                "federation": {
                    "total_sessions": reg.session_count(),
                    "session_ids": reg.session_ids(),
                },
            },
            "budget": entry.budget.budget_status(),
        });

        if let Some(stats) = analytics_stats {
            status["analytics"] = serde_json::json!({
                "total_accesses": stats.total_accesses,
                "unique_sections_accessed": stats.unique_sections_accessed,
                "co_access_pairs": stats.co_access_pairs,
            });
        }

        let text = serde_json::to_string_pretty(&status).unwrap_or_default();
        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                meta: None,
                uri: "iris://status".to_string(),
                mime_type: Some("application/json".to_string()),
                text,
            },
        ]))
    }

    /// Build the `iris://corpus/{path}` resource content.
    async fn read_corpus_resource(&self, path: &str) -> Result<ReadResourceResult, McpError> {
        let storage = self.service.storage();
        let documents = storage.list_documents().await.map_err(|e| {
            McpError::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                format!("storage error: {e}"),
                None,
            )
        })?;

        let doc = documents
            .iter()
            .find(|d| d.source_path == path)
            .ok_or_else(|| {
                McpError::new(
                    rmcp::model::ErrorCode::INVALID_PARAMS,
                    format!("document not found for path: {path}"),
                    None,
                )
            })?;

        let sections = storage.list_sections(&doc.id).await.map_err(|e| {
            McpError::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                format!("storage error: {e}"),
                None,
            )
        })?;

        let metadata = serde_json::json!({
            "id": doc.id.0,
            "title": doc.title,
            "source_path": doc.source_path,
            "summary": doc.summary,
            "section_count": sections.len(),
        });

        let uri = format!("iris://corpus/{path}");
        let text = serde_json::to_string_pretty(&metadata).unwrap_or_default();
        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                meta: None,
                uri,
                mime_type: Some("application/json".to_string()),
                text,
            },
        ]))
    }
}

/// Compute a SHA-256 hex digest of content for change detection.
fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Parse a resolution string back to the enum.
fn parse_resolution(s: &str) -> Resolution {
    match s {
        "summary" => Resolution::Summary,
        "claim" => Resolution::Claim,
        _ => Resolution::Section,
    }
}

/// Convert elapsed duration to milliseconds, saturating at `u64::MAX`.
fn elapsed_millis(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Extract a human-readable display name from a repository URL.
///
/// Strips the host prefix and `.git` suffix to produce e.g. `"owner/repo"`.
fn repo_display_name(repo_url: &str) -> String {
    let name = repo_url
        .rsplit_once("://")
        .map_or(repo_url, |(_, rest)| rest);
    let name = name.strip_prefix("github.com/").unwrap_or(name);
    let name = name.strip_prefix("gitlab.com/").unwrap_or(name);
    name.strip_suffix(".git").unwrap_or(name).to_string()
}

/// Compute language statistics from a list of file paths.
fn compute_language_stats(
    files: &[std::path::PathBuf],
) -> std::collections::HashMap<String, usize> {
    let mut stats = std::collections::HashMap::new();
    for file in files {
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = match ext {
            "rs" => "rust",
            "py" => "python",
            "ts" | "tsx" => "typescript",
            "js" | "jsx" => "javascript",
            "go" => "go",
            "rb" => "ruby",
            "java" => "java",
            "c" | "h" => "c",
            "cpp" | "cxx" | "cc" | "hpp" => "cpp",
            "toml" => "toml",
            "yaml" | "yml" => "yaml",
            "json" => "json",
            "md" => "markdown",
            other if !other.is_empty() => other,
            _ => continue,
        };
        *stats.entry(lang.to_string()).or_insert(0) += 1;
    }
    stats
}

/// Generate a simple UUID v4-style session ID.
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("sess-{}-{}", now.as_secs(), now.subsec_nanos())
}

/// Format a [`QueryError`] into a user-friendly error message for MCP tool responses.
///
/// Produces structured messages that help the agent understand what went wrong
/// and how to recover, rather than exposing raw internal error strings.
fn format_query_error(err: &QueryError) -> String {
    match err {
        QueryError::SectionNotFound { id } => {
            format!(
                "Section not found: '{id}'. Check the section ID format \
                 (e.g. 'docs/auth.md#tokens') and use iris_survey to discover valid IDs."
            )
        }
        QueryError::Index(index_err) => {
            format!(
                "Search index error: {index_err}. The index may need to be rebuilt. \
                 Try a different query or check server logs for details."
            )
        }
        QueryError::Storage(storage_err) => {
            format!(
                "Storage error: {storage_err}. The corpus database may be unavailable. \
                 Check server logs for details."
            )
        }
        QueryError::ClaimNotFound { id } => {
            format!(
                "Claim not found: '{id}'. Use iris_extract to discover valid claim IDs \
                 within a section."
            )
        }
        QueryError::SymbolNotFound { id } => {
            format!("Symbol not found: '{id}'. Use iris_symbols to search for valid symbol IDs.")
        }
    }
}

/// Well-known progress token for iris ingestion notifications.
const INGESTION_PROGRESS_TOKEN: &str = "iris/ingestion";

/// Poll `IngestionProgress` and push MCP `notifications/progress` to the client.
///
/// Runs until ingestion completes or the peer channel closes. Polls every 2
/// seconds to avoid flooding the client with messages.
async fn run_ingestion_progress_notifier(
    progress: Arc<IngestionProgress>,
    peer_lock: Arc<Mutex<Option<Peer<RoleServer>>>>,
) {
    use rmcp::model::ProgressToken;

    // Wait briefly for ingestion to start (it may not have begun yet).
    let mut wait_count = 0;
    while progress.status() == 0 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        wait_count += 1;
        // Give up after 30 seconds if ingestion never starts.
        if wait_count > 60 {
            tracing::debug!("ingestion never started, progress notifier exiting");
            return;
        }
    }

    let token = ProgressToken(NumberOrString::String(INGESTION_PROGRESS_TOKEN.into()));
    let mut last_done = 0;

    loop {
        if !progress.is_running() {
            // Send one final notification with done == total.
            let total = progress.files_total();
            let peer = peer_lock.lock().await;
            if let Some(ref p) = *peer {
                #[allow(clippy::cast_precision_loss)]
                let _ = p
                    .notify_progress(ProgressNotificationParam {
                        progress_token: token.clone(),
                        progress: total as f64,
                        total: Some(total as f64),
                        message: Some("Corpus ready".to_string()),
                    })
                    .await;
            }
            tracing::info!("ingestion complete, progress notifier exiting");
            break;
        }

        let done = progress.files_done();
        let total = progress.files_total();

        // Only send if progress actually changed.
        if done != last_done {
            last_done = done;
            let peer = peer_lock.lock().await;
            if let Some(ref p) = *peer {
                #[allow(clippy::cast_precision_loss)]
                if p.notify_progress(ProgressNotificationParam {
                    progress_token: token.clone(),
                    progress: done as f64,
                    total: Some(total as f64),
                    message: Some(format!("Checking {done}/{total} files")),
                })
                .await
                .is_err()
                {
                    tracing::debug!("peer channel closed, progress notifier exiting");
                    break;
                }
            } else {
                tracing::debug!("no peer available, progress notifier exiting");
                break;
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Listen for coherence change events and push `notifications/resources/updated`
/// to the MCP client for any subscribed resource URIs.
///
/// Currently only `iris://status` supports subscriptions — any coherence event
/// (file change → section invalidation) triggers an update notification for it.
/// Runs until the coherence sender is dropped or the peer channel closes.
async fn run_subscription_notifier(
    mut coherence_rx: tokio::sync::mpsc::UnboundedReceiver<Vec<String>>,
    peer_lock: Arc<Mutex<Option<Peer<RoleServer>>>>,
    subscriptions: Arc<Mutex<HashSet<String>>>,
) {
    tracing::info!("resource subscription notifier started");

    while let Some(affected_sections) = coherence_rx.recv().await {
        let subs = subscriptions.lock().await;
        if subs.is_empty() {
            continue;
        }

        // Any coherence event affects iris://status (it includes session state).
        if subs.contains("iris://status") {
            let peer = peer_lock.lock().await;
            if let Some(ref p) = *peer {
                tracing::debug!(
                    affected_sections = affected_sections.len(),
                    "pushing resource update notification for iris://status"
                );
                if p.notify_resource_updated(ResourceUpdatedNotificationParam {
                    uri: "iris://status".to_string(),
                })
                .await
                .is_err()
                {
                    tracing::debug!("peer channel closed, subscription notifier exiting");
                    break;
                }
            } else {
                tracing::debug!("no peer available, subscription notifier exiting");
                break;
            }
        }
    }

    tracing::info!("resource subscription notifier stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use iris_core::embedding::Embedder;
    use iris_core::error::IndexError;
    use iris_core::index::{HnswIndex, VectorIndex};
    use iris_core::storage::{SqliteStorage, Storage};
    use iris_core::types::{Claim, ClaimId, ContentId, DocumentTree, Section, SectionId};
    use rmcp::model::ProtocolVersion;

    /// Extract the text string from the first Content item.
    fn extract_text(content: &[Content]) -> &str {
        content[0]
            .raw
            .as_text()
            .expect("expected text content")
            .text
            .as_str()
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

    async fn setup_server() -> IrisServer {
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
        IrisServer::new(service)
    }

    /// Wrap an `IrisServer` into an in-process MCP client for testing.
    ///
    /// Returns both client and server handle — the server handle must stay
    /// alive or the server shuts down and the client hangs.
    type TestClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
    type TestServerHandle = rmcp::service::RunningService<RoleServer, IrisServer>;
    async fn wrap_test_client(server: IrisServer) -> (TestClient, TestServerHandle) {
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
    fn setup_server_sync() -> IrisServer {
        let dim = 8;
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
        let index: Arc<dyn iris_core::index::VectorIndex> =
            Arc::new(HnswIndex::new(dim, 100).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();
        let service = Arc::new(QueryService::new(storage, embedder, index));
        IrisServer::new(service)
    }

    // --- ServerInfo tests ---

    #[test]
    fn server_info_has_correct_name_and_version() {
        let server = setup_server_sync();
        let info = server.get_info();

        assert_eq!(info.server_info.name, "iris");
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
        assert!(instructions.contains("iris_survey"));
        assert!(instructions.contains("iris_read"));
        assert!(instructions.contains("iris_extract"));
    }

    #[test]
    fn server_info_uses_latest_protocol() {
        let server = setup_server_sync();
        let info = server.get_info();

        assert_eq!(info.protocol_version, ProtocolVersion::LATEST);
    }

    // --- Extension declaration tests ---

    #[test]
    fn server_info_advertises_iris_extensions() {
        let server = setup_server_sync();
        let info = server.get_info();

        let extensions = info
            .capabilities
            .extensions
            .as_ref()
            .expect("extensions capability should be set");

        assert!(
            extensions.contains_key(EXT_BUDGET_PROTOCOL),
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
    fn extension_budget_protocol_has_version() {
        let server = setup_server_sync();
        let info = server.get_info();
        let extensions = info.capabilities.extensions.as_ref().unwrap();
        let budget = &extensions[EXT_BUDGET_PROTOCOL];
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
        assert!(!result.budget_protocol);
        assert!(!result.coherence);
        assert!(!result.compression);
    }

    #[test]
    fn negotiation_with_empty_client_extensions_yields_all_false() {
        let empty = ExtensionCapabilities::new();
        let result = NegotiatedExtensions::negotiate(Some(&empty));
        assert!(!result.budget_protocol);
        assert!(!result.coherence);
        assert!(!result.compression);
    }

    #[test]
    fn negotiation_with_matching_client_extensions() {
        let mut client_ext = ExtensionCapabilities::new();
        client_ext.insert(EXT_BUDGET_PROTOCOL.to_string(), serde_json::Map::new());
        client_ext.insert(EXT_COHERENCE.to_string(), serde_json::Map::new());
        client_ext.insert(EXT_COMPRESSION.to_string(), serde_json::Map::new());

        let result = NegotiatedExtensions::negotiate(Some(&client_ext));
        assert!(result.budget_protocol);
        assert!(result.coherence);
        assert!(result.compression);
    }

    #[test]
    fn negotiation_partial_match() {
        let mut client_ext = ExtensionCapabilities::new();
        client_ext.insert(EXT_BUDGET_PROTOCOL.to_string(), serde_json::Map::new());
        // Client does NOT advertise coherence or compression.

        let result = NegotiatedExtensions::negotiate(Some(&client_ext));
        assert!(result.budget_protocol);
        assert!(!result.coherence);
        assert!(!result.compression);
    }

    #[test]
    fn negotiation_ignores_unknown_client_extensions() {
        let mut client_ext = ExtensionCapabilities::new();
        client_ext.insert("io.example/unknown".to_string(), serde_json::Map::new());

        let result = NegotiatedExtensions::negotiate(Some(&client_ext));
        assert!(!result.budget_protocol);
        assert!(!result.coherence);
        assert!(!result.compression);
    }

    // --- Budget status tests ---

    #[tokio::test]
    async fn survey_response_includes_budget_status() {
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

        assert!(
            parsed["budget_status"].is_object(),
            "response should include budget_status"
        );
        assert!(
            parsed["budget_status"]["tokens_used"].is_number(),
            "budget_status should have tokens_used"
        );
        assert!(
            parsed["budget_status"]["tokens_remaining"].is_number(),
            "budget_status should have tokens_remaining"
        );
        assert!(
            parsed["budget_status"]["pressure_level"].is_string(),
            "budget_status should have pressure_level"
        );
        assert!(
            parsed["budget_status"]["utilization"].is_number(),
            "budget_status should have utilization"
        );
    }

    #[tokio::test]
    async fn read_response_includes_budget_status() {
        let server = setup_server().await;
        let params = ReadParams {
            section_id: "docs/auth.md#tokens".to_string(),
        };
        let result = server.read(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert!(parsed["budget_status"].is_object());
        assert!(parsed["budget_status"]["tokens_used"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn extract_response_includes_budget_status() {
        let server = setup_server().await;
        let params = ExtractParams {
            section_id: "docs/auth.md#tokens".to_string(),
            query: None,
        };
        let result = server.extract(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert!(parsed["budget_status"].is_object());
    }

    #[tokio::test]
    async fn budget_accumulates_across_tool_calls() {
        let server = setup_server().await;

        // First call — read a section
        let result1 = server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();
        let text1 = extract_text(&result1.content);
        let parsed1: serde_json::Value = serde_json::from_str(text1).unwrap();
        let used_after_read = parsed1["budget_status"]["tokens_used"].as_u64().unwrap();
        assert!(used_after_read > 0, "should track tokens after read");

        // Second call — extract claims
        let result2 = server
            .extract(Parameters(ExtractParams {
                section_id: "docs/auth.md#tokens".to_string(),
                query: None,
            }))
            .await
            .unwrap();
        let text2 = extract_text(&result2.content);
        let parsed2: serde_json::Value = serde_json::from_str(text2).unwrap();
        let used_after_extract = parsed2["budget_status"]["tokens_used"].as_u64().unwrap();
        assert!(
            used_after_extract > used_after_read,
            "budget should accumulate: {used_after_extract} > {used_after_read}"
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
        assert!(
            parsed2["budget_status"].is_object(),
            "response should include budget_status"
        );
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
    async fn read_returns_section_with_budget_status() {
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
        assert!(parsed["budget_status"].is_object());
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
            msg.contains("iris_survey"),
            "should suggest using iris_survey"
        );
    }

    #[test]
    fn format_index_error_includes_details() {
        let err = QueryError::Index(iris_core::error::IndexError::EmbeddingFailed {
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
        let err = QueryError::Storage(iris_core::error::StorageError::NotFound {
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
            text.contains("iris_survey"),
            "error should suggest iris_survey, got: {text}"
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

    // --- iris_evicted tests ---

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
            .evicted(Parameters(EvictedParams {
                content_ids: vec!["docs/auth.md#tokens".to_string()],
            }))
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["evicted"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["result"]["not_found"].as_array().unwrap().len(), 0);
        assert!(parsed["budget_status"].is_object());
        assert_eq!(
            parsed["budget_status"]["tokens_used"].as_u64().unwrap(),
            0,
            "budget should be zero after evicting all content"
        );
    }

    #[tokio::test]
    async fn evicted_reports_not_found_for_unknown_ids() {
        let server = setup_server().await;

        let result = server
            .evicted(Parameters(EvictedParams {
                content_ids: vec!["nonexistent".to_string()],
            }))
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["evicted"].as_array().unwrap().len(), 0);
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
            .evicted(Parameters(EvictedParams {
                content_ids: vec!["docs/auth.md#tokens".to_string(), "nonexistent".to_string()],
            }))
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["evicted"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["result"]["not_found"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn evicted_empty_list_is_noop() {
        let server = setup_server().await;

        let result = server
            .evicted(Parameters(EvictedParams {
                content_ids: vec![],
            }))
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["result"]["evicted"].as_array().unwrap().len(), 0);
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
        let text1 = extract_text(&result1.content);
        let parsed1: serde_json::Value = serde_json::from_str(text1).unwrap();
        let used_after_first = parsed1["budget_status"]["tokens_used"].as_u64().unwrap();
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

        // After fault correction, budget should be 0 — force_evict removed the
        // entry and no re-delivery occurred
        let used_after_second = parsed2["budget_status"]["tokens_used"].as_u64().unwrap();
        assert_eq!(
            used_after_second, 0,
            "budget should be zero after fault correction without re-delivery"
        );
    }

    #[test]
    fn server_instructions_include_evicted_tool() {
        let server = setup_server_sync();
        let info = server.get_info();
        let instructions = info.instructions.unwrap();
        assert!(
            instructions.contains("iris_evicted"),
            "instructions should mention iris_evicted"
        );
    }

    #[test]
    fn server_instructions_include_budget_and_compress_tools() {
        let server = setup_server_sync();
        let info = server.get_info();
        let instructions = info.instructions.unwrap();
        assert!(
            instructions.contains("iris_budget"),
            "instructions should mention iris_budget"
        );
        assert!(
            instructions.contains("iris_compress"),
            "instructions should mention iris_compress"
        );
    }

    // --- iris_budget tests ---

    #[tokio::test]
    async fn budget_returns_status_with_zero_usage() {
        let server = setup_server().await;
        let result = server.budget().await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert!(parsed["total_budget"].is_number());
        assert_eq!(parsed["estimated_used"].as_u64().unwrap(), 0);
        assert_eq!(parsed["pressure_level"], "normal");
        assert!(parsed["eviction_candidates"].is_array());
        assert!(
            parsed["eviction_candidates"].as_array().unwrap().is_empty(),
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
        let budget_config = BudgetConfig {
            max_context_tokens: 20, // Very small — any delivery triggers pressure
            pressure_threshold: 0.5,
            critical_threshold: 0.9,
        };
        let server = IrisServer::with_budget_config(service, budget_config);

        // Read a section to fill the budget
        server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();

        // Now check budget — should be under pressure with candidates
        let result = server.budget().await.unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_ne!(
            parsed["pressure_level"], "normal",
            "should be under pressure"
        );
        assert!(
            !parsed["eviction_candidates"].as_array().unwrap().is_empty(),
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

        let result = server.budget().await.unwrap();
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

    // --- iris_compress tests ---

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
    async fn compress_includes_budget_status() {
        let server = setup_server().await;
        let params = CompressParams {
            content_ids: vec!["docs/auth.md#tokens".to_string()],
        };
        let result = server.compress(Parameters(params)).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert!(
            parsed["budget_status"].is_object(),
            "compress response should include budget_status"
        );
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
            .find(|r| r.uri == "iris://status")
            .expect("should include iris://status resource");
        assert_eq!(status.name, "iris status");
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
            "iris://corpus/{path}"
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
                assert_eq!(uri, "iris://status");
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
                assert_eq!(uri, "iris://corpus/docs/auth.md");
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
            .read_resource(ReadResourceRequestParams::new("iris://status"))
            .await
            .unwrap();

        assert_eq!(result.contents.len(), 1);
    }

    #[tokio::test]
    async fn read_resource_dispatches_corpus_uri() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .read_resource(ReadResourceRequestParams::new("iris://corpus/docs/auth.md"))
            .await
            .unwrap();

        assert_eq!(result.contents.len(), 1);
    }

    #[tokio::test]
    async fn read_resource_unknown_uri_returns_error() {
        let (client, _server) = wrap_test_client(setup_server().await).await;
        let result = client
            .peer()
            .read_resource(ReadResourceRequestParams::new("iris://unknown"))
            .await;

        assert!(result.is_err());
    }

    // --- iris_clone tests ---

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

        let git_config = iris_core::git::GitFetcherConfig {
            remote_dir: std::path::PathBuf::from("/tmp/iris-test-clone"),
            ..iris_core::git::GitFetcherConfig::default()
        };
        let git_fetcher = GitFetcher::new(git_config);

        let server = IrisServer::with_persistence(service, BudgetConfig::default(), storage, None)
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
        let server = IrisServer::new(service);

        assert!(server.git_fetcher.is_none());

        let git_fetcher = GitFetcher::with_defaults();
        let server = server.with_git_fetcher(git_fetcher, embedder, index);

        assert!(server.git_fetcher.is_some());
    }

    // Progress notification tests removed — Peer::new() is pub(crate) in rmcp 0.16.
    // Progress behavior is exercised by the e2e tests through the MCP protocol layer.

    // --- Proactive eviction recommendation tests ---

    /// Helper: create a server with a tiny budget so any delivery triggers pressure.
    async fn setup_pressured_server() -> IrisServer {
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
        let budget_config = BudgetConfig {
            max_context_tokens: 20, // Very small — any delivery triggers pressure
            pressure_threshold: 0.5,
            critical_threshold: 0.9,
        };
        IrisServer::with_budget_config(service, budget_config)
    }

    #[tokio::test]
    async fn no_eviction_recommendations_at_normal_pressure() {
        let server = setup_server().await;

        // TOC doesn't deliver content, so budget stays normal
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

        assert_eq!(
            parsed["budget_status"]["pressure_level"], "normal",
            "pressure should be normal"
        );
        assert!(
            parsed.get("eviction_recommendations").is_none(),
            "should not include eviction_recommendations at normal pressure"
        );
    }

    #[tokio::test]
    async fn eviction_recommendations_included_under_elevated_pressure() {
        let server = setup_pressured_server().await;

        // Read a section to push past the pressure threshold
        server
            .read(Parameters(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            }))
            .await
            .unwrap();

        // Now call toc — a tool that doesn't deliver content itself
        // but should still include eviction recommendations
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

        assert_ne!(
            parsed["budget_status"]["pressure_level"], "normal",
            "pressure should be elevated or critical"
        );
        let recommendations = parsed["eviction_recommendations"]
            .as_array()
            .expect("should have eviction_recommendations array");
        assert!(
            !recommendations.is_empty(),
            "should have at least one eviction recommendation"
        );

        // Verify recommendation structure
        let rec = &recommendations[0];
        assert!(rec["content_id"].is_string(), "should have content_id");
        assert!(rec["reason"].is_string(), "should have reason");
        assert!(
            rec["tokens_recoverable"].is_number(),
            "should have tokens_recoverable"
        );
        assert!(rec["score"].is_number(), "should have score");
    }

    #[tokio::test]
    async fn eviction_recommendations_in_survey_response_under_pressure() {
        let server = setup_pressured_server().await;

        // First survey fills the budget
        let result = server
            .survey(Parameters(SurveyParams {
                query: "JWT tokens".to_string(),
                top_k: Some(5),
            }))
            .await
            .unwrap();
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        // With a 20-token budget the first survey should push past the threshold
        if parsed["budget_status"]["pressure_level"] != "normal" {
            let recommendations = parsed["eviction_recommendations"]
                .as_array()
                .expect("should have eviction_recommendations");
            assert!(
                !recommendations.is_empty(),
                "survey response should include eviction recommendations under pressure"
            );
        }
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
            .insert("iris://status".to_string());
        let subs = server.subscriptions.lock().await;
        assert!(subs.contains("iris://status"));
    }

    #[tokio::test]
    async fn subscribe_is_idempotent() {
        let server = setup_server().await;
        let subs = &server.subscriptions;
        subs.lock().await.insert("iris://status".to_string());
        subs.lock().await.insert("iris://status".to_string());
        assert_eq!(subs.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn unsubscribe_removes_uri() {
        let server = setup_server().await;
        server
            .subscriptions
            .lock()
            .await
            .insert("iris://status".to_string());
        server.subscriptions.lock().await.remove("iris://status");
        assert!(server.subscriptions.lock().await.is_empty());
    }

    #[tokio::test]
    async fn unsubscribe_nonexistent_is_noop() {
        let server = setup_server().await;
        server
            .subscriptions
            .lock()
            .await
            .remove("iris://nonexistent");
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
            .insert("iris://status".to_string());

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

    // --- iris_task tests ---

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
            "iris_survey",
            "iris_read",
            "iris_extract",
            "iris_related",
            "iris_budget",
            "iris_toc",
            "iris_task",
            "iris_symbols",
            "iris_definition",
            "iris_references",
        ];

        let mutating_tools = [
            "iris_evicted",
            "iris_compress",
            "iris_fetch",
            "iris_refresh",
            "iris_clone",
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

        let open_world = ["iris_fetch", "iris_refresh", "iris_clone"];

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

    /// Verify that `iris_survey` returns structured content.
    #[tokio::test]
    async fn survey_returns_structured_content() {
        let server = setup_server().await;
        let (client, _server_handle) = wrap_test_client(server).await;

        let args: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({"query": "authentication"})).unwrap();
        let result = client
            .call_tool(rmcp::model::CallToolRequestParams::new("iris_survey").with_arguments(args))
            .await
            .unwrap();

        // Must have structured_content
        let sc = result
            .structured_content
            .as_ref()
            .expect("iris_survey should return structured_content");

        // The structured content should be a JSON object with expected fields
        assert!(
            sc.get("results").is_some() || sc.get("budget_status").is_some(),
            "structured_content should contain results or budget_status, got: {sc:?}"
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
            .get_tool("iris_fetch")
            .expect("iris_fetch tool not found");
        assert_eq!(
            fetch_tool.task_support(),
            rmcp::model::TaskSupport::Optional,
            "iris_fetch should support optional task mode"
        );
    }

    #[tokio::test]
    async fn clone_tool_has_optional_task_support() {
        let server = setup_server().await;
        let clone_tool = server
            .get_tool("iris_clone")
            .expect("iris_clone tool not found");
        assert_eq!(
            clone_tool.task_support(),
            rmcp::model::TaskSupport::Optional,
            "iris_clone should support optional task mode"
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

        assert_eq!(card["serverInfo"]["name"], "iris");
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
    fn server_card_includes_iris_extensions() {
        let server = setup_server_sync();
        let card = server.build_server_card();

        let extensions = &card["capabilities"]["extensions"];
        assert!(
            extensions[EXT_BUDGET_PROTOCOL].is_object(),
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
        // All 15 iris tools should be listed.
        assert!(
            tools.len() >= 15,
            "should have at least 15 tools, got {}",
            tools.len()
        );

        let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        for expected in &[
            "iris_survey",
            "iris_read",
            "iris_extract",
            "iris_related",
            "iris_evicted",
            "iris_budget",
            "iris_compress",
            "iris_toc",
            "iris_fetch",
            "iris_refresh",
            "iris_clone",
            "iris_task",
            "iris_symbols",
            "iris_definition",
            "iris_references",
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
        assert_eq!(parsed["serverInfo"]["name"], "iris");
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
        use iris_core::session::{BudgetStatus, PressureLevel};

        // Two different tool result types with identical budget status
        let budget = BudgetStatus {
            tokens_used: 5000,
            tokens_remaining: 95_000,
            pressure_level: PressureLevel::Normal,
            utilization: 0.05,
        };

        let resp_a = ToolResponse {
            budget_status: budget.clone(),
            coherence_alerts: Vec::new(),
            indexing_in_progress: false,
            indexing_message: None,
            eviction_recommendations: Vec::new(),
            result: serde_json::json!({"results": [1, 2, 3]}),
        };
        let resp_b = ToolResponse {
            budget_status: budget,
            coherence_alerts: Vec::new(),
            indexing_in_progress: false,
            indexing_message: None,
            eviction_recommendations: Vec::new(),
            result: serde_json::json!({"symbols": ["foo", "bar"]}),
        };

        let json_a = serde_json::to_string(&resp_a).unwrap();
        let json_b = serde_json::to_string(&resp_b).unwrap();

        // Both should start with identical budget_status prefix
        let prefix_a = &json_a[..json_a.find("\"result\"").unwrap()];
        let prefix_b = &json_b[..json_b.find("\"result\"").unwrap()];
        assert_eq!(
            prefix_a, prefix_b,
            "stable prefix should be byte-identical across different tool responses"
        );
    }

    #[test]
    fn tool_response_result_not_flattened() {
        use iris_core::session::{BudgetStatus, PressureLevel};

        let resp = ToolResponse {
            budget_status: BudgetStatus {
                tokens_used: 0,
                tokens_remaining: 100_000,
                pressure_level: PressureLevel::Normal,
                utilization: 0.0,
            },
            coherence_alerts: Vec::new(),
            indexing_in_progress: false,
            indexing_message: None,
            eviction_recommendations: Vec::new(),
            result: serde_json::json!({"items": [1]}),
        };

        let v = serde_json::to_value(&resp).unwrap();
        let obj = v.as_object().unwrap();

        // `result` should be a nested object, not flattened
        assert!(obj.contains_key("result"), "should have 'result' key");
        assert!(
            obj.contains_key("budget_status"),
            "should have 'budget_status' key"
        );
        // Flattened fields should NOT appear at top level
        assert!(
            !obj.contains_key("items"),
            "'items' should be inside 'result', not flattened"
        );
    }

    #[test]
    fn tool_response_skips_empty_optional_fields() {
        use iris_core::session::{BudgetStatus, PressureLevel};

        let resp = ToolResponse {
            budget_status: BudgetStatus {
                tokens_used: 0,
                tokens_remaining: 100_000,
                pressure_level: PressureLevel::Normal,
                utilization: 0.0,
            },
            coherence_alerts: Vec::new(),
            indexing_in_progress: false,
            indexing_message: None,
            eviction_recommendations: Vec::new(),
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
            !obj.contains_key("eviction_recommendations"),
            "empty recs should be skipped"
        );

        // Only budget_status and result should remain
        assert_eq!(obj.len(), 2, "should only have budget_status and result");
    }
}
