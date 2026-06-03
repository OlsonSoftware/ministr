//! MCP tool parameter and response types for the ministr server.
//!
//! All `*Params` structs (deserialized from tool call arguments) and
//! `*Response` structs (serialized into tool results) live here,
//! keeping the main server module focused on handler logic.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use super::coerce;

use ministr_core::service::{
    CompressedItem, DeadSymbol, ImpactResult, RelatedClaimResult, SolidFinding, SurveyResult,
    SymbolRefResult,
};
use ministr_core::session::drops::DropCandidate;
use ministr_core::session::{CoherenceAlert, UsageStatus};

/// Tool response wrapper.
///
/// Carries the tool-specific `result` plus a few genuinely actionable
/// signals (coherence alerts when content changed underneath the agent,
/// ingestion progress, concrete follow-up hints).
///
/// **Budget is deliberately not surfaced here.** `usage_status` and
/// `drop_suggestions` are still tracked internally (the
/// `UsageTracker` keeps recording so compression and dedup keep working),
/// but they are no longer serialized to the agent: the per-response
/// numbers were anchored to an arbitrary window and were causing agents
/// to wrongly conclude they were almost out of context and abandon work.
/// Both fields are retained on the struct (constructors/tests still set
/// them) and marked `#[serde(skip_serializing)]` so nothing reaches the
/// model. An agent that genuinely wants the figure can still call
/// `ministr_usage` explicitly.
///
/// Fields are ordered for KV-cache prefix stability: stable metadata
/// first, varying tool-specific payload last.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct ToolResponse<T: Serialize + schemars::JsonSchema> {
    /// Internal budget snapshot — tracked, never sent to the agent.
    /// See the struct-level note on why this is `skip_serializing`.
    ///
    /// Constructed by every `build_response` call (keeping that
    /// signature stable across all tool handlers) but deliberately not
    /// read back out — the agent-facing path is gone and internal
    /// pressure tracking lives in `UsageTracker`, not here.
    #[expect(
        dead_code,
        reason = "retained so build_response's contract is unchanged; intentionally not serialized"
    )]
    #[serde(skip_serializing)]
    #[schemars(skip)]
    pub(crate) usage_status: UsageStatus,
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
    /// Internal eviction candidates — never sent to the agent. Always
    /// empty now (`build_response_with` stopped computing them); kept as
    /// a field so the struct shape and constructors are unchanged.
    #[expect(
        dead_code,
        reason = "no longer populated or surfaced; field retained to avoid churning all constructors"
    )]
    #[serde(skip_serializing)]
    #[schemars(skip)]
    pub(crate) drop_suggestions: Vec<DropCandidate>,
    /// Concrete next-tool-call suggestions, in priority order.
    ///
    /// Coherence-driven (re-read changed sections) plus any per-handler
    /// hints (e.g. survey's top-result follow-up). Budget pressure no
    /// longer contributes entries here — see the struct-level note.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[schemars(default)]
    pub(crate) next_actions: Vec<NextAction>,
    /// The tool-specific result data (varying — placed last for prefix stability).
    pub(crate) result: T,
}

/// A concrete suggested next tool call the agent should consider making.
///
/// The server uses these to surface the playbook in-band: pressure-driven
/// (compress + evicted), coherence-driven (re-read changed sections), or
/// chain-driven (read top survey hit, fetch definition for a single symbol
/// match, etc.). Cooperative — agents that ignore these still work.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct NextAction {
    /// Tool name to call (e.g. `"ministr_dropped"`, `"ministr_read"`).
    pub(crate) action: String,
    /// Suggested arguments as a JSON object matching the tool's input schema.
    pub(crate) args: serde_json::Value,
    /// One-sentence reason this action is being suggested.
    pub(crate) reason: String,
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
    pub(crate) claims: Vec<ministr_core::service::ClaimResult>,
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

