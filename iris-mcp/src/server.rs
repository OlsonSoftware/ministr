//! MCP server implementation for iris.
//!
//! Implements the rmcp `ServerHandler` trait with `#[tool]` macro-based
//! tool registration. The server exposes iris tools (`iris_survey`,
//! `iris_read`, `iris_extract`, `iris_related`, `iris_evicted`,
//! `iris_budget`, `iris_compress`, `iris_toc`) over the MCP protocol.
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

use std::sync::Arc;

use rmcp::RoleServer;
use rmcp::ServerHandler;
use rmcp::model::ErrorData as McpError;
use rmcp::model::{
    CallToolResult, Content, Implementation, ListResourceTemplatesResult, ListResourcesResult,
    PaginatedRequestParam, ProtocolVersion, RawResource, RawResourceTemplate,
    ReadResourceRequestParam, ReadResourceResult, Resource, ResourceContents, ResourceTemplate,
    ServerCapabilities, ServerInfo,
};
use rmcp::schemars;
use rmcp::service::RequestContext;
use rmcp::tool;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::{Instrument, debug, info_span, warn};

use iris_core::analytics::Analytics;
use iris_core::service::{
    CompressedItem, QueryError, QueryService, RelatedClaimResult, SurveyResult,
};
use iris_core::session::delta::ContentDelta;
use iris_core::session::eviction::EvictionCandidate;
use iris_core::session::prefetch::PrefetchEngine;
use iris_core::session::{
    BudgetConfig, BudgetStatus, BudgetTracker, CoherenceAlert, EvictionPolicy, Session, SessionId,
};
use iris_core::storage::{SqliteStorage, Storage};
use iris_core::token::count_tokens;
use iris_core::types::{ContentId, RelationType, Resolution, SectionId, parent_section_id};

/// MCP server that exposes iris context-cache tools to LLM agents.
///
/// `IrisServer` adapts the [`QueryService`] to the MCP protocol.
/// It handles tool registration, request routing, and response formatting.
/// Tracks session state for deduplication and budget management.
#[derive(Clone)]
pub struct IrisServer {
    service: Arc<QueryService>,
    session: Arc<Mutex<Session>>,
    budget: Arc<Mutex<BudgetTracker>>,
    prefetch: Arc<Mutex<PrefetchEngine>>,
    storage: Option<Arc<SqliteStorage>>,
    analytics: Option<Arc<Analytics>>,
}

/// Tool response wrapper that includes budget status alongside the result data.
///
/// Every tool response is serialized as a JSON object with a `data` field
/// containing the tool-specific result and a `budget_status` field with
/// the current token budget snapshot.
#[derive(Debug, Serialize)]
struct ToolResponse<T: Serialize> {
    /// The tool-specific result data.
    #[serde(flatten)]
    data: T,
    /// Current budget status snapshot.
    budget_status: BudgetStatus,
    /// Pending coherence alerts (present when underlying content has changed).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    coherence_alerts: Vec<CoherenceAlert>,
}

/// Wrapper for survey responses that includes both results and dedup metadata.
#[derive(Debug, Serialize)]
struct SurveyResponse {
    /// The survey results after deduplication.
    results: Vec<SurveyResult>,
    /// Number of results filtered out by deduplication.
    deduplicated_count: usize,
}

/// Wrapper for extract responses.
#[derive(Debug, Serialize)]
struct ExtractResponse {
    /// The extracted claims.
    claims: Vec<iris_core::service::ClaimResult>,
}

/// Response when a section has already been delivered and is unchanged.
///
/// Returned instead of full text to avoid wasting context tokens on
/// content the agent already has.
#[derive(Debug, Serialize)]
struct AlreadyDeliveredResponse {
    /// The requested section ID.
    section_id: String,
    /// Always `"already_delivered"`.
    status: &'static str,
    /// Number of claims available for extraction.
    claims_available: usize,
}

