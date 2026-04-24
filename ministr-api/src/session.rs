//! Session management API types.
//!
//! Wire types for creating, querying, and destroying daemon-side sessions.
//! Sessions track delivered content, enabling deduplication, delta delivery,
//! and token budget management across MCP proxy reconnections.

use serde::{Deserialize, Serialize};

/// Request to create a new session for a corpus.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateSessionRequest {
    /// Maximum context budget in tokens (default: 100 000).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<usize>,
}

/// Response after creating a session.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateSessionResponse {
    /// Unique session identifier (use in subsequent query requests).
    pub session_id: String,
}

/// Budget status snapshot for a session.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionBudgetResponse {
    /// Pressure level: `"normal"`, `"elevated"`, or `"critical"`.
    pub pressure_level: String,
    /// Estimated tokens consumed by delivered content.
    pub tokens_used: usize,
    /// Estimated tokens remaining before budget pressure.
    pub tokens_remaining: usize,
    /// Utilization ratio (0.0–1.0).
    pub utilization: f64,
}

/// Prefetch cache metrics for a corpus.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PrefetchMetricsResponse {
    /// Total cache hits.
    pub hits: u64,
    /// Total cache misses.
    pub misses: u64,
    /// Hits from sequential locality strategy.
    pub sequential_hits: u64,
    /// Hits from topical similarity strategy.
    pub topical_hits: u64,
    /// Hits from structural proximity strategy.
    pub structural_hits: u64,
    /// Hits from cross-session analytics strategy.
    pub cross_session_hits: u64,
    /// Hits from survey expansion strategy.
    pub survey_expand_hits: u64,
    /// Hits from agent intent prediction strategy.
    pub agent_plan_hits: u64,
    /// Current number of entries in the prefetch cache.
    pub cache_size: usize,
    /// Maximum cache capacity.
    pub cache_capacity: usize,
}

/// Request to compress content items for budget-efficient eviction.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompressRequest {
    /// Content IDs (section or symbol) to generate compressed summaries for.
    pub content_ids: Vec<String>,
    /// Session the call belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A single compressed content item.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompressedItemApi {
    /// The original content ID that was compressed.
    pub original_id: String,
    /// The compressed summary text.
    pub summary: String,
    /// Token count of the original content.
    pub original_tokens: usize,
    /// Token count of the compressed summary.
    pub compressed_tokens: usize,
    /// Compression method used (e.g. `"extractive"`, `"symbol_stub"`).
    pub method: String,
}

/// Response from the compress endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompressResponse {
    /// Compressed summaries (one per successfully compressed content ID).
    pub summaries: Vec<CompressedItemApi>,
}

/// Request to signal content evicted from the agent's context window.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EvictRequest {
    /// Content IDs that have been dropped from the agent's context.
    pub content_ids: Vec<String>,
}

/// Response from the eviction endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EvictResponse {
    /// Content IDs that were successfully removed from session tracking.
    pub evicted: Vec<String>,
    /// Content IDs that were not found in the session.
    pub not_found: Vec<String>,
}