/// Response from the `ministr_dropped` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct DroppedResponse {
    /// Content IDs that were successfully removed.
    pub(crate) dropped: Vec<String>,
    /// Content IDs that were not found in the session.
    pub(crate) not_found: Vec<String>,
}

/// Parameters for the `ministr_survey` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SurveyParams {
    /// Natural language query to search for relevant content.
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(description = "Natural language query to search the corpus")]
    pub query: String,

    /// Maximum number of results to return (default: 10).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Maximum number of results to return")]
    pub top_k: Option<usize>,

    /// Optional linked-project label. Omit for the session's primary corpus.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Optional linked-project label (from .ministr.toml [[linked]]). \
                       Omit for the session's primary corpus. Call ministr_projects to list labels."
    )]
    pub project: Option<String>,

    /// F6.3-a — cross-corpus fan-out. When set and non-empty, runs the
    /// query against each listed corpus (own corpora or Atlas slugs
    /// like `atlas/react`), tags each hit with `source_corpus`, and
    /// merges all results sorted by score descending — truncated to
    /// `top_k`. When omitted or empty, behaviour is unchanged (single
    /// corpus resolved through `project`). Mutually compatible with
    /// `project`: when both are set, `corpus_ids` wins.
    #[serde(default, deserialize_with = "coerce::opt_string_or_seq")]
    #[schemars(
        description = "Optional cross-corpus list. When set and non-empty, fans the query out \
                       across each corpus_id (own corpora or Atlas slugs), tags hits with \
                       source_corpus, and merges results by score. Omit to query a single corpus."
    )]
    pub corpus_ids: Option<Vec<String>>,

    /// F6.3-b — per-corpus score multipliers for cross-corpus ranking.
    /// Keys are `corpus_id` strings matching `corpus_ids`; values are
    /// non-negative multipliers (1.0 = unboosted, 2.0 = double weight,
    /// 0.0 = suppressed). Absent corpora default to 1.0. Values are
    /// clamped to `[0.0, 10.0]` and any non-finite (NaN, ±∞) value is
    /// rejected back to 1.0. Only consulted when `corpus_ids` is set.
    #[serde(default, deserialize_with = "coerce::lenient_opt_f32_map")]
    #[schemars(
        description = "Optional per-corpus score multipliers for cross-corpus ranking. \
                       Map<corpus_id, multiplier>; absent corpora default to 1.0; clamped to [0, 10]. \
                       Use 2.0 to float your own repo above Atlas hits, 0.0 to suppress a corpus."
    )]
    pub corpus_boost: Option<std::collections::HashMap<String, f32>>,
}

/// Parameters for the `ministr_read` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadParams {
    /// Hierarchical section ID (e.g. `docs/auth.md#error-handling`).
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(description = "Section ID to read (e.g. 'docs/auth.md#error-handling')")]
    pub section_id: String,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Parameters for the `ministr_dropped` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DroppedParams {
    /// Content IDs that the agent has dropped from its context window.
    #[serde(default, deserialize_with = "coerce::string_or_seq")]
    #[schemars(description = "Content IDs the agent has dropped from its context")]
    pub content_ids: Vec<String>,
}

