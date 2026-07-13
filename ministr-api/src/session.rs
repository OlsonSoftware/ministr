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
    /// Caller-provided session id (gd5). When set, the daemon creates the
    /// session under THIS id instead of generating one — this lets the MCP
    /// proxy pick the id up front so it can build its backend and serve the
    /// handshake *before* the (backgrounded) session creation completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Response after creating a session.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateSessionResponse {
    /// Unique session identifier (use in subsequent query requests).
    pub session_id: String,
}

/// Budget status snapshot for a session.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionUsageResponse {
    /// Pressure level: `"normal"`, `"elevated"`, or `"critical"`.
    pub level: String,
    /// Estimated tokens consumed by delivered content.
    pub tokens_used: usize,
    /// Estimated tokens remaining before budget pressure.
    pub tokens_remaining: usize,
    /// Utilization ratio (0.0–1.0).
    pub utilization: f64,
}

/// Full per-session economics + budget snapshot for the all-sessions list
/// . The point-in-time form the desktop Sessions view renders,
/// aggregated across every corpus by `GET /api/v1/sessions`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionInfo {
    /// Session identifier.
    pub session_id: String,
    /// Corpus this session belongs to.
    pub corpus_id: String,
    /// Pressure level: `"normal"`, `"elevated"`, or `"critical"`.
    pub level: String,
    /// Estimated tokens consumed by delivered content.
    pub tokens_used: usize,
    /// Estimated tokens remaining before budget pressure.
    pub tokens_remaining: usize,
    /// Utilization ratio (0.0–1.0).
    pub utilization: f64,
    /// Distinct content ids delivered so far this session.
    pub delivered_count: usize,
    /// Current turn counter.
    pub current_turn: u32,
    /// Total delivery operations.
    pub total_deliveries: u64,
    /// Cumulative tokens delivered (gross, pre-savings).
    pub cumulative_tokens_delivered: u64,
    /// Tokens saved by dedup + eviction + compression combined.
    pub total_tokens_saved: u64,
    /// Total eviction operations.
    pub total_evictions: u64,
    /// Total compression operations.
    pub total_compressions: u64,
    /// Tokens freed by eviction.
    pub cumulative_tokens_evicted: u64,
    /// Tokens freed by compression.
    pub cumulative_tokens_compressed: u64,
    /// Deliveries that changed since last seen (delta updates).
    pub delta_updates: u64,
    /// Deliveries short-circuited by dedup.
    pub dedup_hits: u64,
    /// `total_tokens_saved / cumulative_tokens_delivered` (0.0 when none).
    pub compression_ratio: f64,
    /// Effective context window in tokens (env-driven).
    pub context_window_tokens: usize,
    /// Fractional pressure threshold.
    pub pressure_threshold: f64,
    /// Fractional critical threshold.
    pub critical_threshold: f64,
    /// Parent session id when created on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// MCP `clientInfo.name` captured at initialize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
}

/// Response for `GET /api/v1/sessions` — every active session across all
/// corpora.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListSessionsResponse {
    /// Active sessions across all registered corpora.
    pub sessions: Vec<SessionInfo>,
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
    /// Hits for definitions warmed by `ministr_symbols`.
    #[serde(default)]
    pub symbol_search_hits: u64,
    /// Hits for definitions warmed by `ministr_references`.
    #[serde(default)]
    pub reference_follow_hits: u64,
    /// Total speculative entries issued.
    #[serde(default)]
    pub prefetches_issued: u64,
    /// Entries evicted or invalidated without a hit.
    #[serde(default)]
    pub prefetches_never_consumed: u64,
    /// `prefetches_never_consumed / prefetches_issued` (0 when none issued).
    #[serde(default)]
    pub waste_rate: f64,
    /// UTF-8 bytes served from warm entries.
    #[serde(default)]
    pub bytes_saved: u64,
    /// Tokens served from warm entries.
    #[serde(default)]
    pub tokens_saved: u64,
    /// Conservative modeled latency avoided.
    #[serde(default)]
    pub latency_saved_ms: u64,
    #[serde(default)]
    pub sequential_issued: u64,
    #[serde(default)]
    pub topical_issued: u64,
    #[serde(default)]
    pub structural_issued: u64,
    #[serde(default)]
    pub cross_session_issued: u64,
    #[serde(default)]
    pub survey_expand_issued: u64,
    #[serde(default)]
    pub agent_plan_issued: u64,
    #[serde(default)]
    pub symbol_search_issued: u64,
    #[serde(default)]
    pub reference_follow_issued: u64,
    /// Current number of entries in the prefetch cache.
    pub cache_size: usize,
    /// Maximum cache capacity.
    pub cache_capacity: usize,
}

/// Request to compress content items for budget-efficient eviction.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompressRequest {
    /// Content IDs (section or symbol) to generate compressed summaries for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_ids: Vec<String>,
    /// Exact corpus/content/resolution deliveries to compress.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identities: Vec<crate::metadata::DeliveryIdentity>,
    /// Session the call belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A single compressed content item.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompressedItemApi {
    /// Exact compressed delivery. Absent in responses from older daemons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<crate::metadata::DeliveryIdentity>,
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
pub struct DropRequest {
    /// Content IDs that have been dropped from the agent's context.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_ids: Vec<String>,
    /// Exact corpus/content/resolution deliveries to make eligible again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identities: Vec<crate::metadata::DeliveryIdentity>,
}

/// Response from the eviction endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DropResponse {
    /// Content IDs that were successfully removed from session tracking.
    pub dropped: Vec<String>,
    /// Content IDs that were not found in the session.
    pub not_found: Vec<String>,
    /// Exact identities successfully removed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped_identities: Vec<crate::metadata::DeliveryIdentity>,
    /// Exact identities not present in the session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_found_identities: Vec<crate::metadata::DeliveryIdentity>,
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;

    #[test]
    fn legacy_compress_payloads_default_typed_identity_fields() {
        let request: CompressRequest = serde_json::from_value(serde_json::json!({
            "content_ids": ["doc.md#section"],
            "session_id": "s1"
        }))
        .unwrap();
        assert!(request.identities.is_empty());

        let item: CompressedItemApi = serde_json::from_value(serde_json::json!({
            "original_id": "doc.md#section",
            "summary": "short",
            "original_tokens": 12,
            "compressed_tokens": 2,
            "method": "extractive"
        }))
        .unwrap();
        assert!(item.identity.is_none());
    }

    #[test]
    fn exact_compression_identity_round_trips_without_string_concatenation() {
        let identity = crate::metadata::DeliveryIdentity {
            corpus_id: "tenant::corpus|one".into(),
            content_id: "file.rs#item::with|delimiters".into(),
            resolution: "section/full:v2".into(),
        };
        let request = CompressRequest {
            content_ids: Vec::new(),
            identities: vec![identity.clone()],
            session_id: Some("session".into()),
        };
        let round_trip: CompressRequest =
            serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
        assert_eq!(round_trip.identities, vec![identity]);
    }
}