/// Response when a previously-delivered section has changed.
///
/// Will be used when full delta delivery is enabled (requires storing
/// delivered text in the session, planned for P6 prefetch cache).
#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct DeltaResponse {
    /// The section that changed.
    section_id: String,
    /// Indicates this is a delta update.
    delta_update: bool,
    /// The content delta (added/removed/context lines).
    delta: ContentDelta,
    /// Token count of the new full text.
    new_token_count: usize,
}

/// Response from the `iris_evicted` tool.
#[derive(Debug, Serialize)]
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
#[derive(Debug, Serialize)]
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
#[derive(Debug, Serialize)]
struct RelatedResponse {
    /// Related claims with relationship metadata.
    related: Vec<RelatedClaimResult>,
}

/// Response from the `iris_compress` tool.
#[derive(Debug, Serialize)]
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
}

/// Corpus-level statistics returned in the `iris_toc` response header.
#[derive(Debug, Serialize)]
struct CorpusStatsHeader {
    /// Number of documents in the corpus.
    documents: usize,
    /// Number of sections across all documents.
    sections: usize,
    /// Number of claims across all sections.
    claims: usize,
}

/// Response from the `iris_toc` tool.
#[derive(Debug, Serialize)]
struct TocResponse {
    /// Corpus-level statistics for quick orientation.
    corpus_stats: CorpusStatsHeader,
    /// Table of contents entries (metadata only, no text).
    entries: Vec<iris_core::types::TocEntry>,
}

#[tool(tool_box)]
impl ServerHandler for IrisServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: "iris".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "iris is a context cache controller for LLM agents. Use iris_toc to get \
                 a structural overview of the indexed corpus, iris_survey to search for \
                 relevant content, iris_read to retrieve full section text, iris_extract \
                 to get atomic claims from a section, iris_related to follow dependency \
                 chains between claims, iris_budget to check context budget status and \
                 get eviction recommendations, iris_compress to generate compressed \
                 summaries of content you want to evict, and iris_evicted to signal when \
                 content has been dropped from your context window."
                    .to_string(),
            ),
        }
    }

    async fn list_resources(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![Resource::new(
                RawResource {
                    uri: "iris://status".to_string(),
                    name: "iris status".to_string(),
                    description: Some(
                        "Index statistics — vector count, dimension, session state, and budget"
                            .to_string(),
                    ),
                    mime_type: Some("application/json".to_string()),
                    size: None,
                },
                None,
            )],
            next_cursor: None,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: PaginatedRequestParam,
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
                },
                None,
            )],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;
        if uri == "iris://status" {
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
}