/// Parameters for the `ministr_extract` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExtractParams {
    /// Section ID to extract claims from.
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(description = "Section ID to extract claims from")]
    pub section_id: String,

    /// Optional query to filter claims by relevance.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional query to filter claims by relevance")]
    pub query: Option<String>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Parameters for the `ministr_compress` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompressParams {
    /// Content IDs to generate compressed summaries for.
    #[serde(default, deserialize_with = "coerce::string_or_seq")]
    #[schemars(description = "Content IDs (section IDs) to generate compressed summaries for")]
    pub content_ids: Vec<String>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Response from the `ministr_usage` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct UsageResponse {
    /// Total context window budget in tokens.
    pub(crate) total_budget: usize,
    /// Estimated tokens currently used.
    pub(crate) estimated_used: usize,
    /// Estimated tokens remaining.
    pub(crate) estimated_remaining: usize,
    /// Current pressure level.
    pub(crate) level: String,
    /// Recommended eviction candidates (empty under normal pressure).
    pub(crate) drop_candidates: Vec<DropCandidate>,
    /// Prefetch cache hit/miss metrics by strategy.
    pub(crate) prefetch_metrics: ministr_core::session::PrefetchMetrics,
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

/// Parameters for the `ministr_related` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RelatedParams {
    /// Claim ID to find related claims for.
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(description = "Claim ID to find related claims for")]
    pub claim_id: String,

    /// Optional filter for specific relation types.
    #[serde(default, deserialize_with = "coerce::opt_string_or_seq")]
    #[schemars(
        description = "Optional filter: 'references', 'contradicts', 'depends_on', 'updates'"
    )]
    pub relation_types: Option<Vec<String>>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Response from the `ministr_related` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct RelatedResponse {
    /// Related claims with relationship metadata.
    pub(crate) related: Vec<RelatedClaimResult>,
}

/// Response from the `ministr_compress` tool.
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

/// Parameters for the `ministr_toc` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TocParams {
    /// Optional document ID filter — returns only sections from this document.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Optional document ID to filter the table of contents to a single document"
    )]
    pub document_id: Option<String>,

    /// Number of entries to skip (default: 0). Use with `limit` for pagination.
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Number of entries to skip (default: 0)")]
    pub offset: Option<usize>,

    /// Maximum number of entries to return (default: 100).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Maximum number of entries to return (default: 100)")]
    pub limit: Option<usize>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Parameters for the `ministr_fetch` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FetchParams {
    /// URL to fetch content from.
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(description = "URL to fetch content from (e.g. 'https://docs.example.com/')")]
    pub url: String,

    /// Crawl depth for following links (default: 0 = single page).
    /// Depth 1+ follows same-domain links up to this depth.
    #[serde(default, deserialize_with = "coerce::lenient_opt_u32")]
    #[schemars(description = "Crawl depth for following links (default: 0 = single page only)")]
    pub depth: Option<u32>,

    /// Maximum number of pages to fetch when crawling (default: 50).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Maximum number of pages to fetch when crawling (default: 50)")]
    pub max_pages: Option<usize>,

    /// Only fetch URLs whose path starts with this prefix (e.g. '/docs/').
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Only fetch URLs whose path starts with this prefix (e.g. '/docs/')")]
    pub path_filter: Option<String>,
}

/// Response from the `ministr_fetch` tool.
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

/// Parameters for the `ministr_refresh` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RefreshParams {
    /// Optional URL or repo URL to refresh. If omitted, checks all cached web
    /// and git sources.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
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

/// Response from the `ministr_refresh` tool.
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

/// Parameters for the `ministr_clone` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CloneParams {
    /// Remote git repository URL (HTTPS or SSH).
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(
        description = "Remote git repository URL to clone (e.g. 'https://github.com/owner/repo.git')"
    )]
    pub repo: String,

    /// Optional list of paths for sparse checkout — only these directories/files
    /// will be checked out and indexed.
    #[serde(default, deserialize_with = "coerce::opt_string_or_seq")]
    #[schemars(
        description = "Optional paths for sparse checkout (e.g. ['docs', 'src']). Omit for full checkout."
    )]
    pub paths: Option<Vec<String>>,

    /// Optional branch to clone (defaults to the repository's default branch).
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional branch to clone (defaults to repository default)")]
    pub branch: Option<String>,
}

/// Response from the `ministr_clone` tool.
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

/// Parameters for the `ministr_task` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskParams {
    /// Task ID to check status for.
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(description = "Task ID to check status for")]
    pub task_id: String,
}

/// Output schema for the `ministr_task` tool response.
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

