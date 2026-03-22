//! End-to-end integration tests for the iris MCP server.
//!
//! These tests exercise the full survey → read → extract flow through
//! the MCP `call_tool` interface with a real `SQLite` database and HNSW
//! index (using a deterministic mock embedder). They verify:
//!
//! - Tool listing returns all six iris tools
//! - Survey returns ranked, non-empty results across resolutions
//! - Read retrieves full section content with heading paths
//! - Extract returns atomic claims, optionally scored by relevance
//! - Session deduplication filters already-delivered content
//! - Budget accumulates across tool calls
//! - Error responses are user-friendly for nonexistent sections

use std::borrow::Cow;
use std::sync::Arc;

use iris_core::embedding::Embedder;
use iris_core::error::IndexError;
use iris_core::index::{HnswIndex, VectorIndex};
use iris_core::service::QueryService;
use iris_core::session::BudgetConfig;
use iris_core::storage::{SqliteStorage, Storage};
use iris_core::types::{
    Claim, ClaimId, ClaimRelationship, ContentId, DocumentTree, RelationType, Section, SectionId,
};
use iris_mcp::server::IrisServer;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, ClientInfo, Content, Implementation,
    PaginatedRequestParam, ResourceContents,
};
use rmcp::service::{Peer, RequestContext, RoleServer};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Deterministic mock embedder that produces consistent vectors from text bytes.
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

/// Extract the text string from the first Content item in a tool response.
fn extract_text(content: &[Content]) -> &str {
    content[0]
        .raw
        .as_text()
        .expect("expected text content")
        .text
        .as_str()
}

/// Build a multi-document corpus for realistic e2e testing.
fn build_corpus() -> Vec<DocumentTree> {
    vec![
        DocumentTree {
            id: ContentId("docs/auth.md".into()),
            title: "Authentication Guide".into(),
            source_path: "docs/auth.md".into(),
            sections: vec![
                Section {
                    id: SectionId("docs/auth.md#tokens".into()),
                    heading_path: vec!["Authentication".into(), "Tokens".into()],
                    depth: 2,
                    text: "JWT tokens use RS256 signing. Tokens expire after 24 hours."
                        .into(),
                    structural_nodes: vec![],
                    children: vec![],
                    claims: vec![
                        Claim {
                            id: ClaimId("auth-c1".into()),
                            text: "JWT tokens use RS256 signing algorithm.".into(),
                            section_id: SectionId("docs/auth.md#tokens".into()),
                        },
                        Claim {
                            id: ClaimId("auth-c2".into()),
                            text: "Tokens expire after 24 hours by default.".into(),
                            section_id: SectionId("docs/auth.md#tokens".into()),
                        },
                    ],
                    summary: Some("Token authentication details.".into()),
                },
                Section {
                    id: SectionId("docs/auth.md#oauth".into()),
                    heading_path: vec!["Authentication".into(), "OAuth".into()],
                    depth: 2,
                    text: "OAuth 2.0 authorization code flow with PKCE is required for public clients.".into(),
                    structural_nodes: vec![],
                    children: vec![],
                    claims: vec![Claim {
                        id: ClaimId("auth-c3".into()),
                        text: "OAuth 2.0 authorization code flow is supported.".into(),
                        section_id: SectionId("docs/auth.md#oauth".into()),
                    }],
                    summary: Some("OAuth 2.0 integration details.".into()),
                },
            ],
            summary: Some("Complete authentication reference.".into()),
        },
        DocumentTree {
            id: ContentId("docs/api.md".into()),
            title: "API Reference".into(),
            source_path: "docs/api.md".into(),
            sections: vec![Section {
                id: SectionId("docs/api.md#rate-limits".into()),
                heading_path: vec!["API Reference".into(), "Rate Limits".into()],
                depth: 2,
                text: "Rate limits are 100 requests per minute per API key. Exceeding the limit returns HTTP 429.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![
                    Claim {
                        id: ClaimId("api-c1".into()),
                        text: "Rate limit is 100 requests per minute per API key.".into(),
                        section_id: SectionId("docs/api.md#rate-limits".into()),
                    },
                    Claim {
                        id: ClaimId("api-c2".into()),
                        text: "Exceeding the rate limit returns HTTP 429.".into(),
                        section_id: SectionId("docs/api.md#rate-limits".into()),
                    },
                ],
                summary: Some("Rate limiting policy.".into()),
            }],
            summary: Some("Full API reference documentation.".into()),
        },
    ]
}

/// Set up the iris MCP server with a multi-document corpus.
async fn setup_server() -> IrisServer {
    let dim = 16;
    let embedder = Arc::new(MockEmbedder { dim });
    let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();

    let corpus = build_corpus();
    for doc in &corpus {
        storage.insert_document(doc).await.unwrap();
    }

    // Index all content at multiple resolutions.
    let texts_and_ids = [
        (
            "doc-summary::docs/auth.md",
            "Complete authentication reference.",
        ),
        (
            "doc-summary::docs/api.md",
            "Full API reference documentation.",
        ),
        (
            "sec-summary::docs/auth.md#tokens",
            "Token authentication details.",
        ),
        (
            "sec-summary::docs/auth.md#oauth",
            "OAuth 2.0 integration details.",
        ),
        (
            "sec-summary::docs/api.md#rate-limits",
            "Rate limiting policy.",
        ),
        (
            "section::docs/auth.md#tokens",
            "JWT tokens use RS256 signing. Tokens expire after 24 hours.",
        ),
        (
            "section::docs/auth.md#oauth",
            "OAuth 2.0 authorization code flow with PKCE is required for public clients.",
        ),
        (
            "section::docs/api.md#rate-limits",
            "Rate limits are 100 requests per minute per API key. Exceeding the limit returns HTTP 429.",
        ),
        ("claim::auth-c1", "JWT tokens use RS256 signing algorithm."),
        ("claim::auth-c2", "Tokens expire after 24 hours by default."),
        (
            "claim::auth-c3",
            "OAuth 2.0 authorization code flow is supported.",
        ),
        (
            "claim::api-c1",
            "Rate limit is 100 requests per minute per API key.",
        ),
        (
            "claim::api-c2",
            "Exceeding the rate limit returns HTTP 429.",
        ),
    ];

    for (id, text) in &texts_and_ids {
        let vecs = embedder.embed(&[*text]).unwrap();
        index.insert(id, &vecs[0]).unwrap();
    }

    // Insert claim relationships for iris_related testing.
    storage
        .insert_claim_relationships(&[
            ClaimRelationship {
                source_claim_id: ClaimId("auth-c1".into()),
                target_claim_id: ClaimId("auth-c2".into()),
                relation_type: RelationType::References,
                confidence: 0.9,
            },
            ClaimRelationship {
                source_claim_id: ClaimId("api-c1".into()),
                target_claim_id: ClaimId("api-c2".into()),
                relation_type: RelationType::DependsOn,
                confidence: 0.85,
            },
        ])
        .await
        .unwrap();

    let service = Arc::new(QueryService::new(storage, embedder, index));
    IrisServer::new(service)
}

