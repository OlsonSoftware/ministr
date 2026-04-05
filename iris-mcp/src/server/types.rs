//! MCP tool parameter and response types for the iris server.
//!
//! All `*Params` structs (deserialized from tool call arguments) and
//! `*Response` structs (serialized into tool results) live here,
//! keeping the main server module focused on handler logic.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use iris_core::service::{CompressedItem, RelatedClaimResult, SurveyResult, SymbolRefResult};
use iris_core::session::eviction::EvictionCandidate;
use iris_core::session::{BudgetStatus, CoherenceAlert};

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
pub(crate) struct ToolResponse<T: Serialize + schemars::JsonSchema> {
    /// Current budget status snapshot (stable across consecutive calls).
    pub(crate) budget_status: BudgetStatus,
    /// Pending coherence alerts (present when underlying content has changed).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[schemars(default)]
    pub(crate) coherence_alerts: Vec<CoherenceAlert>,
    /// True when background corpus ingestion is still running.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    #[schemars(default)]
    pub(crate) indexing_in_progress: bool,
    /// Human-readable ingestion status message (e.g. "Checking 12/42 files").
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(default)]
    pub(crate) indexing_message: Option<String>,
    /// Proactive eviction recommendations when budget pressure is elevated or critical.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[schemars(default)]
    pub(crate) eviction_recommendations: Vec<EvictionCandidate>,
    /// The tool-specific result data (varying — placed last for prefix stability).
    pub(crate) result: T,
}

/// Wrapper for survey responses that includes both results and dedup metadata.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct SurveyResponse {
    /// The survey results after deduplication.
    pub(crate) results: Vec<SurveyResult>,
    /// Number of results filtered out by deduplication.
    pub(crate) deduplicated_count: usize,
}

/// Wrapper for extract responses.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct ExtractResponse {
    /// The extracted claims.
    pub(crate) claims: Vec<iris_core::service::ClaimResult>,
}

/// Response when a section has already been delivered and is unchanged.
///
/// Returned instead of full text to avoid wasting context tokens on
/// content the agent already has.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct AlreadyDeliveredResponse {
    /// The requested section ID.
    pub(crate) section_id: String,
    /// Always `"already_delivered"`.
    pub(crate) status: &'static str,
    /// Number of claims available for extraction.
    pub(crate) claims_available: usize,
}

/// Response from the `iris_evicted` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct EvictedResponse {
    /// Content IDs that were successfully removed.
    pub(crate) evicted: Vec<String>,
    /// Content IDs that were not found in the session.
    pub(crate) not_found: Vec<String>,
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
pub(crate) struct BudgetResponse {
    /// Total context window budget in tokens.
    pub(crate) total_budget: usize,
    /// Estimated tokens currently used.
    pub(crate) estimated_used: usize,
    /// Estimated tokens remaining.
    pub(crate) estimated_remaining: usize,
    /// Current pressure level.
    pub(crate) pressure_level: String,
    /// Recommended eviction candidates (empty under normal pressure).
    pub(crate) eviction_candidates: Vec<EvictionCandidate>,
    /// Prefetch cache hit/miss metrics by strategy.
    pub(crate) prefetch_metrics: iris_core::session::PrefetchMetrics,
    /// Cumulative session token economics.
    pub(crate) session_metrics: SessionMetricsResponse,
    /// Total tokens consumed by MCP tool schemas (descriptions + parameters).
    pub(crate) schema_tokens: usize,
    /// Number of registered tools.
    pub(crate) tool_count: usize,
    /// Pending coherence alerts (present when underlying content has changed).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[schemars(default)]
    pub(crate) coherence_alerts: Vec<CoherenceAlert>,
    /// Content IDs evicted via interactive elicitation (empty if elicitation
    /// was unavailable, declined, or pressure was normal).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[schemars(default)]
    pub(crate) elicitation_evicted: Vec<String>,
}

/// Cumulative token economics for a session.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct SessionMetricsResponse {
    /// Total content deliveries (including re-deliveries).
    pub(crate) total_deliveries: u64,
    /// Cumulative tokens delivered across all deliveries.
    pub(crate) cumulative_tokens_delivered: u64,
    /// Total explicit evictions.
    pub(crate) total_evictions: u64,
    /// Cumulative tokens freed by evictions.
    pub(crate) cumulative_tokens_evicted: u64,
    /// Total compression tier transitions.
    pub(crate) total_compressions: u64,
    /// Cumulative tokens freed by compressions.
    pub(crate) cumulative_tokens_compressed: u64,
    /// Net token savings (evicted + compressed).
    pub(crate) total_tokens_saved: u64,
    /// Savings ratio (saved / delivered). 0.0 if nothing delivered.
    pub(crate) compression_ratio: f64,
    /// Delta updates (content changed since last delivery).
    pub(crate) delta_updates: u64,
    /// Dedup hits (agent re-requested already-delivered content).
    pub(crate) dedup_hits: u64,
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
pub(crate) struct RelatedResponse {
    /// Related claims with relationship metadata.
    pub(crate) related: Vec<RelatedClaimResult>,
}