/// Parameters for the `ministr_symbols` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolsParams {
    /// Fuzzy name search (case-insensitive substring match).
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Fuzzy symbol name search (case-insensitive substring match)")]
    pub query: Option<String>,

    /// Exact kind filter (e.g. "function", "struct", "trait", "enum", "impl").
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Exact kind filter: 'function', 'struct', 'trait', 'enum', 'impl', 'const', 'static', 'type', 'mod'"
    )]
    pub kind: Option<String>,

    /// Module path prefix filter (e.g. "config" matches `config::sub`).
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Module path prefix filter (e.g. 'config' matches config::sub)")]
    pub module: Option<String>,

    /// Exact visibility filter (e.g. "pub", "pub(crate)", "").
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Exact visibility filter: 'pub', 'pub(crate)', 'pub(super)', ''")]
    pub visibility: Option<String>,

    /// Number of entries to skip (default: 0). Use with `limit` for pagination.
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Number of entries to skip (default: 0)")]
    pub offset: Option<usize>,

    /// Maximum number of entries to return (default: 100).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Maximum number of entries to return (default: 100)")]
    pub limit: Option<usize>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Parameters for the `ministr_definition` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DefinitionParams {
    /// The symbol ID to look up.
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(description = "Symbol ID to get the definition for (from ministr_symbols results)")]
    pub symbol_id: String,

    /// Position-addressed lookup: file path of the cursor. When `symbol_id`
    /// is empty and `file`+`line`+`col` are supplied, the symbol under the
    /// cursor is resolved via the occurrence index (LSP-style addressing).
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Position-addressed alternative to symbol_id: file path (with line+col) to resolve the symbol under the cursor"
    )]
    pub file: Option<String>,

    /// 1-based line of the cursor position.
    #[serde(default, deserialize_with = "coerce::lenient_opt_u32")]
    #[schemars(description = "1-based line for position-addressed lookup (requires file+col)")]
    pub line: Option<u32>,

    /// 0-based byte column of the cursor position.
    #[serde(default, deserialize_with = "coerce::lenient_opt_u32")]
    #[schemars(
        description = "0-based byte column for position-addressed lookup (requires file+line)"
    )]
    pub col: Option<u32>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Parameters for the `ministr_references` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReferencesParams {
    /// The symbol ID to find references for.
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(description = "Symbol ID to find references for (from ministr_symbols results)")]
    pub symbol_id: String,

    /// Position-addressed lookup: file path of the cursor. When `symbol_id`
    /// is empty and `file`+`line`+`col` are supplied, references to the
    /// symbol under the cursor are returned (LSP-style addressing).
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Position-addressed alternative to symbol_id: file path (with line+col) to resolve the symbol under the cursor"
    )]
    pub file: Option<String>,

    /// 1-based line of the cursor position.
    #[serde(default, deserialize_with = "coerce::lenient_opt_u32")]
    #[schemars(description = "1-based line for position-addressed lookup (requires file+col)")]
    pub line: Option<u32>,

    /// 0-based byte column of the cursor position.
    #[serde(default, deserialize_with = "coerce::lenient_opt_u32")]
    #[schemars(
        description = "0-based byte column for position-addressed lookup (requires file+line)"
    )]
    pub col: Option<u32>,

    /// Optional reference kind filter.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Optional reference kind filter: 'calls', 'implements', 'imports', 'uses', 'bridge'"
    )]
    pub ref_kind: Option<String>,

    /// Number of entries to skip (default: 0). Use with `limit` for pagination.
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Number of entries to skip (default: 0)")]
    pub offset: Option<usize>,

    /// Maximum number of entries to return (default: 100).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Maximum number of entries to return (default: 100)")]
    pub limit: Option<usize>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Parameters for the `ministr_impact` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImpactParams {
    /// The symbol ID whose blast radius should be analyzed.
    #[serde(default, deserialize_with = "coerce::lenient_string")]
    #[schemars(
        description = "Symbol ID whose blast radius should be analyzed (from ministr_symbols results)"
    )]
    pub symbol_id: String,

    /// Maximum BFS depth to walk (default 3, capped at 10).
    #[serde(default, deserialize_with = "coerce::lenient_opt_u32")]
    #[schemars(description = "Maximum BFS depth to walk the call graph. Default 3, capped at 10.")]
    pub max_depth: Option<u32>,

    /// Call-graph direction: `incoming` (callers, default) or `outgoing` (callees).
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Call-graph direction: 'incoming' = transitive callers / blast radius (default), 'outgoing' = transitive callees (what this symbol calls)."
    )]
    pub direction: Option<String>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Parameters for the `ministr_dead` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeadCodeParams {
    /// Optional kind filter (e.g. `"function"`, `"struct"`).
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional symbol kind filter (e.g. 'function', 'struct')")]
    pub kind: Option<String>,

    /// Optional module path prefix filter.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional module path prefix filter")]
    pub module: Option<String>,

    /// Skip symbols whose body is shorter than this many lines (default 1).
    #[serde(default, deserialize_with = "coerce::lenient_opt_u32")]
    #[schemars(
        description = "Skip symbols whose body is shorter than this many lines. Default 1."
    )]
    pub min_lines: Option<u32>,

    /// Maximum results to return (default 50, capped at 500).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Maximum results to return. Default 50, capped at 500.")]
    pub limit: Option<usize>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Response from the `ministr_symbols` tool.
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
    /// Symbol ID (use with `ministr_definition` / `ministr_references`).
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