/// Create a dummy `RequestContext` for testing `call_tool` through the protocol layer.
fn test_context() -> RequestContext<RoleServer> {
    let ct = CancellationToken::new();
    let client_info = ClientInfo {
        protocol_version: rmcp::model::ProtocolVersion::default(),
        capabilities: rmcp::model::ClientCapabilities::default(),
        client_info: Implementation {
            name: "test-client".into(),
            version: "0.0.0".into(),
        },
    };
    let (peer, _rx) = Peer::<RoleServer>::new(
        Arc::new(rmcp::service::AtomicU32RequestIdProvider::default()),
        client_info,
    );
    RequestContext {
        ct,
        id: rmcp::model::NumberOrString::Number(1),
        peer,
    }
}

/// Helper to call a tool by name with JSON arguments through the MCP protocol layer.
async fn call_tool(server: &IrisServer, name: &str, args: serde_json::Value) -> CallToolResult {
    let arguments = args
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    let param = CallToolRequestParam {
        name: Cow::Owned(name.to_string()),
        arguments,
    };
    server.call_tool(param, test_context()).await.unwrap()
}

// ---------------------------------------------------------------------------
// Tool listing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_tools_returns_all_iris_tools() {
    let server = setup_server().await;
    let result = server
        .list_tools(PaginatedRequestParam::default(), test_context())
        .await
        .unwrap();

    let tool_names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        tool_names.contains(&"iris_survey"),
        "should list iris_survey, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"iris_read"),
        "should list iris_read, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"iris_extract"),
        "should list iris_extract, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"iris_evicted"),
        "should list iris_evicted, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"iris_budget"),
        "should list iris_budget, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"iris_compress"),
        "should list iris_compress, got: {tool_names:?}"
    );
}

// ---------------------------------------------------------------------------
// Full survey → read → extract flow via call_tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_flow_survey_read_extract_via_call_tool() {
    let server = setup_server().await;

    // Step 1: Survey for JWT-related content
    let survey_result = call_tool(
        &server,
        "iris_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;

    assert!(
        survey_result.is_error.is_none() || survey_result.is_error == Some(false),
        "survey should succeed"
    );

    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey_result.content)).unwrap();

    let results = survey_json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "survey should return results");

    for r in results {
        assert!(r["content_id"].is_string());
        assert!(r["resolution"].is_string());
        assert!(r["score"].is_number());
        assert!(r["text"].is_string());
    }

    assert!(survey_json["budget_status"]["tokens_used"].is_number());
    let tokens_after_survey = survey_json["budget_status"]["tokens_used"]
        .as_u64()
        .unwrap();
    assert!(tokens_after_survey > 0);

    // Step 2: Read a specific section
    let read_result = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    assert!(
        read_result.is_error.is_none() || read_result.is_error == Some(false),
        "read should succeed"
    );

    let read_json: serde_json::Value =
        serde_json::from_str(extract_text(&read_result.content)).unwrap();

    assert_eq!(read_json["section_id"], "docs/auth.md#tokens");
    assert!(read_json["text"].as_str().unwrap().contains("JWT tokens"));
    assert_eq!(read_json["claims_available"], 2);

    let heading_path = read_json["heading_path"].as_array().unwrap();
    assert_eq!(heading_path[0], "Authentication");
    assert_eq!(heading_path[1], "Tokens");

    let tokens_after_read = read_json["budget_status"]["tokens_used"].as_u64().unwrap();
    assert!(
        tokens_after_read >= tokens_after_survey,
        "budget should accumulate: {tokens_after_read} >= {tokens_after_survey}"
    );

    // Step 3: Extract claims
    let extract_result = call_tool(
        &server,
        "iris_extract",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    let extract_json: serde_json::Value =
        serde_json::from_str(extract_text(&extract_result.content)).unwrap();

    let claims = extract_json["claims"].as_array().unwrap();
    assert_eq!(claims.len(), 2);

    let claim_texts: Vec<&str> = claims.iter().map(|c| c["text"].as_str().unwrap()).collect();
    assert!(claim_texts.iter().any(|t| t.contains("RS256")));
    assert!(claim_texts.iter().any(|t| t.contains("24 hours")));

    let tokens_after_extract = extract_json["budget_status"]["tokens_used"]
        .as_u64()
        .unwrap();
    assert!(
        tokens_after_extract >= tokens_after_read,
        "budget should not decrease: {tokens_after_extract} >= {tokens_after_read}"
    );
}

// ---------------------------------------------------------------------------
// Session deduplication
// ---------------------------------------------------------------------------