/// Response from the `iris_compress` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct CompressResponse {
    /// Compressed summaries for the requested content.
    pub(crate) summaries: Vec<CompressedItem>,
    /// Total original tokens across all compressed items.
    pub(crate) total_original_tokens: usize,
    /// Total compressed tokens across all compressed items.
    pub(crate) total_compressed_tokens: usize,
    /// Aggregate compression ratio (compressed / original). Lower is better.
    pub(crate) compression_ratio: f64,
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
pub(crate) struct FetchResponse {
    /// Number of pages successfully fetched.
    pub(crate) pages_fetched: usize,
    /// Total sections indexed across all fetched pages.
    pub(crate) sections_indexed: usize,
    /// Total claims extracted across all fetched pages.
    pub(crate) claims_extracted: usize,
    /// Total tokens added to the corpus.
    pub(crate) tokens_added: usize,
    /// The fetch strategy that was used.
    pub(crate) strategy_used: String,
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
pub(crate) struct RefreshUrlDetailResponse {
    /// The URL that was checked.
    pub(crate) url: String,
    /// The outcome: "unchanged", "updated", or "failed: <reason>".
    pub(crate) status: String,
}

/// Per-repo refresh detail for the response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct RefreshGitDetailResponse {
    /// The repository URL that was checked.
    pub(crate) repo_url: String,
    /// The outcome: "unchanged", "updated", or "failed: <reason>".
    pub(crate) status: String,
}

/// Response from the `iris_refresh` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct RefreshResponse {
    /// Number of web URLs checked.
    pub(crate) urls_checked: usize,
    /// Number of web URLs that had new content and were re-indexed.
    pub(crate) urls_refreshed: usize,
    /// Number of web URLs that were unchanged.
    pub(crate) urls_unchanged: usize,
    /// Number of web URLs where the check failed.
    pub(crate) urls_failed: usize,
    /// Per-URL details for web sources.
    pub(crate) details: Vec<RefreshUrlDetailResponse>,
    /// Number of git repos checked.
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) git_repos_checked: usize,
    /// Number of git repos that had new commits and were re-indexed.
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) git_repos_refreshed: usize,
    /// Number of git repos that were unchanged.
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) git_repos_unchanged: usize,
    /// Number of git repos where the check failed.
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) git_repos_failed: usize,
    /// Per-repo details for git sources.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) git_details: Vec<RefreshGitDetailResponse>,
}

/// Helper for `skip_serializing_if` on zero counts.
///
/// Must take `&usize` (not `usize`) because serde's `skip_serializing_if`
/// passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn is_zero(val: &usize) -> bool {
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
pub(crate) struct CloneResponse {
    /// Number of files discovered in the clone checkout.
    pub(crate) files_discovered: usize,
    /// Number of files that were indexed (parsed and stored).
    pub(crate) files_indexed: usize,
    /// Total sections extracted across all indexed files.
    pub(crate) sections_extracted: usize,
    /// Time spent cloning the repository in milliseconds.
    pub(crate) clone_time_ms: u64,
    /// Time spent running the ingestion pipeline in milliseconds.
    pub(crate) index_time_ms: u64,
    /// Whether the clone was served from cache.
    pub(crate) from_cache: bool,
    /// Number of cross-references linked from local code to the cloned dependency.
    pub(crate) dependency_refs_linked: usize,
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
pub(crate) struct TaskStatusResponse {
    /// Unique task identifier.
    pub(crate) task_id: String,
    /// Current status: `working`, `completed`, `failed`, `cancelled`, or `input_required`.
    pub(crate) status: String,
    /// Human-readable status message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status_message: Option<String>,
    /// ISO-8601 creation timestamp.
    pub(crate) created_at: String,
    /// ISO-8601 timestamp for the most recent status change.
    pub(crate) last_updated_at: String,
    /// Retention window in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ttl: Option<u64>,
    /// Suggested polling interval in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) poll_interval: Option<u64>,
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
pub(crate) struct SymbolsResponse {
    /// Matching symbols.
    pub(crate) symbols: Vec<SymbolSummary>,
    /// Total number of matches (before pagination).
    pub(crate) total: usize,
    /// Offset of the first returned entry within the full list.
    pub(crate) offset: usize,
}

/// A compact symbol summary for search results.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct SymbolSummary {
    /// Symbol ID (use with `iris_definition` / `iris_references`).
    pub(crate) id: String,
    /// Symbol name.
    pub(crate) name: String,
    /// Symbol kind.
    pub(crate) kind: String,
    /// Source file path.
    pub(crate) file: String,
    /// Start line number.
    pub(crate) line: u32,
    /// Declaration signature.
    pub(crate) signature: String,
    /// First line of doc comment, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) doc_preview: Option<String>,
    /// Cyclomatic complexity (functions only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) complexity: Option<u32>,
    /// Transitive caller count (impact analysis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) caller_count: Option<u32>,
}

/// Response from the `iris_references` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct ReferencesResponse {
    /// Cross-references for the symbol.
    pub(crate) references: Vec<SymbolRefResult>,
    /// Total number of references (before pagination).
    pub(crate) total: usize,
    /// Offset of the first returned entry within the full list.
    pub(crate) offset: usize,
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
pub(crate) struct BridgeResponse {
    /// Matched bridge links.
    pub(crate) links: Vec<BridgeLinkSummary>,
    /// Total number of links returned.
    pub(crate) total: usize,
}

/// A compact bridge link summary for search results.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct BridgeLinkSummary {
    /// Bridge mechanism kind (e.g. `"tauri_command"`).
    pub(crate) kind: String,
    /// Combined confidence score.
    pub(crate) confidence: f32,
    /// Export (definition) side.
    pub(crate) export: BridgeEndpointSummary,
    /// Import (call site) side.
    pub(crate) import: BridgeEndpointSummary,
}