/// Response from the `ministr_references` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct ReferencesResponse {
    /// Cross-references for the symbol.
    pub(crate) references: Vec<SymbolRefResult>,
    /// Total number of references (before pagination).
    pub(crate) total: usize,
    /// Offset of the first returned entry within the full list.
    pub(crate) offset: usize,
}

/// Response from the `ministr_impact` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct ImpactResponse {
    /// The impact analysis result.
    pub(crate) impact: ImpactResult,
}

/// Response from the `ministr_dead` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct DeadCodeResponse {
    /// Dead-code candidates.
    pub(crate) symbols: Vec<DeadSymbol>,
    /// Total number of candidates that matched (before the limit cap).
    pub(crate) total: usize,
}

/// Parameters for the `ministr_solid` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SolidParams {
    /// Optional symbol-kind filter for the candidate set
    /// (e.g. `"function"`).
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional symbol kind filter (e.g. 'function', 'struct')")]
    pub kind: Option<String>,

    /// Module-path prefix filter (e.g. `"server"`).
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional module path prefix filter")]
    pub module: Option<String>,

    /// Which principles to evaluate: any subset of `dry_ocp`, `srp`, `isp`,
    /// `dip`, `shotgun_surgery`, `cyclic_dependency`. Empty / omitted = all six.
    #[serde(default, deserialize_with = "coerce::string_or_seq")]
    #[schemars(
        description = "Principles to evaluate: 'dry_ocp', 'srp', 'isp', 'dip', 'shotgun_surgery', 'cyclic_dependency'. Omit or pass empty to run all six."
    )]
    pub principles: Vec<String>,

    /// Override container kinds (defaults cover Rust/TS/Python).
    #[serde(default, deserialize_with = "coerce::string_or_seq")]
    #[schemars(
        description = "Override container kinds for SRP detection. Defaults: ['impl','struct','class','mod']"
    )]
    pub container_kinds: Vec<String>,

    /// Override interface kinds.
    #[serde(default, deserialize_with = "coerce::string_or_seq")]
    #[schemars(
        description = "Override interface kinds for ISP/DIP detection. Defaults: ['trait','interface','protocol']"
    )]
    pub interface_kinds: Vec<String>,

    /// Cosine threshold for DRY/OCP clone detection (default 0.86).
    #[serde(default, deserialize_with = "coerce::lenient_opt_f32")]
    #[schemars(description = "Cosine threshold for DRY/OCP clone detection. Default 0.86.")]
    pub similarity_threshold: Option<f32>,

    /// Jaccard threshold over callee-sets for DRY/OCP (default 0.4).
    #[serde(default, deserialize_with = "coerce::lenient_opt_f32")]
    #[schemars(description = "Jaccard threshold over callee-sets for DRY/OCP. Default 0.4.")]
    pub jaccard_threshold: Option<f32>,

    /// Cosine threshold for SRP cohesion edges (default 0.7).
    #[serde(default, deserialize_with = "coerce::lenient_opt_f32")]
    #[schemars(
        description = "Cosine threshold for SRP within-container cohesion edges. Default 0.7."
    )]
    pub srp_cohesion_threshold: Option<f32>,

    /// Minimum method count for ISP to fire (default 6).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Minimum interface method count before ISP fires. Default 6.")]
    pub isp_min_methods: Option<usize>,

    /// Implementor counts as "under-using" at or below this fraction
    /// (default 0.33).
    #[serde(default, deserialize_with = "coerce::lenient_opt_f32")]
    #[schemars(
        description = "Implementor under-using cutoff (fraction of trait methods overlapped). Default 0.33."
    )]
    pub isp_max_overlap_fraction: Option<f32>,

    /// Skip candidate symbols shorter than this many lines (default 5).
    #[serde(default, deserialize_with = "coerce::lenient_opt_u32")]
    #[schemars(description = "Skip candidate symbols shorter than this many lines. Default 5.")]
    pub min_lines: Option<u32>,

    /// Maximum findings to return (default 50, capped at 500).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(description = "Maximum findings to return. Default 50, capped at 500.")]
    pub limit: Option<usize>,

    /// Hard cap on pairwise comparisons inside any single DRY/OCP bucket
    /// (default 100k).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(
        description = "Hard cap on pairwise comparisons inside any DRY/OCP bucket. Default 100k."
    )]
    pub max_pairs: Option<usize>,

    /// Maximum representative members included per finding component list.
    /// When a list exceeds this it's truncated and the remainder is
    /// reported as `*_omitted` (default 5).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(
        description = "Maximum representative members per component list. Larger arrays are truncated and reported via `*_omitted`. Default 5."
    )]
    pub representative_count: Option<usize>,

    /// Minimum file count before a Shotgun-Surgery finding fires (default 3).
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(
        description = "Minimum file count before a Shotgun-Surgery finding fires. Default 3."
    )]
    pub shotgun_min_sites: Option<usize>,

    /// Max callee-set Jaccard for Shotgun-Surgery. Above this the group is a
    /// real Type-4 clone, handled by `dry_ocp` instead (default 0.5).
    #[serde(default, deserialize_with = "coerce::lenient_opt_f32")]
    #[schemars(
        description = "Maximum callee-set Jaccard for ShotgunSurgery. Above this the group is treated as a Type-4 clone and handled by 'dry_ocp'. Default 0.5."
    )]
    pub shotgun_max_jaccard: Option<f32>,

    /// Shotgun-Surgery sites must span at least this many distinct package
    /// prefixes (default 2). Single-crate fan-out is typically intentional
    /// polymorphism, not a cross-layer smell.
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(
        description = "Minimum distinct packages a Shotgun-Surgery group must span. Default 2."
    )]
    pub shotgun_min_packages: Option<usize>,

    /// Whether to drop Shotgun-Surgery candidates with conventional method
    /// names (`new`, `default`, `fmt`, `clone`, `as_str`, `parse`,
    /// `main`, ...). Default `true`.
    #[serde(default, deserialize_with = "coerce::lenient_opt_bool")]
    #[schemars(
        description = "Skip Shotgun-Surgery groups whose name is universally conventional (new/default/fmt/clone/as_str/parse/main/etc.). Default true."
    )]
    pub shotgun_skip_conventional_names: Option<bool>,

    /// Minimum cross-package edges per direction before two packages
    /// count as mutually dependent (default 2). Filters phantom cycles
    /// from ambiguous symbol-name resolution.
    #[serde(default, deserialize_with = "coerce::lenient_opt_usize")]
    #[schemars(
        description = "CyclicDependency: minimum distinct cross-package edges per direction. Single-edge cycles are usually phantom name-resolution artefacts. Default 2."
    )]
    pub cyclic_min_edges_per_direction: Option<usize>,

    /// Whether the cycle detector skips edges whose source or target
    /// lives in a test / fixture path. Default `true`.
    #[serde(default, deserialize_with = "coerce::lenient_opt_bool")]
    #[schemars(
        description = "CyclicDependency: skip edges touching test/fixture paths. Sample data shouldn't drive the workspace dependency graph. Default true."
    )]
    pub cyclic_skip_test_paths: Option<bool>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Response from the `ministr_solid` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct SolidResponse {
    /// All findings across enabled principles.
    pub(crate) findings: Vec<SolidFinding>,
    /// Total findings (≤ `limit`).
    pub(crate) total: usize,
}