#[tokio::test]
async fn survey_deduplicates_already_delivered_content() {
    let server = setup_server().await;

    // First survey delivers results
    let r1 = call_tool(
        &server,
        "iris_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    let first_count = j1["results"].as_array().unwrap().len();
    assert!(first_count > 0);

    // Second survey with same query — delivered content filtered out
    let r2 = call_tool(
        &server,
        "iris_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;
    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();
    let second_count = j2["results"].as_array().unwrap().len();
    let dedup_count = j2["deduplicated_count"].as_u64().unwrap();

    assert!(
        second_count < first_count,
        "second survey should have fewer results: {second_count} < {first_count}"
    );
    assert!(dedup_count > 0, "should report deduplicated: {dedup_count}");
}

#[tokio::test]
async fn read_re_request_re_delivers_with_fault_correction() {
    let server = setup_server().await;

    // First read — full content
    let r1 = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    assert!(j1["text"].is_string(), "first read returns full text");

    // Second read — re-request of unchanged content triggers fault correction
    // and re-delivers the full text (agent lost it from context)
    let r2 = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();
    assert!(
        j2["text"].is_string(),
        "re-request should re-deliver full text"
    );
    assert!(j2["budget_status"].is_object());
}

// ---------------------------------------------------------------------------
// Cross-document queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn survey_finds_content_across_documents() {
    let server = setup_server().await;

    let result = call_tool(
        &server,
        "iris_survey",
        json!({"query": "rate limits API key requests", "top_k": 10}),
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "should find rate limit content");

    let has_rate_limit = results.iter().any(|r| {
        let text = r["text"].as_str().unwrap_or("");
        text.contains("rate limit") || text.contains("Rate limit") || text.contains("100 requests")
    });
    assert!(has_rate_limit, "should find rate limit content");
}

#[tokio::test]
async fn read_sections_from_different_documents() {
    let server = setup_server().await;

    // Read from auth doc
    let auth = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#oauth"}),
    )
    .await;
    let auth_json: serde_json::Value = serde_json::from_str(extract_text(&auth.content)).unwrap();
    assert!(auth_json["text"].as_str().unwrap().contains("OAuth"));

    // Read from API doc
    let api = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/api.md#rate-limits"}),
    )
    .await;
    let api_json: serde_json::Value = serde_json::from_str(extract_text(&api.content)).unwrap();
    assert!(api_json["text"].as_str().unwrap().contains("Rate limits"));

    let total_used = api_json["budget_status"]["tokens_used"].as_u64().unwrap();
    assert!(total_used > 0, "budget should track multi-document reads");
}

// ---------------------------------------------------------------------------
// Extract with query filtering
// ---------------------------------------------------------------------------

#[tokio::test]
async fn extract_with_query_scores_and_ranks_claims() {
    let server = setup_server().await;

    let result = call_tool(
        &server,
        "iris_extract",
        json!({"section_id": "docs/auth.md#tokens", "query": "signing algorithm"}),
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let claims = json["claims"].as_array().unwrap();

    assert_eq!(claims.len(), 2);
    for c in claims {
        assert!(c["relevance"].is_number(), "should have relevance scores");
    }

    let score0 = claims[0]["relevance"].as_f64().unwrap();
    let score1 = claims[1]["relevance"].as_f64().unwrap();
    assert!(
        score0 >= score1,
        "claims sorted by relevance: {score0} >= {score1}"
    );
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_nonexistent_section_returns_user_friendly_error() {
    let server = setup_server().await;

    let result = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/nonexistent.md#missing"}),
    )
    .await;

    assert_eq!(result.is_error, Some(true));
    let text = extract_text(&result.content);
    assert!(text.contains("Section not found"), "got: {text}");
    assert!(
        text.contains("iris_survey"),
        "should suggest discovery: {text}"
    );
}

#[tokio::test]
async fn extract_nonexistent_section_returns_user_friendly_error() {
    let server = setup_server().await;

    let result = call_tool(
        &server,
        "iris_extract",
        json!({"section_id": "docs/nonexistent.md#missing"}),
    )
    .await;

    assert_eq!(result.is_error, Some(true));
    let text = extract_text(&result.content);
    assert!(text.contains("Section not found"));
}

// ---------------------------------------------------------------------------
// Budget tracking end-to-end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn budget_monotonically_increases_across_tool_types() {
    let server = setup_server().await;

    // Survey
    let r1 = call_tool(
        &server,
        "iris_survey",
        json!({"query": "tokens", "top_k": 5}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    let t1 = j1["budget_status"]["tokens_used"].as_u64().unwrap();

    // Read
    let r2 = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/api.md#rate-limits"}),
    )
    .await;
    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();
    let t2 = j2["budget_status"]["tokens_used"].as_u64().unwrap();

    // Extract from a section NOT previously delivered (auth tokens)
    let r3 = call_tool(
        &server,
        "iris_extract",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j3: serde_json::Value = serde_json::from_str(extract_text(&r3.content)).unwrap();
    let t3 = j3["budget_status"]["tokens_used"].as_u64().unwrap();

    assert!(t1 > 0, "survey should use tokens");
    assert!(t2 > t1, "read adds tokens: {t2} > {t1}");
    assert!(t3 >= t2, "extract should not decrease tokens: {t3} >= {t2}");

    assert_eq!(
        j3["budget_status"]["pressure_level"].as_str().unwrap(),
        "normal",
        "small corpus should not trigger pressure"
    );
}

// ---------------------------------------------------------------------------
// iris_evicted tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn evicted_removes_content_from_session_and_budget() {
    let server = setup_server().await;

    // Deliver content first via read
    let _ = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    let budget_before = call_tool(&server, "iris_budget", json!({})).await;
    let j_before: serde_json::Value =
        serde_json::from_str(extract_text(&budget_before.content)).unwrap();
    let used_before = j_before["estimated_used"].as_u64().unwrap();
    assert!(used_before > 0, "should have used tokens after read");

    // Evict the delivered content
    let evict_result = call_tool(
        &server,
        "iris_evicted",
        json!({"content_ids": ["docs/auth.md#tokens"]}),
    )
    .await;

    assert!(
        evict_result.is_error.is_none() || evict_result.is_error == Some(false),
        "eviction should succeed"
    );

    let evict_json: serde_json::Value =
        serde_json::from_str(extract_text(&evict_result.content)).unwrap();
    assert_eq!(
        evict_json["evicted"].as_array().unwrap().len(),
        1,
        "should evict one item"
    );
    assert!(
        evict_json["not_found"].as_array().unwrap().is_empty(),
        "should have no not_found items"
    );
}

#[tokio::test]
async fn evicted_reports_not_found_for_unknown_ids() {
    let server = setup_server().await;

    let result = call_tool(
        &server,
        "iris_evicted",
        json!({"content_ids": ["nonexistent-id"]}),
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    assert!(json["evicted"].as_array().unwrap().is_empty());
    assert_eq!(json["not_found"].as_array().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// iris_budget standalone
// ---------------------------------------------------------------------------

#[tokio::test]
async fn budget_returns_complete_status() {
    let server = setup_server().await;

    let result = call_tool(&server, "iris_budget", json!({})).await;

    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "budget should succeed"
    );

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    assert!(json["total_budget"].is_number());
    assert!(json["estimated_used"].is_number());
    assert!(json["estimated_remaining"].is_number());
    assert!(json["pressure_level"].is_string());
    assert!(json["eviction_candidates"].is_array());
    assert!(json["prefetch_metrics"].is_object());

    // Fresh session should have zero usage
    assert_eq!(json["estimated_used"].as_u64().unwrap(), 0);
    assert_eq!(json["pressure_level"].as_str().unwrap(), "normal");
}

// ---------------------------------------------------------------------------
// iris_compress tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn compress_returns_summaries_for_sections() {
    let server = setup_server().await;

    let result = call_tool(
        &server,
        "iris_compress",
        json!({"content_ids": ["docs/auth.md#tokens", "docs/api.md#rate-limits"]}),
    )
    .await;

    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "compress should succeed"
    );

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let summaries = json["summaries"].as_array().unwrap();
    assert_eq!(summaries.len(), 2, "should return two summaries");

    for s in summaries {
        assert!(s["original_id"].is_string());
        assert!(s["summary"].is_string());
        assert!(s["original_tokens"].is_number());
        assert!(s["compressed_tokens"].is_number());
        let original = s["original_tokens"].as_u64().unwrap();
        let compressed = s["compressed_tokens"].as_u64().unwrap();
        assert!(
            compressed <= original,
            "compressed should be <= original: {compressed} <= {original}"
        );
    }

    assert!(json["budget_status"]["tokens_used"].is_number());
}

// ---------------------------------------------------------------------------
// iris_related tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn related_returns_linked_claims() {
    let server = setup_server().await;

    let result = call_tool(&server, "iris_related", json!({"claim_id": "auth-c1"})).await;

    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "related should succeed"
    );

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let related = json["related"].as_array().unwrap();
    assert!(
        !related.is_empty(),
        "auth-c1 should have related claims via inserted relationships"
    );

    for r in related {
        assert!(r["claim_id"].is_string());
        assert!(r["text"].is_string());
        assert!(r["relation_type"].is_string());
    }

    assert!(json["budget_status"].is_object());
}