/// One side of a bridge link in the response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct BridgeEndpointSummary {
    /// Binding key.
    pub(crate) binding_key: String,
    /// Symbol name.
    pub(crate) symbol_name: String,
    /// Source file path.
    pub(crate) file: String,
    /// Source line number.
    pub(crate) line: u32,
    /// Language.
    pub(crate) language: String,
}

/// Corpus-level statistics returned in the `iris_toc` response header.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct CorpusStatsHeader {
    /// Number of documents in the corpus.
    pub(crate) documents: usize,
    /// Total number of sections across all documents (before pagination).
    pub(crate) sections: usize,
    /// Number of claims across all sections.
    pub(crate) claims: usize,
    /// Offset of the first returned entry within the full list.
    pub(crate) offset: usize,
    /// Number of entries returned in this page.
    pub(crate) returned: usize,
    /// Ingestion state: `"pending"`, `"running"`, or `"complete"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ingestion_status: Option<String>,
}

/// Response from the `iris_toc` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct TocResponse {
    /// Corpus-level statistics for quick orientation.
    pub(crate) corpus_stats: CorpusStatsHeader,
    /// Registered corpus roots with per-directory metadata and language stats.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) roots: Vec<iris_core::types::CorpusRoot>,
    /// Table of contents entries (metadata only, no text).
    pub(crate) entries: Vec<iris_core::types::TocEntry>,
}

// -- Union output types for tools that return different response shapes --

/// Output data for `iris_read`: either full section detail or an "already delivered" skip.
///
/// Used only for output schema generation via `schemars::JsonSchema`.
#[expect(dead_code)]
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub(crate) enum ReadOutputData {
    /// Section was already delivered and is unchanged.
    AlreadyDelivered(AlreadyDeliveredResponse),
    /// Full section detail (new or changed content).
    Detail(iris_core::service::SectionDetail),
}

/// Output data for `iris_fetch`.
///
/// Used for output schema generation via `schemars::JsonSchema`.
/// When invoked as an MCP Task, the result is delivered via `tasks/result`.
pub(crate) type FetchOutputData = ToolResponse<FetchResponse>;

/// Output data for `iris_clone`.
///
/// Used for output schema generation via `schemars::JsonSchema`.
/// When invoked as an MCP Task, the result is delivered via `tasks/result`.
pub(crate) type CloneOutputData = ToolResponse<CloneResponse>;

/// Generate the output schema `Arc<JsonObject>` for a tool response type.
///
/// Used in `#[tool(output_schema = ...)]` macro attributes to provide
/// static output schemas derived from the response types' `JsonSchema` impls.
///
/// Handles types with `#[serde(flatten)]` (which produce `allOf` schemas without
/// a root `type: "object"`) by injecting the required `type` field so the schema
/// passes rmcp's MCP spec validation.
pub(crate) fn tool_output_schema<T: schemars::JsonSchema + 'static>()
-> std::sync::Arc<rmcp::model::JsonObject> {
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