/// Parameters for the `ministr_bridge` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct BridgeParams {
    /// Optional search query to filter bridge links by binding key or symbol name.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Search query to filter by binding key or symbol name (case-insensitive substring match)"
    )]
    pub query: Option<String>,

    /// Optional bridge kind filter.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Filter by bridge kind: 'tauri_command', 'tauri_event', 'napi', 'wasm_bindgen', 'pyo3', 'http_route', 'ffi', 'cgo', 'jni', 'uni_ffi', 'grpc', 'flutter_channel', 'electron_ipc'"
    )]
    pub bridge_kind: Option<String>,

    /// Optional language filter.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(
        description = "Filter links involving this language (e.g. 'rust', 'typescript', 'javascript', 'python')"
    )]
    pub language: Option<String>,

    /// Optional file path filter.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Filter links where either endpoint is in this file path")]
    pub file_path: Option<String>,

    /// Optional linked-project label.
    #[serde(default, deserialize_with = "coerce::lenient_opt_string")]
    #[schemars(description = "Optional linked-project label. Omit for primary corpus.")]
    pub project: Option<String>,
}

/// Response from the `ministr_bridge` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct BridgeResponse {
    /// Matched bridge links.
    pub(crate) links: Vec<BridgeLinkSummary>,
    /// Total number of links returned.
    pub(crate) total: usize,
}