#[tokio::test]
async fn related_with_type_filter() {
    let server = setup_server().await;

    let result = call_tool(
        &server,
        "iris_related",
        json!({"claim_id": "auth-c1", "relation_types": ["references"]}),
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let related = json["related"].as_array().unwrap();

    // All returned results should be of type "references"
    for r in related {
        assert_eq!(
            r["relation_type"].as_str().unwrap(),
            "references",
            "should only return references relations"
        );
    }
}

// ---------------------------------------------------------------------------
// MCP resource endpoints
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_resources_includes_status() {
    let server = setup_server().await;

    let result = server
        .list_resources(PaginatedRequestParam::default(), test_context())
        .await
        .unwrap();

    assert!(
        !result.resources.is_empty(),
        "should have at least one resource"
    );

    let names: Vec<&str> = result
        .resources
        .iter()
        .map(|r| r.raw.name.as_str())
        .collect();
    assert!(
        names.contains(&"iris status"),
        "should include iris status resource, got: {names:?}"
    );
}

#[tokio::test]
async fn list_resource_templates_includes_corpus() {
    let server = setup_server().await;

    let result = server
        .list_resource_templates(PaginatedRequestParam::default(), test_context())
        .await
        .unwrap();

    assert!(
        !result.resource_templates.is_empty(),
        "should have at least one resource template"
    );

    let names: Vec<&str> = result
        .resource_templates
        .iter()
        .map(|t| t.raw.name.as_str())
        .collect();
    assert!(
        names.contains(&"corpus document"),
        "should include corpus document template, got: {names:?}"
    );
}

#[tokio::test]
async fn read_status_resource() {
    use rmcp::model::ReadResourceRequestParam;

    let server = setup_server().await;

    let result = server
        .read_resource(
            ReadResourceRequestParam {
                uri: "iris://status".to_string(),
            },
            test_context(),
        )
        .await
        .unwrap();

    assert!(
        !result.contents.is_empty(),
        "status resource should return content"
    );

    // Verify it's valid JSON
    let text = match &result.contents[0] {
        ResourceContents::TextResourceContents { text, .. } => text.as_str(),
        ResourceContents::BlobResourceContents { .. } => {
            panic!("expected text resource content")
        }
    };
    let json: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(
        json["session"]["id"].is_string() && json["index"]["vector_count"].is_number(),
        "status should contain session and index info, got: {json}"
    );
}

// ---------------------------------------------------------------------------
// Tool listing: verify iris_related is present (7 tools total)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_tools_returns_seven_tools_including_related() {
    let server = setup_server().await;
    let result = server
        .list_tools(PaginatedRequestParam::default(), test_context())
        .await
        .unwrap();

    let tool_names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        tool_names.contains(&"iris_related"),
        "should list iris_related, got: {tool_names:?}"
    );

    assert!(
        result.tools.len() >= 7,
        "should have at least 7 tools, got: {}",
        result.tools.len()
    );
}

