//! MCP server implementation for iris.
//!
//! Implements the rmcp `ServerHandler` trait with `#[tool]` macro-based
//! tool registration. The server exposes iris tools (`iris_survey`,
//! `iris_read`, `iris_extract`) over the MCP protocol.
//!
//! Every tool response includes a `budget_status` object with the current
//! token budget state. Survey and read responses are deduplicated against
//! the session shadow to avoid re-delivering content the agent already has.

use std::sync::Arc;

use rmcp::ServerHandler;
use rmcp::model::{
    CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::schemars;
use rmcp::tool;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::{Instrument, debug, info_span, warn};

use iris_core::service::{QueryError, QueryService, SurveyResult};
use iris_core::session::{
    BudgetConfig, BudgetStatus, BudgetTracker, EvictionPolicy, Session, SessionId,
};
use iris_core::token::count_tokens;
use iris_core::types::{ContentId, Resolution};

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

/// Response when a section was already delivered and hasn't changed.
#[derive(Debug, Serialize)]
struct AlreadyDeliveredResponse {
    /// The section that was already delivered.
    section_id: String,
    /// Indicates this content was already delivered in this session.
    already_delivered: bool,
    /// The turn when it was originally delivered.
    delivered_at_turn: u32,
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

#[tool(tool_box)]
impl ServerHandler for IrisServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "iris".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "iris is a context cache controller for LLM agents. Use iris_survey to \
                 search for relevant content, iris_read to retrieve full section text, \
                 and iris_extract to get atomic claims from a section."
                    .to_string(),
            ),
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
                    drop(session);

                    let response = ToolResponse {
                        data: SurveyResponse {
                            results: filtered,
                            deduplicated_count,
                        },
                        budget_status,
                    };
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
    /// the number of claims available for extraction. If the section
    /// was already delivered and hasn't changed, returns a short
    /// "already delivered" message instead.
    #[tool(
        name = "iris_read",
        description = "Read the full text of a section by its hierarchical ID. Returns content with heading path and available claims count. Skips re-delivery of unchanged content."
    )]
    async fn read(&self, #[tool(aggr)] params: ReadParams) -> Result<CallToolResult, rmcp::Error> {
        let span = info_span!("iris_read", section_id = %params.section_id);

        async {
            debug!(section_id = %params.section_id, "iris_read request");

            match self.service.read_section(&params.section_id).await {
                Ok(detail) => {
                    let current_hash = content_hash(&detail.text);
                    let content_id = ContentId(params.section_id.clone());

                    // Check deduplication against session shadow
                    let session = self.session.lock().await;
                    let already_delivered = session.is_delivered(&content_id);
                    let has_changed = session.has_changed(&content_id, &current_hash);
                    let delivered_turn = session
                        .get_delivered(&content_id)
                        .map(|item| item.turn_delivered);
                    drop(session);

                    if already_delivered && !has_changed {
                        debug!(
                            section_id = %params.section_id,
                            "iris_read dedup: already delivered, unchanged"
                        );
                        let budget_status = self.budget.lock().await.budget_status();
                        let response = ToolResponse {
                            data: AlreadyDeliveredResponse {
                                section_id: params.section_id,
                                already_delivered: true,
                                delivered_at_turn: delivered_turn.unwrap_or(0),
                            },
                            budget_status,
                        };
                        let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| {
                            format!("{{\"error\": \"serialization failed: {e}\"}}")
                        });
                        return Ok(CallToolResult::success(vec![Content::text(json)]));
                    }

                    debug!(
                        section_id = %params.section_id,
                        claims_available = detail.claims_available,
                        re_delivery = already_delivered && has_changed,
                        "iris_read success"
                    );

                    // Record delivery in session and budget
                    let token_count = count_tokens(&detail.text);
                    let mut session = self.session.lock().await;
                    let mut budget = self.budget.lock().await;
                    let turn = session.current_turn() + 1;
                    session.record_delivery(
                        &content_id,
                        Resolution::Section,
                        token_count,
                        turn,
                        current_hash,
                    );
                    budget.record_tokens(&params.section_id, token_count);
                    let budget_status = budget.budget_status();
                    drop(budget);
                    drop(session);

                    let response = ToolResponse {
                        data: detail,
                        budget_status,
                    };
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

                    let response = ToolResponse {
                        data: ExtractResponse { claims },
                        budget_status,
                    };
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
        }
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
    async fn read_returns_already_delivered_for_unchanged_content() {
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

        // Second read — should return already_delivered
        let result2 = server
            .read(ReadParams {
                section_id: "docs/auth.md#tokens".to_string(),
            })
            .await
            .unwrap();
        let text2 = extract_text(&result2.content);
        let parsed2: serde_json::Value = serde_json::from_str(text2).unwrap();
        assert_eq!(
            parsed2["already_delivered"],
            serde_json::Value::Bool(true),
            "second read should indicate already_delivered"
        );
        assert!(
            parsed2["budget_status"].is_object(),
            "dedup response should include budget_status"
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
}