/// A single entry in the `ministr_projects` response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct ProjectEntry {
    /// Linked-project label, or `None` for the current/primary corpus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    /// Whether this entry is the session's primary corpus.
    pub(crate) is_current: bool,
}

/// Response from the `ministr_projects` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct ProjectsResponse {
    /// All available projects — the primary corpus plus any linked ones.
    pub(crate) projects: Vec<ProjectEntry>,
    /// Human-readable hint about how to use linked projects.
    pub(crate) hint: String,
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

/// Corpus-level statistics returned in the `ministr_toc` response header.
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

/// Response from the `ministr_toc` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub(crate) struct TocResponse {
    /// Corpus-level statistics for quick orientation.
    pub(crate) corpus_stats: CorpusStatsHeader,
    /// Registered corpus roots with per-directory metadata and language stats.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) roots: Vec<ministr_core::types::CorpusRoot>,
    /// Table of contents entries (metadata only, no text).
    pub(crate) entries: Vec<ministr_core::types::TocEntry>,
}

// -- Union output types for tools that return different response shapes --

/// Output data for `ministr_read`: either full section detail or an "already delivered" skip.
///
/// Used only for output schema generation via `schemars::JsonSchema`.
#[expect(dead_code)]
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub(crate) enum ReadOutputData {
    /// Section was already delivered and is unchanged.
    AlreadyDelivered(AlreadyDeliveredResponse),
    /// Full section detail (new or changed content).
    Detail(ministr_core::service::SectionDetail),
}

/// Output data for `ministr_fetch`.
///
/// Used for output schema generation via `schemars::JsonSchema`.
/// When invoked as an MCP Task, the result is delivered via `tasks/result`.
pub(crate) type FetchOutputData = ToolResponse<FetchResponse>;

/// Output data for `ministr_clone`.
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
