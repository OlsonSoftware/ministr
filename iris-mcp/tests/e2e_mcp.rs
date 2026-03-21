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
use iris_core::storage::{SqliteStorage, Storage};
use iris_core::types::{Claim, ClaimId, ContentId, DocumentTree, Section, SectionId};
use iris_mcp::server::IrisServer;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, ClientInfo, Content, Implementation,
    PaginatedRequestParam,
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