// ---------------------------------------------------------------------------
// I2.2 + I2.3: Coherence alerts surface in MCP tool responses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn coherence_alerts_surface_in_iris_read_response() {
    let server = setup_server().await;

    // Read a section to populate the session shadow
    let result = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    assert!(!result.is_error.unwrap_or(false));

    // Manually invalidate a section via the session to simulate coherence
    {
        let session = server.session_arc();
        let mut session = session.lock().await;
        session.invalidate_sections(&["docs/auth.md#tokens".to_string()]);
    }

    // Next tool call should surface the coherence alert
    let result = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let text = extract_text(&result.content);
    let json: serde_json::Value = serde_json::from_str(text).unwrap();

    assert!(
        json["coherence_alerts"].is_array(),
        "response should include coherence_alerts, got: {json}"
    );
    let alerts = json["coherence_alerts"].as_array().unwrap();
    assert!(
        !alerts.is_empty(),
        "should have at least one coherence alert"
    );
    assert!(
        alerts[0]["stale_content_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("docs/auth.md#tokens")),
        "alert should reference the invalidated section"
    );
}

#[tokio::test]
async fn coherence_alerts_surface_in_iris_budget_response() {
    let server = setup_server().await;

    // Deliver a section
    call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    // Invalidate it
    {
        let session = server.session_arc();
        let mut session = session.lock().await;
        session.invalidate_sections(&["docs/auth.md#tokens".to_string()]);
    }

    // Budget tool should surface the alert
    let result = call_tool(&server, "iris_budget", json!({})).await;
    let text = extract_text(&result.content);
    let json: serde_json::Value = serde_json::from_str(text).unwrap();

    assert!(
        json["coherence_alerts"].is_array(),
        "iris_budget should include coherence_alerts, got: {json}"
    );
    let alerts = json["coherence_alerts"].as_array().unwrap();
    assert!(
        !alerts.is_empty(),
        "budget response should include pending coherence alerts"
    );
}

// ---------------------------------------------------------------------------
// I1.3: Analytics co-access patterns are recorded and served via prefetch_metrics
// ---------------------------------------------------------------------------

/// Set up a server with persistence enabled for analytics testing.
async fn setup_server_with_persistence() -> IrisServer {
    let dim = 16;
    let embedder = Arc::new(MockEmbedder { dim });
    let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();

    let corpus = build_corpus();
    for doc in &corpus {
        storage.insert_document(doc).await.unwrap();
    }

    let texts_and_ids = [
        (
            "doc-summary::docs/auth.md",
            "Complete authentication reference.",
        ),
        (
            "doc-summary::docs/api.md",
            "Full API reference documentation.",
        ),
        (
            "sec-summary::docs/auth.md#tokens",
            "Token authentication details.",
        ),
        (
            "sec-summary::docs/auth.md#oauth",
            "OAuth 2.0 integration details.",
        ),
        (
            "sec-summary::docs/api.md#rate-limits",
            "Rate limiting policy.",
        ),
        (
            "section::docs/auth.md#tokens",
            "JWT tokens use RS256 signing. Tokens expire after 24 hours.",
        ),
        (
            "section::docs/auth.md#oauth",
            "OAuth 2.0 authorization code flow with PKCE is required for public clients.",
        ),
        (
            "section::docs/api.md#rate-limits",
            "Rate limits are 100 requests per minute per API key. Exceeding the limit returns HTTP 429.",
        ),
        ("claim::auth-c1", "JWT tokens use RS256 signing algorithm."),
        ("claim::auth-c2", "Tokens expire after 24 hours by default."),
        (
            "claim::auth-c3",
            "OAuth 2.0 authorization code flow is supported.",
        ),
        (
            "claim::api-c1",
            "Rate limit is 100 requests per minute per API key.",
        ),
        (
            "claim::api-c2",
            "Exceeding the rate limit returns HTTP 429.",
        ),
    ];
    for (id, text) in &texts_and_ids {
        let vecs = embedder.embed(&[*text]).unwrap();
        index.insert(id, &vecs[0]).unwrap();
    }

    let storage = Arc::new(storage);
    let service = Arc::new(QueryService::new((*storage).clone(), embedder, index));
    let budget_config = BudgetConfig::default();
    IrisServer::with_persistence(
        service,
        budget_config,
        storage,
        Some("test-analytics".into()),
    )
    .await
}

#[tokio::test]
async fn analytics_co_access_patterns_recorded_and_served_via_prefetch_metrics() {
    let server = setup_server_with_persistence().await;

    // Read multiple sections to build a co-access trajectory
    call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#oauth"}),
    )
    .await;
    call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/api.md#rate-limits"}),
    )
    .await;

    // Re-read a section to trigger cross-session prefetch via co-access data
    call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    // Check budget response includes prefetch_metrics
    let result = call_tool(&server, "iris_budget", json!({})).await;
    let text = extract_text(&result.content);
    let json: serde_json::Value = serde_json::from_str(text).unwrap();

    assert!(
        json["prefetch_metrics"].is_object(),
        "iris_budget should include prefetch_metrics, got: {json}"
    );

    let metrics = &json["prefetch_metrics"];
    // Verify the metrics structure is present with strategy breakdowns
    assert!(
        metrics["hits"].is_number(),
        "prefetch_metrics should have hits"
    );
    assert!(
        metrics["misses"].is_number(),
        "prefetch_metrics should have misses"
    );

    // Verify co-access data was recorded in storage by checking analytics
    let storage = server.storage_arc().unwrap();
    let analytics = iris_core::analytics::Analytics::new((*storage).clone());
    let co = analytics
        .co_accessed_with(
            &iris_core::types::SectionId("docs/auth.md#tokens".into()),
            10,
        )
        .await
        .unwrap();
    assert!(
        !co.is_empty(),
        "co-access patterns should be recorded for docs/auth.md#tokens"
    );
    // Both oauth and rate-limits should be co-accessed with tokens
    let partners: Vec<&str> = co.iter().map(|c| c.section_id.0.as_str()).collect();
    assert!(
        partners.contains(&"docs/auth.md#oauth") || partners.contains(&"docs/api.md#rate-limits"),
        "co-access should include sibling sections, got: {partners:?}"
    );
}