#[tool(tool_box)]
impl IrisServer {
    /// Search the corpus for sections relevant to your query.
    ///
    /// Returns ranked summaries with relevance scores across all resolution
    /// levels (document summaries, section text, atomic claims).
    /// Results that were already delivered in this session are filtered out.
    #[tool(
        name = "iris_survey",
        description = "Search the indexed corpus for sections relevant to a natural language query. Returns ranked summaries with relevance scores. Already-delivered content is filtered out."
    )]
    async fn survey(
        &self,
        #[tool(aggr)] params: SurveyParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        let top_k = params.top_k.unwrap_or(10);
        let span = info_span!("iris_survey", query_len = params.query.len(), top_k);

        async {
            debug!(query = %params.query, top_k, "iris_survey request");

            match self.service.survey(&params.query, top_k).await {
                Ok(results) => {
                    let original_count = results.len();

                    // Deduplicate against session shadow
                    let session = self.session.lock().await;
                    let filtered: Vec<SurveyResult> = results
                        .into_iter()
                        .filter(|r| !session.is_delivered(&ContentId(r.content_id.clone())))
                        .collect();
                    drop(session);

                    let deduplicated_count = original_count - filtered.len();

                    debug!(
                        result_count = filtered.len(),
                        deduplicated_count, "iris_survey success"
                    );

                    // Record delivered content in session and budget
                    let mut session = self.session.lock().await;
                    let mut budget = self.budget.lock().await;
                    let turn = session.current_turn() + 1;
                    for r in &filtered {
                        let token_count = count_tokens(&r.text);
                        let hash = content_hash(&r.text);
                        let resolution = parse_resolution(&r.resolution);
                        session.record_delivery(
                            &ContentId(r.content_id.clone()),
                            resolution,
                            token_count,
                            turn,
                            hash,
                        );
                        budget.record_tokens(&r.content_id, token_count);
                    }
                    let budget_status = budget.budget_status();
                    drop(budget);

                    // Survey-triggered prefetch: pre-warm parent sections of claim hits
                    let claim_section_ids: Vec<String> = filtered
                        .iter()
                        .filter(|r| r.resolution == "claim")
                        .filter_map(|r| parent_section_id(&r.content_id).map(String::from))
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .filter(|sid| !session.is_delivered(&ContentId(sid.clone())))
                        .collect();
                    drop(session);

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

                    self.persist_session().await;

                    let response = self
                        .build_response(
                            SurveyResponse {
                                results: filtered,
                                deduplicated_count,
                            },
                            budget_status,
                        )
                        .await;
                    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                        format!("{{\"error\": \"serialization failed: {e}\"}}")
                    });
                    Ok(CallToolResult::success(vec![Content::text(json)]))
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
        description = "Read the full text of a section by its hierarchical ID. Returns content with heading path and available claims count. Returns deltas for changed content and skips re-delivery of unchanged content."
    )]
    async fn read(&self, #[tool(aggr)] params: ReadParams) -> Result<CallToolResult, rmcp::Error> {
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
                    let session = self.session.lock().await;
                    let already_delivered = session.is_delivered(&content_id);
                    let has_changed = session.has_changed(&content_id, &current_hash);
                    let is_re_request = session.is_re_request(&content_id, &current_hash);
                    drop(session);

                    // Case 2: Already delivered and unchanged — skip re-delivery
                    if already_delivered && !has_changed {
                        debug!(
                            section_id = %params.section_id,
                            "iris_read: already delivered, skipping re-delivery"
                        );

                        // If agent re-requests content it should still have,
                        // treat as a fault-based eviction signal.
                        if is_re_request {
                            let mut budget = self.budget.lock().await;
                            budget.force_evict(&params.section_id);
                        }

                        let budget = self.budget.lock().await;
                        let budget_status = budget.budget_status();
                        drop(budget);

                        let skip = AlreadyDeliveredResponse {
                            section_id: params.section_id.clone(),
                            status: "already_delivered",
                            claims_available: detail.claims_available,
                        };
                        let response = self.build_response(skip, budget_status).await;
                        let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                            format!("{{\"error\": \"serialization failed: {e}\"}}")
                        });
                        return Ok(CallToolResult::success(vec![Content::text(json)]));
                    }

                    // Case 1: New content (or changed) — deliver full text
                    let budget_status = self
                        .record_section_delivery(&params.section_id, &detail.text, current_hash)
                        .await;
                    self.record_analytics_access(&params.section_id).await;
                    self.trigger_prefetch(&params.section_id).await;

                    let response = self.build_response(detail, budget_status).await;
                    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                        format!("{{\"error\": \"serialization failed: {e}\"}}")
                    });
                    Ok(CallToolResult::success(vec![Content::text(json)]))
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
        description = "Extract atomic claims from a section, optionally filtered by relevance to a query."
    )]
    async fn extract(
        &self,
        #[tool(aggr)] params: ExtractParams,
    ) -> Result<CallToolResult, rmcp::Error> {
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
                    let mut session = self.session.lock().await;
                    let mut budget = self.budget.lock().await;
                    let turn = session.current_turn() + 1;
                    for c in &claims {
                        let token_count = count_tokens(&c.text);
                        let hash = content_hash(&c.text);
                        session.record_delivery(
                            &ContentId(c.claim_id.clone()),
                            Resolution::Claim,
                            token_count,
                            turn,
                            hash,
                        );
                        budget.record_tokens(&c.claim_id, token_count);
                    }
                    let budget_status = budget.budget_status();
                    drop(budget);
                    drop(session);

                    self.persist_session().await;

                    let response = self
                        .build_response(ExtractResponse { claims }, budget_status)
                        .await;
                    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                        format!("{{\"error\": \"serialization failed: {e}\"}}")
                    });
                    Ok(CallToolResult::success(vec![Content::text(json)]))
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
        description = "Follow dependency chains between claims. Given a claim ID, returns related claims with relationship type (references, contradicts, depends_on, updates) and source section."
    )]
    async fn related(
        &self,
        #[tool(aggr)] params: RelatedParams,
    ) -> Result<CallToolResult, rmcp::Error> {
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
                    let mut session = self.session.lock().await;
                    let mut budget = self.budget.lock().await;
                    let turn = session.current_turn() + 1;
                    for r in &related {
                        let token_count = count_tokens(&r.text);
                        let hash = content_hash(&r.text);
                        session.record_delivery(
                            &ContentId(r.claim_id.clone()),
                            Resolution::Claim,
                            token_count,
                            turn,
                            hash,
                        );
                        budget.record_tokens(&r.claim_id, token_count);
                    }
                    let budget_status = budget.budget_status();
                    drop(budget);
                    drop(session);

                    self.persist_session().await;

                    let response = self
                        .build_response(RelatedResponse { related }, budget_status)
                        .await;
                    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                        format!("{{\"error\": \"serialization failed: {e}\"}}")
                    });
                    Ok(CallToolResult::success(vec![Content::text(json)]))
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
        description = "Signal that content IDs have been evicted from the agent's context window. Updates session tracking for accurate budget and deduplication."
    )]
    async fn evicted(
        &self,
        #[tool(aggr)] params: EvictedParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        let span = info_span!("iris_evicted", count = params.content_ids.len());

        async {
            debug!(content_ids = ?params.content_ids, "iris_evicted request");

            let mut evicted = Vec::new();
            let mut not_found = Vec::new();

            let mut session = self.session.lock().await;
            let mut budget = self.budget.lock().await;

            for id_str in &params.content_ids {
                let content_id = ContentId(id_str.clone());
                if session.remove_delivered(&content_id).is_some() {
                    budget.force_evict(id_str);
                    evicted.push(id_str.clone());
                } else {
                    not_found.push(id_str.clone());
                }
            }

            let budget_status = budget.budget_status();
            drop(budget);
            drop(session);

            self.persist_session().await;

            debug!(
                evicted_count = evicted.len(),
                not_found_count = not_found.len(),
                "iris_evicted complete"
            );

            let response = self
                .build_response(EvictedResponse { evicted, not_found }, budget_status)
                .await;
            let json = serde_json::to_string_pretty(&response)
                .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {e}\"}}"));
            Ok(CallToolResult::success(vec![Content::text(json)]))
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
        description = "Get the current context budget status: total budget, estimated usage, pressure level, and eviction recommendations. Call this to understand budget health."
    )]
    async fn budget(&self) -> Result<CallToolResult, rmcp::Error> {
        let span = info_span!("iris_budget");

        async {
            debug!("iris_budget request");

            let mut session = self.session.lock().await;
            let budget = self.budget.lock().await;
            let prefetch = self.prefetch.lock().await;

            let status = budget.budget_status();
            let candidates = budget.eviction_candidates(&session, 5);
            let prefetch_metrics = prefetch.metrics();
            let alerts = session.drain_alerts();

            drop(prefetch);
            drop(budget);
            drop(session);

            let pressure_str = match status.pressure_level {
                iris_core::session::PressureLevel::Normal => "normal",
                iris_core::session::PressureLevel::Elevated => "elevated",
                iris_core::session::PressureLevel::Critical => "critical",
            };

            debug!(
                pressure = pressure_str,
                used = status.tokens_used,
                remaining = status.tokens_remaining,
                candidate_count = candidates.len(),
                "iris_budget complete"
            );

            let response = BudgetResponse {
                total_budget: status.tokens_used + status.tokens_remaining,
                estimated_used: status.tokens_used,
                estimated_remaining: status.tokens_remaining,
                pressure_level: pressure_str.to_string(),
                eviction_candidates: candidates,
                prefetch_metrics,
                coherence_alerts: alerts,
            };
            let json = serde_json::to_string_pretty(&response)
                .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {e}\"}}"));
            Ok(CallToolResult::success(vec![Content::text(json)]))
        }
        .instrument(span)
        .await
    }

    /// Generate compressed summaries for content the agent wants to evict.
    ///
    /// For each content ID, returns a short extractive summary that preserves
    /// the gist while reducing token count by 60–80%. The agent can replace
    /// the full section with this summary to free budget.
    #[tool(
        name = "iris_compress",
        description = "Generate compressed summaries for sections the agent wants to evict from context. Returns short summaries preserving the gist, with original and compressed token counts."
    )]
    async fn compress(
        &self,
        #[tool(aggr)] params: CompressParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        let span = info_span!("iris_compress", count = params.content_ids.len());

        async {
            debug!(content_ids = ?params.content_ids, "iris_compress request");

            match self.service.compress_content(&params.content_ids).await {
                Ok(summaries) => {
                    debug!(summary_count = summaries.len(), "iris_compress success");

                    let budget = self.budget.lock().await;
                    let budget_status = budget.budget_status();
                    drop(budget);

                    let response = self
                        .build_response(CompressResponse { summaries }, budget_status)
                        .await;
                    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                        format!("{{\"error\": \"serialization failed: {e}\"}}")
                    });
                    Ok(CallToolResult::success(vec![Content::text(json)]))
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
        description = "Return a table of contents for the indexed corpus. Lists all documents and sections with metadata (heading path, depth, claim count, token count) but no text content. Optionally filter to a single document by ID."
    )]
    async fn toc(&self, #[tool(aggr)] params: TocParams) -> Result<CallToolResult, rmcp::Error> {
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

                    debug!(
                        total_documents,
                        total_sections, total_claims, "iris_toc success"
                    );

                    let budget = self.budget.lock().await;
                    let budget_status = budget.budget_status();
                    drop(budget);

                    let response = self
                        .build_response(
                            TocResponse {
                                corpus_stats: CorpusStatsHeader {
                                    documents: total_documents,
                                    sections: total_sections,
                                    claims: total_claims,
                                },
                                entries,
                            },
                            budget_status,
                        )
                        .await;
                    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                        format!("{{\"error\": \"serialization failed: {e}\"}}")
                    });
                    Ok(CallToolResult::success(vec![Content::text(json)]))
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
        let session = Session::new(
            SessionId::from(uuid_v4()),
            budget_config.max_context_tokens,
            EvictionPolicy::Fifo,
        );
        let budget = BudgetTracker::new(budget_config, EvictionPolicy::Fifo);
        Self {
            service,
            session: Arc::new(Mutex::new(session)),
            budget: Arc::new(Mutex::new(budget)),
            prefetch: Arc::new(Mutex::new(PrefetchEngine::with_default_capacity())),
            storage: None,
            analytics: None,
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
        let session_id = SessionId::from(sid);

        let session = match storage.load_session(&session_id).await {
            Ok(Some(restored)) => {
                debug!(
                    session_id = %session_id,
                    delivered_count = restored.delivered_count(),
                    "restored session from storage"
                );
                restored
            }
            _ => Session::new(
                session_id,
                budget_config.max_context_tokens,
                EvictionPolicy::Fifo,
            ),
        };

        let budget = BudgetTracker::new(budget_config, EvictionPolicy::Fifo);
        let analytics = Arc::new(Analytics::new((*storage).clone()));
        Self {
            service,
            session: Arc::new(Mutex::new(session)),
            budget: Arc::new(Mutex::new(budget)),
            prefetch: Arc::new(Mutex::new(PrefetchEngine::with_default_capacity())),
            storage: Some(storage),
            analytics: Some(analytics),
        }
    }

    /// Access the session `Arc` for external use (e.g. coherence task).
    #[must_use]
    pub fn session_arc(&self) -> Arc<Mutex<Session>> {
        Arc::clone(&self.session)
    }

    /// Access the storage `Arc`, if persistence is enabled.
    #[must_use]
    pub fn storage_arc(&self) -> Option<Arc<SqliteStorage>> {
        self.storage.clone()
    }

    /// Record a section delivery in the session shadow and budget tracker.
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
        let mut session = self.session.lock().await;
        let mut budget = self.budget.lock().await;
        let turn = session.current_turn() + 1;
        session.record_delivery(
            &content_id,
            Resolution::Section,
            token_count,
            turn,
            content_hash,
        );
        budget.record_tokens(section_id, token_count);
        let status = budget.budget_status();
        drop(budget);
        drop(session);
        self.persist_session().await;
        status
    }

    /// Build a tool response with budget status and any pending coherence alerts.
    async fn build_response<T: Serialize>(
        &self,
        data: T,
        budget_status: BudgetStatus,
    ) -> ToolResponse<T> {
        let mut session = self.session.lock().await;
        let alerts = session.drain_alerts();
        drop(session);

        ToolResponse {
            data,
            budget_status,
            coherence_alerts: alerts,
        }
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
            let session = self.session.lock().await;
            if let Err(e) = storage.save_session(&session).await {
                warn!(error = %e, "failed to persist session");
            }
            // Flush co-access patterns from trajectory
            if let Some(ref analytics) = self.analytics {
                let trajectory = session.trajectory();
                let section_ids: Vec<SectionId> = trajectory
                    .iter()
                    .map(|cid| SectionId(cid.0.clone()))
                    .collect();
                drop(session);
                if let Err(e) = analytics.record_co_accesses(&section_ids).await {
                    warn!(error = %e, "failed to record co-access patterns");
                }
            }
        }
    }

    /// Build the `iris://status` resource content.
    async fn read_status_resource(&self) -> Result<ReadResourceResult, McpError> {
        let index = self.service.index();
        let session = self.session.lock().await;
        let budget = self.budget.lock().await;

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
                "id": session.id.to_string(),
                "delivered_count": session.delivered_count(),
            },
            "budget": budget.budget_status(),
        });

        if let Some(stats) = analytics_stats {
            status["analytics"] = serde_json::json!({
                "total_accesses": stats.total_accesses,
                "unique_sections_accessed": stats.unique_sections_accessed,
                "co_access_pairs": stats.co_access_pairs,
            });
        }

        let text = serde_json::to_string_pretty(&status).unwrap_or_default();
        Ok(ReadResourceResult {
            contents: vec![ResourceContents::TextResourceContents {
                uri: "iris://status".to_string(),
                mime_type: Some("application/json".to_string()),
                text,
            }],
        })
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
        Ok(ReadResourceResult {
            contents: vec![ResourceContents::TextResourceContents {
                uri,
                mime_type: Some("application/json".to_string()),
                text,
            }],
        })
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iris_core::embedding::Embedder;
    use iris_core::error::IndexError;
    use iris_core::index::{HnswIndex, VectorIndex};
    use iris_core::storage::{SqliteStorage, Storage};
    use iris_core::types::{Claim, ClaimId, ContentId, DocumentTree, Section, SectionId};

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

    // --- Budget status tests ---

    #[tokio::test]
    async fn survey_response_includes_budget_status() {
        let server = setup_server().await;
        let params = SurveyParams {
            query: "JWT authentication tokens".to_string(),
            top_k: Some(5),
        };
        let result = server.survey(params).await.unwrap();

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
        let result = server.read(params).await.unwrap();

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
        let result = server.extract(params).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert!(parsed["budget_status"].is_object());
    }

    #[tokio::test]
    async fn budget_accumulates_across_tool_calls() {
        let server = setup_server().await;

        // First call — read a section
        let result1 = server
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
            .await
            .unwrap();
        let text1 = extract_text(&result1.content);
        let parsed1: serde_json::Value = serde_json::from_str(text1).unwrap();
        let used_after_read = parsed1["budget_status"]["tokens_used"].as_u64().unwrap();
        assert!(used_after_read > 0, "should track tokens after read");

        // Second call — extract claims
        let result2 = server
            .extract(ExtractParams {
                section_id: "docs/auth.md#tokens".to_string(),
                query: None,
            })
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
            .survey(SurveyParams {
                query: "JWT authentication tokens".to_string(),
                top_k: Some(10),
            })
            .await
            .unwrap();
        let text1 = extract_text(&result1.content);
        let parsed1: serde_json::Value = serde_json::from_str(text1).unwrap();
        let first_count = parsed1["results"].as_array().unwrap().len();
        assert!(first_count > 0, "first survey should return results");

        // Second survey with same query — should filter out delivered content
        let result2 = server
            .survey(SurveyParams {
                query: "JWT authentication tokens".to_string(),
                top_k: Some(10),
            })
            .await
            .unwrap();
        let text2 = extract_text(&result2.content);
        let parsed2: serde_json::Value = serde_json::from_str(text2).unwrap();
        let second_count = parsed2["results"].as_array().unwrap().len();
        let dedup_count = parsed2["deduplicated_count"].as_u64().unwrap();

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
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
            .await
            .unwrap();
        let text1 = extract_text(&result1.content);
        let parsed1: serde_json::Value = serde_json::from_str(text1).unwrap();
        assert!(
            parsed1["text"].is_string(),
            "first read should return full text"
        );

        // Second read — already delivered and unchanged, should skip re-delivery
        let result2 = server
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
            .await
            .unwrap();
        let text2 = extract_text(&result2.content);
        let parsed2: serde_json::Value = serde_json::from_str(text2).unwrap();
        assert_eq!(
            parsed2["status"], "already_delivered",
            "re-request should return already_delivered status"
        );
        assert!(
            parsed2["text"].is_null(),
            "re-request should not include full text"
        );
        assert!(
            parsed2["budget_status"].is_object(),
            "response should include budget_status"
        );
        assert!(
            parsed2["claims_available"].is_number(),
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
        let result = server.survey(params).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(parsed["results"].is_array(), "should have results array");
        assert!(
            parsed["deduplicated_count"].is_number(),
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
        let result = server.read(params).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["section_id"], "docs/auth.md#tokens");
        assert_eq!(parsed["claims_available"], 2);
        assert!(parsed["text"].as_str().unwrap().contains("JWT tokens"));
        assert!(parsed["budget_status"].is_object());
    }

    #[tokio::test]
    async fn read_not_found_returns_error() {
        let server = setup_server().await;
        let params = ReadParams {
            section_id: "nonexistent#section".to_string(),
        };
        let result = server.read(params).await.unwrap();

        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn extract_not_found_returns_error() {
        let server = setup_server().await;
        let params = ExtractParams {
            section_id: "nonexistent#section".to_string(),
            query: None,
        };
        let result = server.extract(params).await.unwrap();

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
        let result = server.read(params).await.unwrap();

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
        let result = server.extract(params).await.unwrap();

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
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
            .await
            .unwrap();

        // Evict it
        let result = server
            .evicted(EvictedParams {
                content_ids: vec!["docs/auth.md#tokens".to_string()],
            })
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["evicted"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["not_found"].as_array().unwrap().len(), 0);
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
            .evicted(EvictedParams {
                content_ids: vec!["nonexistent".to_string()],
            })
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["evicted"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["not_found"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn evicted_handles_mixed_known_and_unknown() {
        let server = setup_server().await;

        // Deliver content
        server
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
            .await
            .unwrap();

        // Evict a mix of known and unknown
        let result = server
            .evicted(EvictedParams {
                content_ids: vec!["docs/auth.md#tokens".to_string(), "nonexistent".to_string()],
            })
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["evicted"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["not_found"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn evicted_empty_list_is_noop() {
        let server = setup_server().await;

        let result = server
            .evicted(EvictedParams {
                content_ids: vec![],
            })
            .await
            .unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(parsed["evicted"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["not_found"].as_array().unwrap().len(), 0);
    }

    // --- Fault-based correction tests ---

    #[tokio::test]
    async fn re_request_corrects_window_estimate() {
        let server = setup_server().await;

        // First read — delivers and records in budget
        let result1 = server
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
            .await
            .unwrap();
        let text1 = extract_text(&result1.content);
        let parsed1: serde_json::Value = serde_json::from_str(text1).unwrap();
        let used_after_first = parsed1["budget_status"]["tokens_used"].as_u64().unwrap();
        assert!(used_after_first > 0, "first read should use tokens");

        // Second read (re-request) — triggers fault correction (force_evict)
        // and returns already_delivered without re-recording
        let result2 = server
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
            .await
            .unwrap();
        let text2 = extract_text(&result2.content);
        let parsed2: serde_json::Value = serde_json::from_str(text2).unwrap();
        assert_eq!(
            parsed2["status"], "already_delivered",
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
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
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
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
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
        let result = server.compress(params).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let summaries = parsed["summaries"].as_array().unwrap();
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
        let result = server.compress(params).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let summaries = parsed["summaries"].as_array().unwrap();
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
        let result = server.compress(params).await.unwrap();

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
        let result = server.compress(params).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let summaries = parsed["summaries"].as_array().unwrap();
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
        let result = server.compress(params).await.unwrap();

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();

        let summaries = parsed["summaries"].as_array().unwrap();
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
        let server = setup_server().await;
        let result = server
            .list_resources(PaginatedRequestParam::default(), make_test_context())
            .await
            .unwrap();

        assert_eq!(result.resources.len(), 1);
        assert_eq!(result.resources[0].uri, "iris://status");
        assert_eq!(result.resources[0].name, "iris status");
        assert!(result.resources[0].description.is_some());
        assert_eq!(
            result.resources[0].mime_type.as_deref(),
            Some("application/json")
        );
    }

    #[tokio::test]
    async fn list_resource_templates_returns_corpus_template() {
        let server = setup_server().await;
        let result = server
            .list_resource_templates(PaginatedRequestParam::default(), make_test_context())
            .await
            .unwrap();

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
        let server = setup_server().await;
        let result = server
            .read_resource(
                ReadResourceRequestParam {
                    uri: "iris://status".to_string(),
                },
                make_test_context(),
            )
            .await
            .unwrap();

        assert_eq!(result.contents.len(), 1);
    }

    #[tokio::test]
    async fn read_resource_dispatches_corpus_uri() {
        let server = setup_server().await;
        let result = server
            .read_resource(
                ReadResourceRequestParam {
                    uri: "iris://corpus/docs/auth.md".to_string(),
                },
                make_test_context(),
            )
            .await
            .unwrap();

        assert_eq!(result.contents.len(), 1);
    }

    #[tokio::test]
    async fn read_resource_unknown_uri_returns_error() {
        let server = setup_server().await;
        let result = server
            .read_resource(
                ReadResourceRequestParam {
                    uri: "iris://unknown".to_string(),
                },
                make_test_context(),
            )
            .await;

        assert!(result.is_err());
    }

    /// Create a minimal `RequestContext` for testing resource handlers.
    fn make_test_context() -> RequestContext<RoleServer> {
        use rmcp::model::{ClientInfo, RequestId};
        use rmcp::service::{AtomicU32RequestIdProvider, Peer};
        use tokio_util::sync::CancellationToken;

        let id_provider = Arc::new(AtomicU32RequestIdProvider::default());
        let (peer, _rx) = Peer::new(id_provider, ClientInfo::default());
        RequestContext {
            ct: CancellationToken::new(),
            id: RequestId::Number(1),
            peer,
        }
    }
}