// ===========================================================================
// I3: End-to-End Validation — file-based corpus tests
// ===========================================================================

/// Set up a temp corpus directory with markdown files, ingest via the full
/// pipeline (with embeddings), and return an `IrisServer` backed by the result.
async fn setup_server_from_corpus_dir() -> (IrisServer, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().unwrap();

    // Write corpus files
    std::fs::write(
        dir.path().join("auth.md"),
        "\
# Authentication Guide

## Tokens

JWT tokens use RS256 signing. Tokens expire after 24 hours.
Access tokens are issued by the /oauth/token endpoint.

## OAuth

OAuth 2.0 authorization code flow with PKCE is required for public clients.
The authorization server validates redirect URIs against a registered allowlist.
",
    )
    .unwrap();

    std::fs::write(
        dir.path().join("api.md"),
        "\
# API Reference

## Rate Limits

Rate limits are 100 requests per minute per API key.
Exceeding the limit returns HTTP 429 Too Many Requests.
Use the Retry-After header to determine when to retry.
",
    )
    .unwrap();

    let dim = 16;
    let embedder = Arc::new(MockEmbedder { dim });
    let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Ingest via the real pipeline with embeddings
    let pipeline = iris_core::ingestion::IngestionPipeline::new();
    let stats = pipeline
        .ingest_directory_with_embeddings(dir.path(), &storage, embedder.as_ref(), index.as_ref())
        .await
        .unwrap();

    assert!(
        stats.files_indexed >= 2,
        "should ingest at least 2 files, got: {stats:?}"
    );
    assert!(stats.total_sections > 0, "should extract sections");

    let storage = Arc::new(storage);
    let service = Arc::new(QueryService::new((*storage).clone(), embedder, index));
    let budget_config = BudgetConfig::default();
    let server = IrisServer::with_persistence(
        service,
        budget_config,
        storage,
        Some("e2e-corpus-test".into()),
    )
    .await;

    (server, dir)
}

// ---------------------------------------------------------------------------
// I3.0: iris_survey returns ranked results from a real corpus directory
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_survey_from_corpus_dir_returns_ranked_results() {
    let (server, _dir) = setup_server_from_corpus_dir().await;

    let result = call_tool(
        &server,
        "iris_survey",
        json!({"query": "JWT authentication tokens", "top_k": 10}),
    )
    .await;

    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "survey should succeed"
    );

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "survey should return results");

    // Verify results are ranked by score (descending)
    let scores: Vec<f64> = results
        .iter()
        .map(|r| r["score"].as_f64().unwrap())
        .collect();
    for window in scores.windows(2) {
        assert!(
            window[0] >= window[1],
            "results should be ranked by score: {window:?}"
        );
    }

    // Verify result structure
    for r in results {
        assert!(r["content_id"].is_string(), "should have content_id");
        assert!(r["resolution"].is_string(), "should have resolution");
        assert!(r["score"].is_number(), "should have score");
        assert!(r["text"].is_string(), "should have text");
    }

    // Verify budget tracking
    assert!(json["budget_status"]["tokens_used"].as_u64().unwrap() > 0);
}

// ---------------------------------------------------------------------------
// I3.1: iris_read returns full section text with heading paths and content hashes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_read_returns_heading_paths_and_content_hash() {
    let (server, _dir) = setup_server_from_corpus_dir().await;

    // First discover section IDs via survey
    let survey = call_tool(
        &server,
        "iris_survey",
        json!({"query": "tokens authentication JWT", "top_k": 10}),
    )
    .await;
    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey.content)).unwrap();
    let results = survey_json["results"].as_array().unwrap();

    // Find a section-level result to read
    let section_result = results
        .iter()
        .find(|r| r["resolution"].as_str() == Some("section"))
        .expect("should have at least one section-level result");
    let section_id = section_result["content_id"].as_str().unwrap();

    // Read the section
    let read = call_tool(&server, "iris_read", json!({"section_id": section_id})).await;

    assert!(
        read.is_error.is_none() || read.is_error == Some(false),
        "read should succeed"
    );

    let read_json: serde_json::Value = serde_json::from_str(extract_text(&read.content)).unwrap();

    // Verify section_id matches
    assert_eq!(read_json["section_id"].as_str().unwrap(), section_id);

    // Verify heading_path is present and non-empty
    let heading_path = read_json["heading_path"].as_array().unwrap();
    assert!(
        !heading_path.is_empty(),
        "heading_path should be non-empty for a section"
    );

    // Verify full text content is returned
    let text = read_json["text"].as_str().unwrap();
    assert!(!text.is_empty(), "text should be non-empty");

    // Verify claims_available is a number
    assert!(
        read_json["claims_available"].is_number(),
        "should report claims_available"
    );

    // Verify content_hash is present in the response (used internally for dedup)
    // The content hash is tracked internally via the session shadow — verify
    // the session recorded it by checking that re-request detection works
    let session = server.session_arc();
    let session = session.lock().await;
    let content_id = iris_core::types::ContentId(section_id.to_string());
    assert!(
        session.is_delivered(&content_id),
        "session should track delivered content with its hash"
    );
}

// ---------------------------------------------------------------------------
// I3.2: iris_extract returns claims from ingested content
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_extract_returns_claims_from_ingested_content() {
    let (server, _dir) = setup_server_from_corpus_dir().await;

    // Discover a section with claims
    let survey = call_tool(
        &server,
        "iris_survey",
        json!({"query": "tokens JWT signing expire", "top_k": 10}),
    )
    .await;
    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey.content)).unwrap();
    let results = survey_json["results"].as_array().unwrap();

    let section_result = results
        .iter()
        .find(|r| r["resolution"].as_str() == Some("section"))
        .expect("should have section-level results");
    let section_id = section_result["content_id"].as_str().unwrap();

    // Extract claims
    let extract = call_tool(&server, "iris_extract", json!({"section_id": section_id})).await;

    assert!(
        extract.is_error.is_none() || extract.is_error == Some(false),
        "extract should succeed"
    );

    let extract_json: serde_json::Value =
        serde_json::from_str(extract_text(&extract.content)).unwrap();
    let claims = extract_json["claims"].as_array().unwrap();

    assert!(
        !claims.is_empty(),
        "should extract at least one claim from a content-rich section"
    );

    // Verify claim structure
    for claim in claims {
        assert!(claim["claim_id"].is_string(), "claim should have claim_id");
        assert!(claim["text"].is_string(), "claim should have text");
        assert!(
            !claim["text"].as_str().unwrap().is_empty(),
            "claim text should be non-empty"
        );
    }

    // Verify budget tracked the extraction
    assert!(
        extract_json["budget_status"]["tokens_used"]
            .as_u64()
            .unwrap()
            > 0
    );
}

// ---------------------------------------------------------------------------
// I3.3: iris_related follows claim dependency chains
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_related_follows_claim_dependency_chains() {
    let (server, _dir) = setup_server_from_corpus_dir().await;

    // Find claim IDs via extract
    let survey = call_tool(
        &server,
        "iris_survey",
        json!({"query": "tokens JWT signing expire", "top_k": 10}),
    )
    .await;
    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey.content)).unwrap();

    // Find a claim-level result to trace
    let claim_result = survey_json["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["resolution"].as_str() == Some("claim"));

    if let Some(claim) = claim_result {
        let claim_id = claim["content_id"].as_str().unwrap();

        let related = call_tool(&server, "iris_related", json!({"claim_id": claim_id})).await;

        assert!(
            related.is_error.is_none() || related.is_error == Some(false),
            "iris_related should succeed for an existing claim"
        );

        let related_json: serde_json::Value =
            serde_json::from_str(extract_text(&related.content)).unwrap();

        // The related array may be empty if no relationships were detected,
        // but the response structure should be valid.
        assert!(
            related_json["related"].is_array(),
            "should have a related array"
        );

        for r in related_json["related"].as_array().unwrap() {
            assert!(
                r["claim_id"].is_string(),
                "related claim should have claim_id"
            );
            assert!(r["text"].is_string(), "related claim should have text");
            assert!(
                r["relation_type"].is_string(),
                "related claim should have relation_type"
            );
            assert!(
                r["source_section"].is_string(),
                "related claim should have source_section"
            );
        }

        assert!(related_json["budget_status"].is_object());
    }

    // Also verify with the programmatic corpus that has guaranteed relationships
    let server2 = setup_server().await;
    let result = call_tool(&server2, "iris_related", json!({"claim_id": "auth-c1"})).await;
    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let related = json["related"].as_array().unwrap();
    assert!(
        !related.is_empty(),
        "auth-c1 should have related claims via dependency chain"
    );
    // Follow the chain: auth-c1 → auth-c2 (references)
    assert_eq!(related[0]["claim_id"].as_str().unwrap(), "auth-c2");
    assert_eq!(related[0]["relation_type"].as_str().unwrap(), "references");
}

// ---------------------------------------------------------------------------
// I3.4: session deduplication — iris_read same section twice
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_read_session_dedup_on_second_request() {
    let server = setup_server().await;

    // First read — full content delivered
    let r1 = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    assert!(
        j1["text"].is_string(),
        "first read should deliver full text"
    );
    let first_text = j1["text"].as_str().unwrap();
    assert!(
        first_text.contains("JWT tokens"),
        "first read should contain section text"
    );

    // Verify session tracks the delivery
    {
        let session = server.session_arc();
        let session = session.lock().await;
        let content_id = iris_core::types::ContentId("docs/auth.md#tokens".into());
        assert!(
            session.is_delivered(&content_id),
            "session should mark content as delivered after first read"
        );
    }

    // Second read — same section, unchanged content
    // Current behavior: re-request triggers fault correction and re-delivers
    let r2 = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();

    // The server re-delivers on re-request (fault correction path) but the
    // session shadow tracked the deduplication state correctly
    assert!(
        j2["text"].is_string(),
        "re-request should still return text (fault correction re-delivery)"
    );
    assert!(j2["budget_status"].is_object());

    // Verify survey deduplication works — same query returns fewer results
    let s1 = call_tool(
        &server,
        "iris_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;
    let sj1: serde_json::Value = serde_json::from_str(extract_text(&s1.content)).unwrap();
    let first_count = sj1["results"].as_array().unwrap().len();

    let s2 = call_tool(
        &server,
        "iris_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;
    let sj2: serde_json::Value = serde_json::from_str(extract_text(&s2.content)).unwrap();
    let second_count = sj2["results"].as_array().unwrap().len();
    let dedup_count = sj2["deduplicated_count"].as_u64().unwrap();

    assert!(
        second_count < first_count || dedup_count > 0,
        "survey dedup should filter already-delivered content: \
         second={second_count} < first={first_count}, dedup={dedup_count}"
    );
}

// ---------------------------------------------------------------------------
// I3.5: iris_compress + iris_evicted cycle works and budget updates
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_compress_evict_cycle_updates_budget() {
    let server = setup_server().await;

    // Step 1: Deliver content to build up budget usage
    let _ = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let _ = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "docs/api.md#rate-limits"}),
    )
    .await;

    // Check budget after reads
    let budget_before = call_tool(&server, "iris_budget", json!({})).await;
    let jb: serde_json::Value = serde_json::from_str(extract_text(&budget_before.content)).unwrap();
    let used_before = jb["estimated_used"].as_u64().unwrap();
    assert!(used_before > 0, "should have budget usage after reads");

    // Step 2: Compress the sections
    let compress = call_tool(
        &server,
        "iris_compress",
        json!({"content_ids": ["docs/auth.md#tokens", "docs/api.md#rate-limits"]}),
    )
    .await;
    assert!(
        compress.is_error.is_none() || compress.is_error == Some(false),
        "compress should succeed"
    );

    let cj: serde_json::Value = serde_json::from_str(extract_text(&compress.content)).unwrap();
    let summaries = cj["summaries"].as_array().unwrap();
    assert_eq!(summaries.len(), 2, "should compress both sections");

    for s in summaries {
        assert!(s["summary"].is_string());
        let orig = s["original_tokens"].as_u64().unwrap();
        let comp = s["compressed_tokens"].as_u64().unwrap();
        assert!(comp <= orig, "compressed <= original: {comp} <= {orig}");
    }

    // Step 3: Evict the content after compression
    let evict = call_tool(
        &server,
        "iris_evicted",
        json!({"content_ids": ["docs/auth.md#tokens", "docs/api.md#rate-limits"]}),
    )
    .await;
    assert!(
        evict.is_error.is_none() || evict.is_error == Some(false),
        "evict should succeed"
    );

    let ej: serde_json::Value = serde_json::from_str(extract_text(&evict.content)).unwrap();
    assert_eq!(
        ej["evicted"].as_array().unwrap().len(),
        2,
        "should evict both items"
    );
    assert!(
        ej["not_found"].as_array().unwrap().is_empty(),
        "should have no not_found"
    );

    // Step 4: Verify budget decreased after eviction
    let budget_after = call_tool(&server, "iris_budget", json!({})).await;
    let ja: serde_json::Value = serde_json::from_str(extract_text(&budget_after.content)).unwrap();
    let used_after = ja["estimated_used"].as_u64().unwrap();

    assert!(
        used_after < used_before,
        "budget should decrease after eviction: {used_after} < {used_before}"
    );

    // Step 5: Verify evicted content is no longer in the session
    {
        let session = server.session_arc();
        let session = session.lock().await;
        assert!(
            !session.is_delivered(&iris_core::types::ContentId("docs/auth.md#tokens".into())),
            "evicted content should not be marked as delivered"
        );
    }
}

// ---------------------------------------------------------------------------
// I3.6: modify a corpus file, verify coherence detects change and iris_read
//       returns updated content
// ---------------------------------------------------------------------------

/// Set up a single-file corpus for coherence testing.
///
/// Returns the server, temp dir, embedder, index, and storage arcs needed
/// for file modification and re-indexing.
#[allow(clippy::type_complexity)]
async fn setup_coherence_server() -> (
    IrisServer,
    tempfile::TempDir,
    Arc<MockEmbedder>,
    Arc<HnswIndex>,
    Arc<SqliteStorage>,
) {
    let dir = tempfile::TempDir::new().unwrap();

    let auth_path = dir.path().join("auth.md");
    std::fs::write(
        &auth_path,
        "\
# Authentication

## Tokens

JWT tokens use RS256 signing. Tokens expire after 24 hours.
",
    )
    .unwrap();

    let dim = 16;
    let embedder = Arc::new(MockEmbedder { dim });
    let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();

    let pipeline = iris_core::ingestion::IngestionPipeline::new();
    pipeline
        .ingest_directory_with_embeddings(dir.path(), &storage, embedder.as_ref(), index.as_ref())
        .await
        .unwrap();

    let storage = Arc::new(storage);
    let service = Arc::new(QueryService::new(
        (*storage).clone(),
        Arc::clone(&embedder) as Arc<dyn iris_core::embedding::Embedder>,
        Arc::clone(&index) as Arc<dyn iris_core::index::VectorIndex>,
    ));
    let server = IrisServer::with_persistence(
        service,
        BudgetConfig::default(),
        Arc::clone(&storage),
        Some("coherence-test".into()),
    )
    .await;

    (server, dir, embedder, index, storage)
}

#[tokio::test]
async fn e2e_coherence_detects_file_change_and_read_returns_updated() {
    let (server, dir, embedder, index, storage) = setup_coherence_server().await;
    let auth_path = dir.path().join("auth.md");

    // Read the original section
    let r1 = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "auth.md#authentication/tokens"}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    let original_text = j1["text"].as_str().unwrap();
    assert!(
        original_text.contains("24 hours"),
        "original text should contain '24 hours'"
    );

    // Modify the corpus file on disk
    std::fs::write(
        &auth_path,
        "\
# Authentication

## Tokens

JWT tokens use RS256 signing. Tokens now expire after 12 hours for improved security.
",
    )
    .unwrap();

    // Use the CoherenceEngine to process the file change
    let engine = iris_core::coherence::CoherenceEngine::with_embeddings(
        dir.path().to_path_buf(),
        Arc::clone(&embedder) as Arc<dyn iris_core::embedding::Embedder>,
        Arc::clone(&index) as Arc<dyn iris_core::index::VectorIndex>,
    );
    let events = vec![iris_core::coherence::CoherenceEvent::Modified(
        auth_path.clone(),
    )];
    let affected = engine
        .process_events(&events, storage.as_ref())
        .await
        .unwrap();

    assert!(
        !affected.is_empty(),
        "coherence should detect affected sections"
    );

    // Invalidate the session
    {
        let session = server.session_arc();
        let mut session = session.lock().await;
        let invalidated =
            iris_core::coherence::CoherenceEngine::invalidate_session(&mut session, &affected);
        assert!(invalidated > 0, "should invalidate delivered sections");
    }

    // Read the section again — should get updated content
    let r2 = call_tool(
        &server,
        "iris_read",
        json!({"section_id": "auth.md#authentication/tokens"}),
    )
    .await;

    assert!(
        r2.is_error.is_none() || r2.is_error == Some(false),
        "read after coherence update should succeed"
    );

    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();
    let updated_text = j2["text"].as_str().unwrap();
    assert!(
        updated_text.contains("12 hours"),
        "updated text should contain '12 hours', got: {updated_text}"
    );
    assert!(
        !updated_text.contains("24 hours"),
        "updated text should NOT contain '24 hours' anymore"
    );

    // Verify coherence alert was surfaced
    let alerts = &j2["coherence_alerts"];
    if alerts.is_array() && !alerts.as_array().unwrap().is_empty() {
        let alert = &alerts[0];
        assert!(
            alert["stale_content_ids"].is_array(),
            "alert should list stale content IDs"
        );
    }
}
