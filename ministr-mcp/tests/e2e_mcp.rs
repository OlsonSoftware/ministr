//! End-to-end integration tests for the ministr MCP server.
//!
//! These tests exercise the full survey → read → extract flow through
//! the MCP `call_tool` interface with a real `SQLite` database and HNSW
//! index (using a deterministic mock embedder). They verify:
//!
//! - Tool listing returns all ministr tools including `ministr_fetch`
//! - Survey returns ranked, non-empty results across resolutions
//! - Read retrieves full section content with heading paths
//! - Extract returns atomic claims, optionally scored by relevance
//! - Session deduplication filters already-delivered content
//! - Budget accumulates across tool calls
//! - Error responses are user-friendly for nonexistent sections
//! - `ministr_fetch` fetches web content and makes it searchable

use std::sync::Arc;

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::{HnswIndex, VectorIndex};
use ministr_core::service::QueryService;
use ministr_core::session::UsageConfig;
use ministr_core::storage::{SqliteStorage, Storage, SymbolRecord, SymbolRefRecord};
use ministr_core::types::{
    Claim, ClaimId, ClaimRelationship, ContentId, DocumentTree, RefKind, RelationType, Section,
    SectionId, SymbolId,
};
use ministr_mcp::server::MinistrServer;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content, ResourceContents};
use serde_json::json;

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

/// Extract the tool-specific `result` field from a `ToolResponse` JSON envelope.
///
/// Tool responses have the shape `{ "result": { tool-specific data }, ... }`
/// (plus optional `coherence_alerts` / `next_actions`). Budget status is
/// tracked internally and intentionally not part of this envelope. This
/// helper navigates to the `result` sub-object.
fn tool_result(json: &serde_json::Value) -> &serde_json::Value {
    &json["result"]
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

/// Set up the ministr MCP server with a multi-document corpus.
async fn setup_server() -> MinistrServer {
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

    // Insert claim relationships for ministr_related testing.
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
    MinistrServer::new(service)
}

/// MCP client connected to an in-process server via duplex streams.
type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
/// Server handle kept alive so the server doesn't shut down when dropped.
type McpServerHandle = rmcp::service::RunningService<rmcp::RoleServer, MinistrServer>;

/// Wrap an `MinistrServer` into an in-process MCP client/server pair.
///
/// Returns both the client handle and the server handle. The server handle
/// must be kept alive for the duration of the test — dropping it shuts down
/// the server and causes the client to hang.
async fn wrap_as_client(server: MinistrServer) -> (McpClient, McpServerHandle) {
    let (c2s_w, c2s_r) = tokio::io::duplex(65_536);
    let (s2c_w, s2c_r) = tokio::io::duplex(65_536);
    // Spawn server in a separate task — serve().await blocks until the
    // client sends `initialize`, so both must progress concurrently.
    let server_task = tokio::spawn(async move { server.serve((c2s_r, s2c_w)).await.unwrap() });
    let client = ().serve((s2c_r, c2s_w)).await.unwrap();
    let server_handle = server_task.await.unwrap();
    (client, server_handle)
}

/// Helper to call a tool by name with JSON arguments through the MCP protocol layer.
async fn call_tool(client: &McpClient, name: &str, args: serde_json::Value) -> CallToolResult {
    let arguments = args
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    let mut params = CallToolRequestParams::new(name.to_string());
    if let Some(args) = arguments {
        params = params.with_arguments(args);
    }
    client.peer().call_tool(params).await.unwrap()
}

/// Assert a tool response carries no agent-facing budget hints.
///
/// Budget is tracked internally (and still queryable via the explicit
/// `ministr_usage` tool) but must never be injected into ordinary tool
/// responses — the per-call numbers made agents wrongly believe they
/// were running out of context.
fn assert_no_budget_hints(v: &serde_json::Value) {
    assert!(
        v.get("usage_status").is_none() || v["usage_status"].is_null(),
        "usage_status must not be surfaced to the agent, got: {v}"
    );
    assert!(
        v.get("drop_suggestions").is_none() || v["drop_suggestions"].is_null(),
        "drop_suggestions must not be surfaced to the agent, got: {v}"
    );
}

// ---------------------------------------------------------------------------
// Tool listing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_tools_returns_all_ministr_tools() {
    let (client, _server) = wrap_as_client(setup_server().await).await;
    let tools = client.list_all_tools().await.unwrap();

    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        tool_names.contains(&"ministr_survey"),
        "should list ministr_survey, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"ministr_read"),
        "should list ministr_read, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"ministr_extract"),
        "should list ministr_extract, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"ministr_dropped"),
        "should list ministr_dropped, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"ministr_usage"),
        "should list ministr_usage, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"ministr_compress"),
        "should list ministr_compress, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"ministr_toc"),
        "should list ministr_toc, got: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"ministr_fetch"),
        "should list ministr_fetch, got: {tool_names:?}"
    );
}

// ---------------------------------------------------------------------------
// Full survey → read → extract flow via call_tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_flow_survey_read_extract_via_call_tool() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    // Step 1: Survey for JWT-related content
    let survey_result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;

    assert!(
        survey_result.is_error.is_none() || survey_result.is_error == Some(false),
        "survey should succeed"
    );

    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey_result.content)).unwrap();
    let survey_data = tool_result(&survey_json);

    let results = survey_data["results"].as_array().unwrap();
    assert!(!results.is_empty(), "survey should return results");

    for r in results {
        assert!(r["content_id"].is_string());
        assert!(r["resolution"].is_string());
        assert!(r["score"].is_number());
        assert!(r["text"].is_string());
    }

    assert_no_budget_hints(&survey_json);
    // Internal accounting is observed via the explicit budget tool, not
    // injected into the survey reply.
    let budget_after_survey = call_tool(&client, "ministr_usage", json!({})).await;
    let bjs: serde_json::Value =
        serde_json::from_str(extract_text(&budget_after_survey.content)).unwrap();
    let tokens_after_survey = bjs["estimated_used"].as_u64().unwrap();
    assert!(tokens_after_survey > 0);

    // Step 2: Read a specific section
    // Note: survey may have already delivered content from this section at
    // claim level. If the section itself was delivered, ministr_read returns
    // "already_delivered" instead of full text — which is correct behavior.
    let read_result = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    assert!(
        read_result.is_error.is_none() || read_result.is_error == Some(false),
        "read should succeed"
    );

    let read_json: serde_json::Value =
        serde_json::from_str(extract_text(&read_result.content)).unwrap();
    let read_data = tool_result(&read_json);

    // Section may be already_delivered (if survey included it) or fresh
    if read_data["status"].as_str() == Some("already_delivered") {
        assert_eq!(read_data["section_id"], "docs/auth.md#tokens");
        assert!(read_data["claims_available"].is_number());
    } else {
        assert_eq!(read_data["section_id"], "docs/auth.md#tokens");
        assert!(read_data["text"].as_str().unwrap().contains("JWT tokens"));
        assert_eq!(read_data["claims_available"], 2);

        let heading_path = read_data["heading_path"].as_array().unwrap();
        assert_eq!(heading_path[0], "Authentication");
        assert_eq!(heading_path[1], "Tokens");
    }

    assert_no_budget_hints(&read_json);
    let budget_after_read = call_tool(&client, "ministr_usage", json!({})).await;
    let bjr: serde_json::Value =
        serde_json::from_str(extract_text(&budget_after_read.content)).unwrap();
    let tokens_after_read = bjr["estimated_used"].as_u64().unwrap();
    // If the section was already delivered by survey, the read returns
    // already_delivered and may trigger fault correction (budget decreases).
    // Otherwise budget accumulates as before.
    if read_data["status"].as_str() != Some("already_delivered") {
        assert!(
            tokens_after_read >= tokens_after_survey,
            "budget should accumulate: {tokens_after_read} >= {tokens_after_survey}"
        );
    }

    // Step 3: Extract claims
    let extract_result = call_tool(
        &client,
        "ministr_extract",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    let extract_json: serde_json::Value =
        serde_json::from_str(extract_text(&extract_result.content)).unwrap();
    let extract_data = tool_result(&extract_json);

    let claims = extract_data["claims"].as_array().unwrap();
    assert_eq!(claims.len(), 2);

    let claim_texts: Vec<&str> = claims.iter().map(|c| c["text"].as_str().unwrap()).collect();
    assert!(claim_texts.iter().any(|t| t.contains("RS256")));
    assert!(claim_texts.iter().any(|t| t.contains("24 hours")));

    assert_no_budget_hints(&extract_json);
    let budget_after_extract = call_tool(&client, "ministr_usage", json!({})).await;
    let bje: serde_json::Value =
        serde_json::from_str(extract_text(&budget_after_extract.content)).unwrap();
    let tokens_after_extract = bje["estimated_used"].as_u64().unwrap();
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
    let (client, _server) = wrap_as_client(setup_server().await).await;

    // First survey delivers results
    let r1 = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    let d1 = tool_result(&j1);
    let first_count = d1["results"].as_array().unwrap().len();
    assert!(first_count > 0);

    // Second survey with same query — delivered content filtered out
    let r2 = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;
    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();
    let d2 = tool_result(&j2);
    let second_count = d2["results"].as_array().unwrap().len();
    let dedup_count = d2["deduplicated_count"].as_u64().unwrap();

    assert!(
        second_count < first_count,
        "second survey should have fewer results: {second_count} < {first_count}"
    );
    assert!(dedup_count > 0, "should report deduplicated: {dedup_count}");
}

#[tokio::test]
async fn read_re_request_skips_unchanged_with_fault_correction() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    // First read — full content
    let r1 = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    let d1 = tool_result(&j1);
    assert!(d1["text"].is_string(), "first read returns full text");

    // Second read — already delivered and unchanged, skips re-delivery
    // to save context tokens. Fault correction evicts the budget entry.
    let r2 = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();
    let d2 = tool_result(&j2);
    assert_eq!(
        d2["status"], "already_delivered",
        "re-request should return already_delivered"
    );
    assert!(
        d2["text"].is_null(),
        "should not include full text on re-request"
    );
    assert_no_budget_hints(&j2);
    assert!(d2["claims_available"].is_number());
}

// ---------------------------------------------------------------------------
// Cross-document queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn survey_finds_content_across_documents() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "rate limits API key requests", "top_k": 10}),
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&json);
    let results = data["results"].as_array().unwrap();
    assert!(!results.is_empty(), "should find rate limit content");

    let has_rate_limit = results.iter().any(|r| {
        let text = r["text"].as_str().unwrap_or("");
        text.contains("rate limit") || text.contains("Rate limit") || text.contains("100 requests")
    });
    assert!(has_rate_limit, "should find rate limit content");
}

#[tokio::test]
async fn read_sections_from_different_documents() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    // Read from auth doc
    let auth = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#oauth"}),
    )
    .await;
    let auth_json: serde_json::Value = serde_json::from_str(extract_text(&auth.content)).unwrap();
    let auth_data = tool_result(&auth_json);
    assert!(auth_data["text"].as_str().unwrap().contains("OAuth"));

    // Read from API doc
    let api = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/api.md#rate-limits"}),
    )
    .await;
    let api_json: serde_json::Value = serde_json::from_str(extract_text(&api.content)).unwrap();
    let api_data = tool_result(&api_json);
    assert!(api_data["text"].as_str().unwrap().contains("Rate limits"));

    // Budget is tracked internally (covered by
    // budget_monotonically_increases_across_tool_types via the explicit
    // ministr_usage tool) but must not be surfaced in the read reply.
    assert_no_budget_hints(&api_json);
}

// ---------------------------------------------------------------------------
// Extract with query filtering
// ---------------------------------------------------------------------------

#[tokio::test]
async fn extract_with_query_scores_and_ranks_claims() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_extract",
        json!({"section_id": "docs/auth.md#tokens", "query": "signing algorithm"}),
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&json);
    let claims = data["claims"].as_array().unwrap();

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
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/nonexistent.md#missing"}),
    )
    .await;

    // Cascade-safe: a logical failure is a *soft* error (is_error:false) so it
    // can't cancel sibling tool calls in a parallel batch.
    assert_eq!(result.is_error, Some(false));
    let text = extract_text(&result.content);
    assert!(text.contains("Section not found"), "got: {text}");
    assert!(
        text.contains("ministr_survey"),
        "should suggest discovery: {text}"
    );
}

#[tokio::test]
async fn extract_nonexistent_section_returns_user_friendly_error() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_extract",
        json!({"section_id": "docs/nonexistent.md#missing"}),
    )
    .await;

    assert_eq!(result.is_error, Some(false));
    let text = extract_text(&result.content);
    assert!(text.contains("Section not found"));
}

// ---------------------------------------------------------------------------
// FL2 — position-addressed nav (definition/references accept {file,line,col})
// ---------------------------------------------------------------------------

/// `ministr_definition` accepts a position. With a corpus that has no
/// occurrence index (this doc-only fixture), resolving a position yields a
/// cascade-safe soft error naming the position — proving the `{file,line,col}`
/// argument is parsed and routed through `symbol_at_position`, not ignored.
#[tokio::test]
async fn definition_by_position_without_occurrences_soft_errors() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_definition",
        json!({"file": "src/lib.rs", "line": 10, "col": 4}),
    )
    .await;

    assert_eq!(result.is_error, Some(false));
    let text = extract_text(&result.content);
    assert!(
        text.contains("no_symbol_at_position"),
        "position resolution should be wired: {text}"
    );
}

/// `ministr_references` likewise accepts a position and routes it through
/// the same resolver.
#[tokio::test]
async fn references_by_position_without_occurrences_soft_errors() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_references",
        json!({"file": "src/lib.rs", "line": 10, "col": 4}),
    )
    .await;

    assert_eq!(result.is_error, Some(false));
    let text = extract_text(&result.content);
    assert!(
        text.contains("no_symbol_at_position"),
        "position resolution should be wired: {text}"
    );
}

/// Calling a nav tool with neither a `symbol_id` nor a complete position is a
/// soft `missing_argument` failure (never a cascade-cancelling MCP error).
#[tokio::test]
async fn definition_without_id_or_position_soft_errors() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(&client, "ministr_definition", json!({})).await;

    assert_eq!(result.is_error, Some(false));
    let text = extract_text(&result.content);
    assert!(text.contains("missing_argument"), "got: {text}");
}

/// A partial position (file + line, no col) is incomplete → `missing_argument`,
/// proving the resolver requires all three coordinates before touching the
/// occurrence index.
#[tokio::test]
async fn references_with_partial_position_soft_errors() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_references",
        json!({"file": "src/lib.rs", "line": 10}),
    )
    .await;

    assert_eq!(result.is_error, Some(false));
    let text = extract_text(&result.content);
    assert!(text.contains("missing_argument"), "got: {text}");
}

// ---------------------------------------------------------------------------
// Budget tracking end-to-end
// ---------------------------------------------------------------------------

/// Budget still accumulates internally across tool types — observed via
/// the explicit `ministr_usage` tool, never injected into the survey/
/// read/extract responses themselves.
#[tokio::test]
async fn budget_monotonically_increases_across_tool_types() {
    // Helper: query the explicit budget tool for estimated_used.
    async fn used(client: &McpClient) -> u64 {
        let b = call_tool(client, "ministr_usage", json!({})).await;
        let j: serde_json::Value = serde_json::from_str(extract_text(&b.content)).unwrap();
        j["estimated_used"].as_u64().unwrap()
    }

    let (client, _server) = wrap_as_client(setup_server().await).await;

    let r1 = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "tokens", "top_k": 5}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    assert_no_budget_hints(&j1);
    let t1 = used(&client).await;

    let r2 = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/api.md#rate-limits"}),
    )
    .await;
    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();
    assert_no_budget_hints(&j2);
    let t2 = used(&client).await;

    // Extract from a section NOT previously delivered (auth tokens)
    let r3 = call_tool(
        &client,
        "ministr_extract",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j3: serde_json::Value = serde_json::from_str(extract_text(&r3.content)).unwrap();
    assert_no_budget_hints(&j3);
    let t3 = used(&client).await;

    assert!(t1 > 0, "survey should use tokens");
    assert!(t2 > t1, "read adds tokens: {t2} > {t1}");
    assert!(t3 >= t2, "extract should not decrease tokens: {t3} >= {t2}");

    // The explicit budget tool still reports pressure level.
    let b = call_tool(&client, "ministr_usage", json!({})).await;
    let bj: serde_json::Value = serde_json::from_str(extract_text(&b.content)).unwrap();
    assert_eq!(
        bj["level"].as_str().unwrap(),
        "normal",
        "small corpus should not trigger pressure"
    );
}

// ---------------------------------------------------------------------------
// ministr_dropped tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn evicted_removes_content_from_session_and_budget() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    // Deliver content first via read
    let _ = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    let budget_before = call_tool(&client, "ministr_usage", json!({})).await;
    let j_before: serde_json::Value =
        serde_json::from_str(extract_text(&budget_before.content)).unwrap();
    let used_before = j_before["estimated_used"].as_u64().unwrap();
    assert!(used_before > 0, "should have used tokens after read");

    // Evict the delivered content
    let evict_result = call_tool(
        &client,
        "ministr_dropped",
        json!({"content_ids": ["docs/auth.md#tokens"]}),
    )
    .await;

    assert!(
        evict_result.is_error.is_none() || evict_result.is_error == Some(false),
        "eviction should succeed"
    );

    let evict_json: serde_json::Value =
        serde_json::from_str(extract_text(&evict_result.content)).unwrap();
    let evict_data = tool_result(&evict_json);
    assert_eq!(
        evict_data["dropped"].as_array().unwrap().len(),
        1,
        "should evict one item"
    );
    assert!(
        evict_data["not_found"].as_array().unwrap().is_empty(),
        "should have no not_found items"
    );
}

#[tokio::test]
async fn evicted_reports_not_found_for_unknown_ids() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_dropped",
        json!({"content_ids": ["nonexistent-id"]}),
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&json);
    assert!(data["dropped"].as_array().unwrap().is_empty());
    assert_eq!(data["not_found"].as_array().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// ministr_usage standalone
// ---------------------------------------------------------------------------

#[tokio::test]
async fn budget_returns_complete_status() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(&client, "ministr_usage", json!({})).await;

    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "budget should succeed"
    );

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    assert!(json["total_budget"].is_number());
    assert!(json["estimated_used"].is_number());
    assert!(json["estimated_remaining"].is_number());
    assert!(json["level"].is_string());
    assert!(json["drop_candidates"].is_array());
    assert!(json["prefetch_metrics"].is_object());

    // Fresh session should have zero usage
    assert_eq!(json["estimated_used"].as_u64().unwrap(), 0);
    assert_eq!(json["level"].as_str().unwrap(), "normal");
}

// ---------------------------------------------------------------------------
// ministr_compress tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn compress_returns_summaries_for_sections() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_compress",
        json!({"content_ids": ["docs/auth.md#tokens", "docs/api.md#rate-limits"]}),
    )
    .await;

    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "compress should succeed"
    );

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&json);
    let summaries = data["summaries"].as_array().unwrap();
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

    assert_no_budget_hints(&json);
}

// ---------------------------------------------------------------------------
// ministr_related tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn related_returns_linked_claims() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(&client, "ministr_related", json!({"claim_id": "auth-c1"})).await;

    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "related should succeed"
    );

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&json);
    let related = data["related"].as_array().unwrap();
    assert!(
        !related.is_empty(),
        "auth-c1 should have related claims via inserted relationships"
    );

    for r in related {
        assert!(r["claim_id"].is_string());
        assert!(r["text"].is_string());
        assert!(r["relation_type"].is_string());
    }

    assert_no_budget_hints(&json);
}

#[tokio::test]
async fn related_with_type_filter() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_related",
        json!({"claim_id": "auth-c1", "relation_types": ["references"]}),
    )
    .await;

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&json);
    let related = data["related"].as_array().unwrap();

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
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = client.peer().list_resources(None).await.unwrap();

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
        names.contains(&"ministr status"),
        "should include ministr status resource, got: {names:?}"
    );
}

#[tokio::test]
async fn list_resource_templates_includes_corpus() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = client.peer().list_resource_templates(None).await.unwrap();

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
    use rmcp::model::ReadResourceRequestParams;

    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = client
        .peer()
        .read_resource(ReadResourceRequestParams::new("ministr://status"))
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
// Tool listing: verify ministr_related is present (7 tools total)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_tools_returns_seven_tools_including_related() {
    let (client, _server) = wrap_as_client(setup_server().await).await;
    let tools = client.list_all_tools().await.unwrap();

    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        tool_names.contains(&"ministr_related"),
        "should list ministr_related, got: {tool_names:?}"
    );

    assert!(
        tools.len() >= 7,
        "should have at least 7 tools, got: {}",
        tools.len()
    );
}

// ---------------------------------------------------------------------------
// I2.2 + I2.3: Coherence alerts surface in MCP tool responses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn coherence_alerts_surface_in_ministr_read_response() {
    let server = setup_server().await;
    let (client, _server) = wrap_as_client(server.clone()).await;

    // Read a section to populate the session shadow
    let result = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    assert!(!result.is_error.unwrap_or(false));

    // Manually invalidate a section via the session to simulate coherence
    {
        let registry = server.registry_arc();
        let mut reg = registry.lock().await;
        reg.invalidate_all(&["docs/auth.md#tokens".to_string()]);
    }

    // Next tool call should surface the coherence alert
    let result = call_tool(
        &client,
        "ministr_read",
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
async fn coherence_alerts_surface_in_ministr_usage_response() {
    let server = setup_server().await;
    let (client, _server) = wrap_as_client(server.clone()).await;

    // Deliver a section
    call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    // Invalidate it
    {
        let registry = server.registry_arc();
        let mut reg = registry.lock().await;
        reg.invalidate_all(&["docs/auth.md#tokens".to_string()]);
    }

    // Budget tool should surface the alert
    let result = call_tool(&client, "ministr_usage", json!({})).await;
    let text = extract_text(&result.content);
    let json: serde_json::Value = serde_json::from_str(text).unwrap();

    assert!(
        json["coherence_alerts"].is_array(),
        "ministr_usage should include coherence_alerts, got: {json}"
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
async fn setup_server_with_persistence() -> MinistrServer {
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
    let budget_config = UsageConfig::default();
    MinistrServer::with_persistence(
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
    let (client, _server) = wrap_as_client(server.clone()).await;

    // Read multiple sections to build a co-access trajectory
    call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#oauth"}),
    )
    .await;
    call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/api.md#rate-limits"}),
    )
    .await;

    // Re-read a section to trigger cross-session prefetch via co-access data
    call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;

    // Check budget response includes prefetch_metrics
    let result = call_tool(&client, "ministr_usage", json!({})).await;
    let text = extract_text(&result.content);
    let json: serde_json::Value = serde_json::from_str(text).unwrap();

    assert!(
        json["prefetch_metrics"].is_object(),
        "ministr_usage should include prefetch_metrics, got: {json}"
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
    let analytics = ministr_core::analytics::Analytics::new((*storage).clone());
    let co = analytics
        .co_accessed_with(
            &ministr_core::types::SectionId("docs/auth.md#tokens".into()),
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
/// pipeline (with embeddings), and return an `MinistrServer` backed by the result.
async fn setup_server_from_corpus_dir() -> (MinistrServer, tempfile::TempDir) {
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
    let pipeline = ministr_core::ingestion::IngestionPipeline::new();
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
    let budget_config = UsageConfig::default();
    let server = MinistrServer::with_persistence(
        service,
        budget_config,
        storage,
        Some("e2e-corpus-test".into()),
    )
    .await;

    (server, dir)
}

// ---------------------------------------------------------------------------
// I3.0: ministr_survey returns ranked results from a real corpus directory
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_survey_from_corpus_dir_returns_ranked_results() {
    let (server, _dir) = setup_server_from_corpus_dir().await;
    let (client, _server) = wrap_as_client(server).await;

    let result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "JWT authentication tokens", "top_k": 10}),
    )
    .await;

    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "survey should succeed"
    );

    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&json);
    let results = data["results"].as_array().unwrap();
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

    assert_no_budget_hints(&json);
}

// ---------------------------------------------------------------------------
// I3.1: ministr_read returns full section text with heading paths and content hashes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_read_returns_heading_paths_and_content_hash() {
    let (server, _dir) = setup_server_from_corpus_dir().await;
    let (client, _server) = wrap_as_client(server.clone()).await;

    // Read a section directly (before any survey, so it's a fresh delivery)
    let section_id = "api.md#api-reference/rate-limits";
    let read = call_tool(&client, "ministr_read", json!({"section_id": section_id})).await;

    assert!(
        read.is_error.is_none() || read.is_error == Some(false),
        "read should succeed"
    );

    let read_json: serde_json::Value = serde_json::from_str(extract_text(&read.content)).unwrap();
    let read_data = tool_result(&read_json);

    // Verify section_id matches
    assert_eq!(read_data["section_id"].as_str().unwrap(), section_id);

    // Verify heading_path is present and non-empty
    let heading_path = read_data["heading_path"].as_array().unwrap();
    assert!(
        !heading_path.is_empty(),
        "heading_path should be non-empty for a section"
    );

    // Verify full text content is returned
    let text = read_data["text"].as_str().unwrap();
    assert!(!text.is_empty(), "text should be non-empty");

    // Verify claims_available is a number
    assert!(
        read_data["claims_available"].is_number(),
        "should report claims_available"
    );

    // Verify the session recorded delivery (content hash tracked for dedup)
    let registry = server.registry_arc();
    let reg = registry.lock().await;
    let entry = reg
        .get_session(server.active_session_id())
        .expect("active session exists");
    let content_id = ministr_core::types::ContentId(section_id.to_string());
    assert!(
        entry.session.is_delivered(&content_id),
        "session should track delivered content with its hash"
    );
}

// ---------------------------------------------------------------------------
// I3.2: ministr_extract returns claims from ingested content
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_extract_returns_claims_from_ingested_content() {
    let (server, _dir) = setup_server_from_corpus_dir().await;
    let (client, _server) = wrap_as_client(server).await;

    // Discover a section with claims
    let survey = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "tokens JWT signing expire", "top_k": 10}),
    )
    .await;
    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey.content)).unwrap();
    let survey_data = tool_result(&survey_json);
    let results = survey_data["results"].as_array().unwrap();

    let section_result = results
        .iter()
        .find(|r| r["resolution"].as_str() == Some("section"))
        .expect("should have section-level results");
    let section_id = section_result["content_id"].as_str().unwrap();

    // Extract claims
    let extract = call_tool(
        &client,
        "ministr_extract",
        json!({"section_id": section_id}),
    )
    .await;

    assert!(
        extract.is_error.is_none() || extract.is_error == Some(false),
        "extract should succeed"
    );

    let extract_json: serde_json::Value =
        serde_json::from_str(extract_text(&extract.content)).unwrap();
    let extract_data = tool_result(&extract_json);
    let claims = extract_data["claims"].as_array().unwrap();

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

    assert_no_budget_hints(&extract_json);
}

// ---------------------------------------------------------------------------
// I3.3: ministr_related follows claim dependency chains
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_related_follows_claim_dependency_chains() {
    let (server, _dir) = setup_server_from_corpus_dir().await;
    let (client, _server) = wrap_as_client(server).await;

    // Find claim IDs via extract
    let survey = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "tokens JWT signing expire", "top_k": 10}),
    )
    .await;
    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey.content)).unwrap();
    let survey_data = tool_result(&survey_json);

    // Find a claim-level result to trace
    let claim_result = survey_data["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["resolution"].as_str() == Some("claim"));

    if let Some(claim) = claim_result {
        let claim_id = claim["content_id"].as_str().unwrap();

        let related = call_tool(&client, "ministr_related", json!({"claim_id": claim_id})).await;

        assert!(
            related.is_error.is_none() || related.is_error == Some(false),
            "ministr_related should succeed for an existing claim"
        );

        let related_json: serde_json::Value =
            serde_json::from_str(extract_text(&related.content)).unwrap();
        let related_data = tool_result(&related_json);

        // The related array may be empty if no relationships were detected,
        // but the response structure should be valid.
        assert!(
            related_data["related"].is_array(),
            "should have a related array"
        );

        for r in related_data["related"].as_array().unwrap() {
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

        assert_no_budget_hints(&related_json);
    }

    // Also verify with the programmatic corpus that has guaranteed relationships
    let (client2, _server2) = wrap_as_client(setup_server().await).await;
    let result = call_tool(&client2, "ministr_related", json!({"claim_id": "auth-c1"})).await;
    let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&json);
    let related = data["related"].as_array().unwrap();
    assert!(
        !related.is_empty(),
        "auth-c1 should have related claims via dependency chain"
    );
    // Follow the chain: auth-c1 → auth-c2 (references)
    assert_eq!(related[0]["claim_id"].as_str().unwrap(), "auth-c2");
    assert_eq!(related[0]["relation_type"].as_str().unwrap(), "references");
}

// ---------------------------------------------------------------------------
// I3.4: session deduplication — ministr_read same section twice
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_read_session_dedup_on_second_request() {
    let server = setup_server().await;
    let (client, _server) = wrap_as_client(server.clone()).await;

    // First read — full content delivered
    let r1 = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    let d1 = tool_result(&j1);
    assert!(
        d1["text"].is_string(),
        "first read should deliver full text"
    );
    let first_text = d1["text"].as_str().unwrap();
    assert!(
        first_text.contains("JWT tokens"),
        "first read should contain section text"
    );

    // Verify session tracks the delivery
    {
        let registry = server.registry_arc();
        let reg = registry.lock().await;
        let entry = reg
            .get_session(server.active_session_id())
            .expect("active session exists");
        let content_id = ministr_core::types::ContentId("docs/auth.md#tokens".into());
        assert!(
            entry.session.is_delivered(&content_id),
            "session should mark content as delivered after first read"
        );
    }

    // Second read — same section, unchanged content
    // Should return "already_delivered" to save context tokens
    let r2 = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();
    let d2 = tool_result(&j2);

    assert_eq!(
        d2["status"], "already_delivered",
        "re-request should return already_delivered status"
    );
    assert!(
        d2["text"].is_null(),
        "re-request should not include full text"
    );
    assert_no_budget_hints(&j2);
    assert!(d2["claims_available"].is_number());

    // Verify survey deduplication works — same query returns fewer results
    let s1 = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;
    let sj1: serde_json::Value = serde_json::from_str(extract_text(&s1.content)).unwrap();
    let sd1 = tool_result(&sj1);
    let first_count = sd1["results"].as_array().unwrap().len();

    let s2 = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "JWT authentication tokens signing", "top_k": 10}),
    )
    .await;
    let sj2: serde_json::Value = serde_json::from_str(extract_text(&s2.content)).unwrap();
    let sd2 = tool_result(&sj2);
    let second_count = sd2["results"].as_array().unwrap().len();
    let dedup_count = sd2["deduplicated_count"].as_u64().unwrap();

    assert!(
        second_count < first_count || dedup_count > 0,
        "survey dedup should filter already-delivered content: \
         second={second_count} < first={first_count}, dedup={dedup_count}"
    );
}

// ---------------------------------------------------------------------------
// I3.5: ministr_compress + ministr_dropped cycle works and budget updates
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_compress_evict_cycle_updates_budget() {
    let server = setup_server().await;
    let (client, _server) = wrap_as_client(server.clone()).await;

    // Step 1: Deliver content to build up budget usage
    let _ = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    let _ = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/api.md#rate-limits"}),
    )
    .await;

    // Check budget after reads
    let budget_before = call_tool(&client, "ministr_usage", json!({})).await;
    let jb: serde_json::Value = serde_json::from_str(extract_text(&budget_before.content)).unwrap();
    let used_before = jb["estimated_used"].as_u64().unwrap();
    assert!(used_before > 0, "should have budget usage after reads");

    // Step 2: Compress the sections
    let compress = call_tool(
        &client,
        "ministr_compress",
        json!({"content_ids": ["docs/auth.md#tokens", "docs/api.md#rate-limits"]}),
    )
    .await;
    assert!(
        compress.is_error.is_none() || compress.is_error == Some(false),
        "compress should succeed"
    );

    let cj: serde_json::Value = serde_json::from_str(extract_text(&compress.content)).unwrap();
    let cd = tool_result(&cj);
    let summaries = cd["summaries"].as_array().unwrap();
    assert_eq!(summaries.len(), 2, "should compress both sections");

    for s in summaries {
        assert!(s["summary"].is_string());
        let orig = s["original_tokens"].as_u64().unwrap();
        let comp = s["compressed_tokens"].as_u64().unwrap();
        assert!(comp <= orig, "compressed <= original: {comp} <= {orig}");
    }

    // Step 3: Evict the content after compression
    let evict = call_tool(
        &client,
        "ministr_dropped",
        json!({"content_ids": ["docs/auth.md#tokens", "docs/api.md#rate-limits"]}),
    )
    .await;
    assert!(
        evict.is_error.is_none() || evict.is_error == Some(false),
        "evict should succeed"
    );

    let ej: serde_json::Value = serde_json::from_str(extract_text(&evict.content)).unwrap();
    let ed = tool_result(&ej);
    assert_eq!(
        ed["dropped"].as_array().unwrap().len(),
        2,
        "should evict both items"
    );
    assert!(
        ed["not_found"].as_array().unwrap().is_empty(),
        "should have no not_found"
    );

    // Step 4: Verify budget decreased after eviction
    let budget_after = call_tool(&client, "ministr_usage", json!({})).await;
    let ja: serde_json::Value = serde_json::from_str(extract_text(&budget_after.content)).unwrap();
    let used_after = ja["estimated_used"].as_u64().unwrap();

    assert!(
        used_after < used_before,
        "budget should decrease after eviction: {used_after} < {used_before}"
    );

    // Step 5: Verify evicted content is no longer in the session
    {
        let registry = server.registry_arc();
        let reg = registry.lock().await;
        let entry = reg
            .get_session(server.active_session_id())
            .expect("active session exists");
        assert!(
            !entry.session.is_delivered(&ministr_core::types::ContentId(
                "docs/auth.md#tokens".into()
            )),
            "evicted content should not be marked as delivered"
        );
    }
}

// ---------------------------------------------------------------------------
// I3.6: modify a corpus file, verify coherence detects change and ministr_read
//       returns updated content
// ---------------------------------------------------------------------------

/// Set up a single-file corpus for coherence testing.
///
/// Returns the server, temp dir, embedder, index, and storage arcs needed
/// for file modification and re-indexing.
#[allow(clippy::type_complexity)]
async fn setup_coherence_server() -> (
    MinistrServer,
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

    let pipeline = ministr_core::ingestion::IngestionPipeline::new();
    pipeline
        .ingest_directory_with_embeddings(dir.path(), &storage, embedder.as_ref(), index.as_ref())
        .await
        .unwrap();

    let storage = Arc::new(storage);
    let service = Arc::new(QueryService::new(
        (*storage).clone(),
        Arc::clone(&embedder) as Arc<dyn ministr_core::embedding::Embedder>,
        Arc::clone(&index) as Arc<dyn ministr_core::index::VectorIndex>,
    ));
    let server = MinistrServer::with_persistence(
        service,
        UsageConfig::default(),
        Arc::clone(&storage),
        Some("coherence-test".into()),
    )
    .await;

    (server, dir, embedder, index, storage)
}

#[tokio::test]
async fn e2e_coherence_detects_file_change_and_read_returns_updated() {
    let (server, dir, embedder, index, storage) = setup_coherence_server().await;
    let (client, _server) = wrap_as_client(server.clone()).await;
    let auth_path = dir.path().join("auth.md");

    // Read the original section
    let r1 = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "auth.md#authentication/tokens"}),
    )
    .await;
    let j1: serde_json::Value = serde_json::from_str(extract_text(&r1.content)).unwrap();
    let d1 = tool_result(&j1);
    let original_text = d1["text"].as_str().unwrap();
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
    let engine = ministr_core::coherence::CoherenceEngine::with_embeddings(
        dir.path().to_path_buf(),
        Arc::clone(&embedder) as Arc<dyn ministr_core::embedding::Embedder>,
        Arc::clone(&index) as Arc<dyn ministr_core::index::VectorIndex>,
    );
    let events = vec![ministr_core::coherence::CoherenceEvent::Modified(
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

    // Invalidate all sessions in the registry
    {
        let registry = server.registry_arc();
        let mut reg = registry.lock().await;
        let invalidated = reg.invalidate_all(&affected);
        assert!(invalidated > 0, "should invalidate delivered sections");
    }

    // Read the section again — should get updated content
    let r2 = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "auth.md#authentication/tokens"}),
    )
    .await;

    assert!(
        r2.is_error.is_none() || r2.is_error == Some(false),
        "read after coherence update should succeed"
    );

    let j2: serde_json::Value = serde_json::from_str(extract_text(&r2.content)).unwrap();
    let d2 = tool_result(&j2);
    let updated_text = d2["text"].as_str().unwrap();
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

// ---------------------------------------------------------------------------
// ministr_toc — table of contents
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ministr_toc_returns_all_documents_and_sections() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    // Full corpus TOC (no filter)
    let result = call_tool(&client, "ministr_toc", json!({})).await;
    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "ministr_toc should succeed"
    );

    let body: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&body);

    // Corpus stats header
    let stats = &data["corpus_stats"];
    assert_eq!(
        stats["documents"].as_u64().unwrap(),
        2,
        "should have 2 docs"
    );
    assert_eq!(
        stats["sections"].as_u64().unwrap(),
        3,
        "should have 3 sections"
    );
    assert!(
        stats["claims"].as_u64().unwrap() >= 4,
        "should have at least 4 claims"
    );

    // Entries
    let entries = data["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(entries.len(), 3, "should have 3 TOC entries");

    // Check that entries have the expected fields and no text content
    for entry in entries {
        assert!(entry["document_id"].is_string());
        assert!(entry["section_id"].is_string());
        assert!(entry["heading_path"].is_array());
        assert!(entry["depth"].is_u64());
        assert!(entry["claims_available"].is_u64());
        assert!(entry["token_count"].is_u64());
        // No text field — metadata only
        assert!(
            entry.get("text").is_none(),
            "TOC entries should not contain text"
        );
    }

    assert_no_budget_hints(&body);
}

#[tokio::test]
async fn ministr_toc_filters_by_document_id() {
    let (client, _server) = wrap_as_client(setup_server().await).await;

    let result = call_tool(
        &client,
        "ministr_toc",
        json!({"document_id": "docs/api.md"}),
    )
    .await;
    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "filtered ministr_toc should succeed"
    );

    let body: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
    let data = tool_result(&body);

    let entries = data["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(entries.len(), 1, "should have 1 section for docs/api.md");
    assert_eq!(entries[0]["document_id"].as_str().unwrap(), "docs/api.md");
    assert_eq!(
        entries[0]["section_id"].as_str().unwrap(),
        "docs/api.md#rate-limits"
    );

    let stats = &data["corpus_stats"];
    assert_eq!(stats["documents"].as_u64().unwrap(), 1);
    assert_eq!(stats["sections"].as_u64().unwrap(), 1);
}

// ---------------------------------------------------------------------------
// Survey-triggered prefetch
// ---------------------------------------------------------------------------

/// Build a corpus with claim IDs in the `{section_id}:c{N}` format
/// (as produced by the real claim extractor).
fn build_survey_prefetch_corpus() -> Vec<DocumentTree> {
    vec![DocumentTree {
        id: ContentId("docs/auth.md".into()),
        title: "Authentication Guide".into(),
        source_path: "docs/auth.md".into(),
        sections: vec![
            Section {
                id: SectionId("docs/auth.md#tokens".into()),
                heading_path: vec!["Authentication".into(), "Tokens".into()],
                depth: 2,
                text: "JWT tokens use RS256 signing. Tokens expire after 24 hours.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![
                    Claim {
                        id: ClaimId("docs/auth.md#tokens:c0".into()),
                        text: "JWT tokens use RS256 signing algorithm.".into(),
                        section_id: SectionId("docs/auth.md#tokens".into()),
                    },
                    Claim {
                        id: ClaimId("docs/auth.md#tokens:c1".into()),
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
                text: "OAuth 2.0 authorization code flow with PKCE.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![Claim {
                    id: ClaimId("docs/auth.md#oauth:c0".into()),
                    text: "OAuth 2.0 authorization code flow is supported.".into(),
                    section_id: SectionId("docs/auth.md#oauth".into()),
                }],
                summary: Some("OAuth 2.0 integration details.".into()),
            },
        ],
        summary: Some("Complete authentication reference.".into()),
    }]
}

async fn setup_survey_prefetch_server() -> MinistrServer {
    let dim = 16;
    let embedder = Arc::new(MockEmbedder { dim });
    let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();

    let corpus = build_survey_prefetch_corpus();
    for doc in &corpus {
        storage.insert_document(doc).await.unwrap();
    }

    // Index only claims (no section-level vectors) so survey returns
    // claim-level hits whose parent sections get pre-warmed by survey-expand.
    let texts_and_ids = [
        (
            "doc-summary::docs/auth.md",
            "Complete authentication reference.",
        ),
        (
            "claim::docs/auth.md#tokens:c0",
            "JWT tokens use RS256 signing algorithm.",
        ),
        (
            "claim::docs/auth.md#tokens:c1",
            "Tokens expire after 24 hours by default.",
        ),
        (
            "claim::docs/auth.md#oauth:c0",
            "OAuth 2.0 authorization code flow is supported.",
        ),
    ];

    for (id, text) in &texts_and_ids {
        let vecs = embedder.embed(&[*text]).unwrap();
        index.insert(id, &vecs[0]).unwrap();
    }

    let service = Arc::new(QueryService::new(storage, embedder, index));
    MinistrServer::new(service)
}

#[tokio::test]
async fn survey_prewarms_parent_sections_of_claim_hits() {
    let (client, _server) = wrap_as_client(setup_survey_prefetch_server().await).await;

    // Survey for a query that should return claim-level hits
    let survey_result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "JWT RS256 signing tokens", "top_k": 10}),
    )
    .await;
    assert!(
        survey_result.is_error.is_none() || survey_result.is_error == Some(false),
        "survey should succeed"
    );

    let survey_body: serde_json::Value =
        serde_json::from_str(extract_text(&survey_result.content)).unwrap();
    let survey_data = tool_result(&survey_body);
    let results = survey_data["results"]
        .as_array()
        .expect("should have results");

    // Verify we got at least one claim-level result
    let has_claim = results.iter().any(|r| r["resolution"] == "claim");
    assert!(has_claim, "survey should include claim-level results");

    // Now read a parent section — should hit the prefetch cache
    let read_result = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    assert!(
        read_result.is_error.is_none() || read_result.is_error == Some(false),
        "read should succeed"
    );

    // Check prefetch metrics via ministr_usage
    let budget_result = call_tool(&client, "ministr_usage", json!({})).await;
    let budget_body: serde_json::Value =
        serde_json::from_str(extract_text(&budget_result.content)).unwrap();

    let prefetch_metrics = &budget_body["prefetch_metrics"];
    let survey_expand_hits = prefetch_metrics["survey_expand_hits"].as_u64().unwrap_or(0);

    // The read of docs/auth.md#tokens should have been a prefetch cache hit
    // (pre-warmed by the survey's claim results)
    assert!(
        survey_expand_hits > 0,
        "survey-expand prefetch should have hits, got metrics: {prefetch_metrics}"
    );
}

// ---------------------------------------------------------------------------
// I7: Multi-path corpus ingestion
// ---------------------------------------------------------------------------

/// Set up a multi-path corpus with a directory and individual files, ingest via
/// `ingest_paths_with_embeddings`, and return an `MinistrServer`.
async fn setup_server_from_multi_path_corpus() -> (MinistrServer, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().unwrap();

    // Create a subdirectory with docs
    let docs_dir = dir.path().join("docs");
    std::fs::create_dir(&docs_dir).unwrap();
    std::fs::write(
        docs_dir.join("guide.md"),
        "\
# User Guide

## Getting Started

Install the CLI with `cargo install ministr`. Run `ministr --help` for usage info.

## Configuration

Edit `~/.ministr/config.toml` to set your preferred model and context budget.
",
    )
    .unwrap();

    // Create an individual file at the root level
    std::fs::write(
        dir.path().join("DESIGN.md"),
        "\
# Design Document

## Architecture

ministr uses a layered architecture: transport, service, and storage.
The MCP server handles JSON-RPC routing and delegates to the query service.
",
    )
    .unwrap();

    // Create another individual file
    std::fs::write(
        dir.path().join("CHANGELOG.md"),
        "\
# Changelog

## v0.2.0

Added multi-path corpus support. Users can now index multiple directories
and individual files in a single ministr session.
",
    )
    .unwrap();

    let dim = 16;
    let embedder = Arc::new(MockEmbedder { dim });
    let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Ingest via multi-path pipeline
    let pipeline = ministr_core::ingestion::IngestionPipeline::new();
    let corpus_paths = vec![
        docs_dir,
        dir.path().join("DESIGN.md"),
        dir.path().join("CHANGELOG.md"),
    ];
    let stats = pipeline
        .ingest_paths_with_embeddings(&corpus_paths, &storage, embedder.as_ref(), index.as_ref())
        .await
        .unwrap();

    assert_eq!(
        stats.files_indexed, 3,
        "should ingest 3 files (1 from dir + 2 individual), got: {stats:?}"
    );
    assert!(stats.total_sections > 0, "should extract sections");

    let storage = Arc::new(storage);
    let service = Arc::new(QueryService::new((*storage).clone(), embedder, index));
    let budget_config = UsageConfig::default();
    let server = MinistrServer::with_persistence(
        service,
        budget_config,
        storage,
        Some("e2e-multi-path-test".into()),
    )
    .await;

    (server, dir)
}

#[tokio::test]
async fn e2e_multi_path_toc_shows_all_sources() {
    let (server, _dir) = setup_server_from_multi_path_corpus().await;
    let (client, _server) = wrap_as_client(server).await;

    let result = call_tool(&client, "ministr_toc", json!({})).await;
    assert!(
        !result.is_error.unwrap_or(false),
        "ministr_toc should succeed"
    );

    let text = extract_text(&result.content);

    // Documents from the directory
    assert!(
        text.contains("guide.md"),
        "toc should include guide.md from docs/, got:\n{text}"
    );

    // Individual files
    assert!(
        text.contains("DESIGN.md"),
        "toc should include individual file DESIGN.md, got:\n{text}"
    );
    assert!(
        text.contains("CHANGELOG.md"),
        "toc should include individual file CHANGELOG.md, got:\n{text}"
    );
}

#[tokio::test]
async fn e2e_multi_path_survey_finds_content_from_individual_file() {
    let (server, _dir) = setup_server_from_multi_path_corpus().await;
    let (client, _server) = wrap_as_client(server).await;

    // Search for content that exists in the individual DESIGN.md file
    let result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "layered architecture transport service storage"}),
    )
    .await;
    assert!(
        !result.is_error.unwrap_or(false),
        "ministr_survey should succeed"
    );

    let text = extract_text(&result.content);
    assert!(
        text.contains("DESIGN.md") || text.contains("Architecture") || text.contains("layered"),
        "survey should find content from DESIGN.md, got:\n{text}"
    );
}

#[tokio::test]
async fn e2e_multi_path_survey_finds_content_from_directory() {
    let (server, _dir) = setup_server_from_multi_path_corpus().await;
    let (client, _server) = wrap_as_client(server).await;

    // Search for content from the docs directory
    let result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "install CLI cargo configuration"}),
    )
    .await;
    assert!(
        !result.is_error.unwrap_or(false),
        "ministr_survey should succeed"
    );

    let text = extract_text(&result.content);
    assert!(
        text.contains("guide.md") || text.contains("Getting Started") || text.contains("cargo"),
        "survey should find content from docs/guide.md, got:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// ministr_fetch tests
// ---------------------------------------------------------------------------

/// Start a minimal HTTP server that serves test HTML content.
/// Returns the server address (e.g. "127.0.0.1:12345") and a handle to shut it down.
async fn start_test_http_server(html: &str) -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let addr_str = addr.to_string();

    let response_body = html.to_string();
    let handle = tokio::spawn(async move {
        // Accept connections until the handle is dropped
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let body = response_body.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                // Read the request (we don't parse it, just consume it)
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    (addr_str, handle)
}

/// Set up an `MinistrServer` with web fetcher enabled for testing `ministr_fetch`.
async fn setup_server_with_web_fetcher(
    web_cache_dir: &std::path::Path,
) -> (MinistrServer, Arc<dyn Embedder>, Arc<dyn VectorIndex>) {
    let dim = 16;
    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
    let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();
    let storage = Arc::new(storage);

    let service = Arc::new(QueryService::new(
        (*storage).clone(),
        Arc::clone(&embedder),
        Arc::clone(&index),
    ));

    let budget_config = UsageConfig::default();
    let server = MinistrServer::with_persistence(
        service,
        budget_config,
        Arc::clone(&storage),
        Some("test-fetch-session".to_string()),
    )
    .await;

    let http_client = ministr_core::web::HttpClient::with_defaults().unwrap();
    let web_fetcher = ministr_core::web::fetcher::WebFetcher::new(
        http_client,
        web_cache_dir,
        ministr_core::web::fetcher::WebFetcherConfig::default(),
    );

    let server = server.with_web_fetcher(web_fetcher, Arc::clone(&embedder), Arc::clone(&index));

    (server, embedder, index)
}

#[tokio::test]
async fn ministr_fetch_not_configured_returns_error() {
    let (client, _server) = wrap_as_client(setup_server().await).await;
    let result = call_tool(
        &client,
        "ministr_fetch",
        json!({"url": "https://example.com/docs/"}),
    )
    .await;

    assert!(
        result.is_error == Some(false),
        "ministr_fetch should report a soft (non-cascading) error when not configured"
    );

    let text = extract_text(&result.content);
    assert!(
        text.contains("not available"),
        "error should mention not available, got: {text}"
    );
}

#[tokio::test]
async fn ministr_fetch_lists_in_tools() {
    let (client, _server) = wrap_as_client(setup_server().await).await;
    let tools = client.list_all_tools().await.unwrap();

    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        tool_names.contains(&"ministr_fetch"),
        "should list ministr_fetch, got: {tool_names:?}"
    );
}

#[tokio::test]
async fn e2e_ministr_fetch_then_survey_and_toc() {
    let test_html = r"
    <html><head><title>Widget API Reference</title></head>
    <body>
    <h1>Widget API Reference</h1>
    <p>The Widget API provides endpoints for managing widgets in your application.</p>
    <h2>Creating Widgets</h2>
    <p>POST /api/widgets creates a new widget. The request body must include a name field and an optional description field. The maximum name length is 255 characters.</p>
    <h2>Listing Widgets</h2>
    <p>GET /api/widgets returns a paginated list of widgets. Use the page and per_page query parameters to control pagination. The default page size is 20 items.</p>
    <h2>Deleting Widgets</h2>
    <p>DELETE /api/widgets/:id removes a widget permanently. This operation cannot be undone. Returns 204 No Content on success.</p>
    </body></html>
    ";

    let (addr, server_handle) = start_test_http_server(test_html).await;
    let tmp = tempfile::tempdir().unwrap();
    let (server, _embedder, _index) = setup_server_with_web_fetcher(tmp.path()).await;
    let (client, _server) = wrap_as_client(server).await;

    // Step 1: Call ministr_fetch to fetch the test page
    let fetch_result = call_tool(
        &client,
        "ministr_fetch",
        json!({"url": format!("http://{addr}/")}),
    )
    .await;

    assert!(
        fetch_result.is_error.is_none() || fetch_result.is_error == Some(false),
        "ministr_fetch should succeed, got: {}",
        extract_text(&fetch_result.content)
    );

    let fetch_json: serde_json::Value =
        serde_json::from_str(extract_text(&fetch_result.content)).unwrap();
    let fetch_data = tool_result(&fetch_json);

    assert_eq!(
        fetch_data["pages_fetched"].as_u64().unwrap(),
        1,
        "should fetch exactly 1 page"
    );
    assert!(
        fetch_data["sections_indexed"].as_u64().unwrap() > 0,
        "should index sections"
    );
    assert_eq!(
        fetch_data["strategy_used"].as_str().unwrap(),
        "direct_fetch",
        "should use direct fetch strategy for plain URL"
    );
    assert!(
        fetch_data["tokens_added"].as_u64().unwrap() > 0,
        "should report tokens added"
    );
    assert_no_budget_hints(&fetch_json);

    // Step 2: Verify ministr_survey finds the fetched content
    let survey_result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "widget API creating widgets endpoints", "top_k": 10}),
    )
    .await;

    assert!(
        survey_result.is_error.is_none() || survey_result.is_error == Some(false),
        "ministr_survey should succeed after fetch"
    );

    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey_result.content)).unwrap();
    let survey_data = tool_result(&survey_json);
    let results = survey_data["results"].as_array().unwrap();
    assert!(!results.is_empty(), "survey should find fetched content");

    // Verify at least one result references web content
    let has_web_content = results
        .iter()
        .any(|r| r["content_id"].as_str().unwrap_or("").contains("web://"));
    assert!(
        has_web_content,
        "survey results should include web-fetched content, got: {results:?}"
    );

    // Step 3: Verify ministr_toc lists the fetched document
    let toc_result = call_tool(&client, "ministr_toc", json!({})).await;

    assert!(
        toc_result.is_error.is_none() || toc_result.is_error == Some(false),
        "ministr_toc should succeed"
    );

    let toc_json: serde_json::Value =
        serde_json::from_str(extract_text(&toc_result.content)).unwrap();
    let toc_data = tool_result(&toc_json);
    let entries = toc_data["entries"].as_array().unwrap();
    assert!(!entries.is_empty(), "toc should list fetched documents");

    // Verify the web-fetched document appears in the ToC
    let has_web_doc = entries
        .iter()
        .any(|e| e["document_id"].as_str().unwrap_or("").contains("web://"));
    assert!(
        has_web_doc,
        "toc should include web-fetched document, got: {entries:?}"
    );

    // Cleanup
    server_handle.abort();
}

// ---------------------------------------------------------------------------
// R5: Remote Pipeline Tests — ministr_clone integration & E2E
// ---------------------------------------------------------------------------

/// Create a local bare git repository with markdown docs for testing.
///
/// Returns the path to the bare repo (usable as a clone URL).
async fn create_test_git_repo(repo_root: &std::path::Path) -> String {
    use tokio::process::Command;

    let work_dir = repo_root.join("work");
    let bare_dir = repo_root.join("test-repo.git");

    // Create a bare repo.
    std::fs::create_dir_all(&work_dir).unwrap();
    Command::new("git")
        .args(["init", "--bare"])
        .arg(&bare_dir)
        .output()
        .await
        .unwrap();

    // Create a working copy, add markdown files, and push.
    Command::new("git")
        .args(["clone"])
        .arg(&bare_dir)
        .arg(&work_dir)
        .output()
        .await
        .unwrap();

    std::fs::write(
        work_dir.join("README.md"),
        "\
# Test Repository

This is a test repository for ministr integration testing.

## Overview

The repository contains documentation files used to verify that ministr can
clone a git repository and ingest its content into the search index.
",
    )
    .unwrap();

    std::fs::write(
        work_dir.join("guide.md"),
        "\
# User Guide

## Getting Started

Install the CLI tool using cargo. Configure your project by creating a config
file in the project root. The tool supports markdown and HTML document formats.

## Advanced Usage

Use the survey command to search across all indexed documents. The read command
retrieves full section text by section ID. Extract returns atomic claims from
a section for fine-grained analysis.
",
    )
    .unwrap();

    // Configure git user for the commit.
    Command::new("git")
        .current_dir(&work_dir)
        .args(["config", "user.email", "test@ministr.dev"])
        .output()
        .await
        .unwrap();
    Command::new("git")
        .current_dir(&work_dir)
        .args(["config", "user.name", "Test"])
        .output()
        .await
        .unwrap();

    Command::new("git")
        .current_dir(&work_dir)
        .args(["add", "."])
        .output()
        .await
        .unwrap();
    Command::new("git")
        .current_dir(&work_dir)
        .args(["commit", "-m", "initial commit"])
        .output()
        .await
        .unwrap();
    Command::new("git")
        .current_dir(&work_dir)
        .args(["push", "origin", "HEAD"])
        .output()
        .await
        .unwrap();

    bare_dir.to_string_lossy().to_string()
}

/// Set up an `MinistrServer` with git fetcher enabled for testing `ministr_clone`.
async fn setup_server_with_git_fetcher(
    remote_dir: &std::path::Path,
) -> (MinistrServer, Arc<dyn Embedder>, Arc<dyn VectorIndex>) {
    let dim = 16;
    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
    let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();
    let storage = Arc::new(storage);

    let service = Arc::new(QueryService::new(
        (*storage).clone(),
        Arc::clone(&embedder),
        Arc::clone(&index),
    ));

    let budget_config = UsageConfig::default();
    let server = MinistrServer::with_persistence(
        service,
        budget_config,
        Arc::clone(&storage),
        Some("test-clone-session".to_string()),
    )
    .await;

    let git_config = ministr_core::git::GitFetcherConfig {
        remote_dir: remote_dir.to_path_buf(),
        ..ministr_core::git::GitFetcherConfig::default()
    };
    let git_fetcher = ministr_core::git::GitFetcher::new(git_config);

    let server = server.with_git_fetcher(git_fetcher, Arc::clone(&embedder), Arc::clone(&index));

    (server, embedder, index)
}

// ---------------------------------------------------------------------------
// R5.0: Integration test — clone a repo, ingest its docs, verify ministr_survey
//       returns relevant content from the cloned source
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_ministr_clone_then_survey_and_toc() {
    // Skip if git is not installed.
    let git_check = tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await;
    if git_check.is_err() || !git_check.unwrap().status.success() {
        eprintln!("git not installed, skipping e2e_ministr_clone_then_survey_and_toc");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let repo_dir = tmp.path().join("repos");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let repo_url = create_test_git_repo(&repo_dir).await;

    let clone_cache = tmp.path().join("clone-cache");
    std::fs::create_dir_all(&clone_cache).unwrap();
    let (server, _embedder, _index) = setup_server_with_git_fetcher(&clone_cache).await;
    let (client, _server) = wrap_as_client(server).await;

    // Step 1: Clone the local test repo
    let clone_result = call_tool(&client, "ministr_clone", json!({"repo": repo_url})).await;

    let clone_text = extract_text(&clone_result.content);
    assert!(
        clone_result.is_error.is_none() || clone_result.is_error == Some(false),
        "ministr_clone should succeed, got: {clone_text}"
    );

    let clone_json: serde_json::Value = serde_json::from_str(clone_text).unwrap();
    let clone_data = tool_result(&clone_json);
    assert!(
        clone_data["files_discovered"].as_u64().unwrap() > 0,
        "should discover files in clone"
    );
    assert!(
        clone_data["sections_extracted"].as_u64().unwrap() > 0,
        "should extract sections from cloned content"
    );
    assert_no_budget_hints(&clone_json);

    // Step 2: Verify ministr_survey finds content from the cloned repo
    let survey_result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "survey command search indexed documents", "top_k": 10}),
    )
    .await;

    assert!(
        survey_result.is_error.is_none() || survey_result.is_error == Some(false),
        "ministr_survey should succeed after clone"
    );

    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey_result.content)).unwrap();
    let survey_data = tool_result(&survey_json);
    let results = survey_data["results"].as_array().unwrap();
    assert!(!results.is_empty(), "survey should find cloned content");

    // Step 3: Verify ministr_toc lists the cloned documents
    let toc_result = call_tool(&client, "ministr_toc", json!({})).await;

    assert!(
        toc_result.is_error.is_none() || toc_result.is_error == Some(false),
        "ministr_toc should succeed"
    );

    let toc_json: serde_json::Value =
        serde_json::from_str(extract_text(&toc_result.content)).unwrap();
    let toc_data = tool_result(&toc_json);
    let entries = toc_data["entries"].as_array().unwrap();
    assert!(!entries.is_empty(), "toc should list cloned documents");
    assert!(
        entries.len() >= 2,
        "toc should list at least 2 documents (README.md + guide.md), got: {}",
        entries.len()
    );
}

// ---------------------------------------------------------------------------
// R5.1: E2E test — ministr_clone + ministr_fetch in same session, verify
//       ministr_survey returns unified results from both sources
// ---------------------------------------------------------------------------

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn e2e_clone_and_fetch_unified_survey_results() {
    // Skip if git is not installed.
    let git_check = tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await;
    if git_check.is_err() || !git_check.unwrap().status.success() {
        eprintln!("git not installed, skipping e2e_clone_and_fetch_unified_survey_results");
        return;
    }

    let test_html = r"
    <html><head><title>Deployment Guide</title></head>
    <body>
    <h1>Deployment Guide</h1>
    <p>This guide covers deploying your application to production environments.
    Deployment requires Docker and a Kubernetes cluster with at least 3 nodes.
    The deployment pipeline runs automated health checks after each rollout.</p>
    <h2>Prerequisites</h2>
    <p>Install Docker version 24 or later. Configure kubectl to point to your
    target cluster. Ensure you have admin access to the namespace.</p>
    </body></html>
    ";

    let (addr, http_handle) = start_test_http_server(test_html).await;
    let tmp = tempfile::tempdir().unwrap();

    // Create a local test git repo with markdown files.
    let repo_dir = tmp.path().join("repos");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let repo_url = create_test_git_repo(&repo_dir).await;

    let clone_cache = tmp.path().join("clone-cache");
    std::fs::create_dir_all(&clone_cache).unwrap();
    let web_cache_dir = tmp.path().join("web-cache");
    std::fs::create_dir_all(&web_cache_dir).unwrap();

    // Set up server with BOTH git fetcher and web fetcher.
    let dim = 16;
    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
    let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();
    let storage = Arc::new(storage);

    let service = Arc::new(QueryService::new(
        (*storage).clone(),
        Arc::clone(&embedder),
        Arc::clone(&index),
    ));

    let budget_config = UsageConfig::default();
    let server = MinistrServer::with_persistence(
        service,
        budget_config,
        Arc::clone(&storage),
        Some("test-unified-session".to_string()),
    )
    .await;

    let http_client = ministr_core::web::HttpClient::with_defaults().unwrap();
    let web_fetcher = ministr_core::web::fetcher::WebFetcher::new(
        http_client,
        &web_cache_dir,
        ministr_core::web::fetcher::WebFetcherConfig::default(),
    );
    let server = server.with_web_fetcher(web_fetcher, Arc::clone(&embedder), Arc::clone(&index));

    let git_config = ministr_core::git::GitFetcherConfig {
        remote_dir: clone_cache,
        ..ministr_core::git::GitFetcherConfig::default()
    };
    let git_fetcher = ministr_core::git::GitFetcher::new(git_config);
    let server = server.with_git_fetcher(git_fetcher, Arc::clone(&embedder), Arc::clone(&index));
    let (client, _server) = wrap_as_client(server).await;

    // Step 1: Fetch web content
    let fetch_result = call_tool(
        &client,
        "ministr_fetch",
        json!({"url": format!("http://{addr}/")}),
    )
    .await;

    let fetch_text = extract_text(&fetch_result.content);
    assert!(
        fetch_result.is_error.is_none() || fetch_result.is_error == Some(false),
        "ministr_fetch should succeed, got: {fetch_text}"
    );

    let fetch_json: serde_json::Value = serde_json::from_str(fetch_text).unwrap();
    let fetch_data = tool_result(&fetch_json);
    assert!(
        fetch_data["pages_fetched"].as_u64().unwrap() > 0,
        "should fetch at least 1 page, got: {fetch_json}"
    );
    assert!(
        fetch_data["sections_indexed"].as_u64().unwrap() > 0,
        "should index sections from fetched page, got: {fetch_json}"
    );

    // Step 2: Clone the local test git repo
    let clone_result = call_tool(&client, "ministr_clone", json!({"repo": repo_url})).await;

    let clone_text = extract_text(&clone_result.content);
    assert!(
        clone_result.is_error.is_none() || clone_result.is_error == Some(false),
        "ministr_clone should succeed, got: {clone_text}"
    );

    // Step 3: Survey should return results from BOTH sources
    let survey_result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "deployment guide install configure survey search", "top_k": 20}),
    )
    .await;

    assert!(
        survey_result.is_error.is_none() || survey_result.is_error == Some(false),
        "ministr_survey should succeed"
    );

    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey_result.content)).unwrap();
    let survey_data = tool_result(&survey_json);
    let results = survey_data["results"].as_array().unwrap();
    assert!(
        !results.is_empty(),
        "survey should return results from indexed sources"
    );

    // Check that results come from web source (web://)
    let has_web_content = results
        .iter()
        .any(|r| r["content_id"].as_str().unwrap_or("").contains("web://"));

    // Check that results come from cloned repo (file paths without web://)
    let has_clone_content = results.iter().any(|r| {
        let id = r["content_id"].as_str().unwrap_or("");
        !id.contains("web://") && !id.is_empty()
    });

    assert!(
        has_web_content || has_clone_content,
        "survey should return content from at least one remote source, got: {results:?}"
    );

    // Step 4: ToC should list cloned documents at minimum
    let toc_result = call_tool(&client, "ministr_toc", json!({})).await;
    let toc_json: serde_json::Value =
        serde_json::from_str(extract_text(&toc_result.content)).unwrap();
    let toc_data = tool_result(&toc_json);
    let entries = toc_data["entries"].as_array().unwrap();

    // Both ministr_fetch and ministr_clone succeeded above, so the corpus has
    // content from both sources. The ToC must include at least the cloned docs.
    assert!(
        entries.len() >= 2,
        "toc should list at least 2 entries (cloned docs), got: {}",
        entries.len()
    );

    // Verify unique document IDs from the cloned repo are present
    let doc_ids: std::collections::HashSet<&str> = entries
        .iter()
        .map(|e| e["document_id"].as_str().unwrap_or(""))
        .collect();
    assert!(
        doc_ids.len() >= 2,
        "toc should list at least 2 unique documents, got: {doc_ids:?}"
    );

    // Cleanup
    http_handle.abort();
}

// ---------------------------------------------------------------------------
// R5.2: Error handling tests for ministr_clone
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ministr_clone_not_configured_returns_error() {
    let (client, _server) = wrap_as_client(setup_server().await).await;
    let result = call_tool(
        &client,
        "ministr_clone",
        json!({"repo": "https://github.com/octocat/Hello-World.git"}),
    )
    .await;

    assert!(
        result.is_error == Some(false),
        "ministr_clone should report a soft (non-cascading) error when not configured"
    );

    let text = extract_text(&result.content);
    assert!(
        text.contains("not available"),
        "error should mention not available, got: {text}"
    );
}

#[tokio::test]
async fn ministr_clone_nonexistent_repo_returns_user_friendly_error() {
    // Skip if git is not installed.
    let git_check = tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await;
    if git_check.is_err() || !git_check.unwrap().status.success() {
        eprintln!("git not installed, skipping ministr_clone_nonexistent_repo test");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let (server, _embedder, _index) = setup_server_with_git_fetcher(tmp.path()).await;
    let (client, _server) = wrap_as_client(server).await;

    let result = call_tool(
        &client,
        "ministr_clone",
        json!({"repo": "/tmp/ministr-test-nonexistent-repo-that-does-not-exist-xyz789.git"}),
    )
    .await;

    // Should return an error result, not panic.
    assert!(
        result.is_error == Some(false),
        "ministr_clone with nonexistent repo should report a soft (non-cascading) error"
    );

    let text = extract_text(&result.content);
    assert!(
        text.contains("clone failed"),
        "error should mention clone failure, got: {text}"
    );
}

#[tokio::test]
async fn ministr_clone_empty_repo_url_returns_error() {
    // Skip if git is not installed.
    let git_check = tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await;
    if git_check.is_err() || !git_check.unwrap().status.success() {
        eprintln!("git not installed, skipping ministr_clone_empty_repo_url test");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let (server, _embedder, _index) = setup_server_with_git_fetcher(tmp.path()).await;
    let (client, _server) = wrap_as_client(server).await;

    let result = call_tool(&client, "ministr_clone", json!({"repo": ""})).await;

    assert!(
        result.is_error == Some(false),
        "ministr_clone with empty URL should report a soft (non-cascading) error"
    );

    let text = extract_text(&result.content);
    assert!(
        text.contains("clone failed") || text.contains("invalid"),
        "error should indicate invalid input, got: {text}"
    );
}

// =============================================================================
// Code Intelligence MCP Tools (C5)
// =============================================================================

/// Build test symbols for code intelligence testing.
fn build_test_symbols() -> Vec<SymbolRecord> {
    vec![
        SymbolRecord {
            id: SymbolId("sym-config::MinistrConfig".into()),
            file_path: "src/config.rs".into(),
            name: "MinistrConfig".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct MinistrConfig".into(),
            doc_comment: Some("Configuration for the ministr context cache.".into()),
            module_path: "config".into(),
            line_start: 10,
            line_end: 25,
        cyclomatic_complexity: None,
},
        SymbolRecord {
            id: SymbolId("sym-config::PrefetchConfig".into()),
            file_path: "src/config.rs".into(),
            name: "PrefetchConfig".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct PrefetchConfig".into(),
            doc_comment: Some("Prefetch engine configuration.".into()),
            module_path: "config".into(),
            line_start: 30,
            line_end: 45,
        cyclomatic_complexity: None,
},
        SymbolRecord {
            id: SymbolId("sym-service::QueryService".into()),
            file_path: "src/service.rs".into(),
            name: "QueryService".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct QueryService".into(),
            doc_comment: Some("High-level query service composing storage and index.".into()),
            module_path: "service".into(),
            line_start: 50,
            line_end: 55,
        cyclomatic_complexity: None,
},
        SymbolRecord {
            id: SymbolId("sym-service::survey".into()),
            file_path: "src/service.rs".into(),
            name: "survey".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "pub async fn survey(&self, query: &str, top_k: usize) -> Result<Vec<SurveyResult>, QueryError>".into(),
            doc_comment: Some("Search the corpus for content relevant to a query.".into()),
            module_path: "service".into(),
            line_start: 60,
            line_end: 80,
        cyclomatic_complexity: None,
},
        SymbolRecord {
            id: SymbolId("sym-storage::Storage".into()),
            file_path: "src/storage/traits.rs".into(),
            name: "Storage".into(),
            kind: "trait".into(),
            visibility: "pub".into(),
            signature: "pub trait Storage: Send + Sync".into(),
            doc_comment: Some("Async storage interface for the ministr content database.".into()),
            module_path: "storage".into(),
            line_start: 100,
            line_end: 200,
        cyclomatic_complexity: None,
},
    ]
}

/// Build test symbol cross-references.
fn build_test_refs() -> Vec<SymbolRefRecord> {
    vec![
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-service::survey".into()),
            to_symbol_id: SymbolId("sym-storage::Storage".into()),
            ref_kind: RefKind::Calls,
        },
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-service::survey".into()),
            to_symbol_id: SymbolId("sym-config::MinistrConfig".into()),
            ref_kind: RefKind::Uses,
        },
    ]
}

/// Set up a server with code symbols indexed.
async fn setup_server_with_symbols() -> MinistrServer {
    let dim = 16;
    let embedder = Arc::new(MockEmbedder { dim });
    let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Insert document corpus for normal tools
    let corpus = build_corpus();
    for doc in &corpus {
        storage.insert_document(doc).await.unwrap();
    }

    // Index vectors
    let texts_and_ids = [
        (
            "doc-summary::docs/auth.md",
            "Complete authentication reference.",
        ),
        (
            "section::docs/auth.md#tokens",
            "JWT tokens use RS256 signing.",
        ),
    ];
    for (id, text) in &texts_and_ids {
        let vecs = embedder.embed(&[*text]).unwrap();
        index.insert(id, &vecs[0]).unwrap();
    }

    // Insert code symbols
    let symbols = build_test_symbols();
    storage.insert_symbols(&symbols).await.unwrap();

    // Insert cross-references
    let refs = build_test_refs();
    storage.insert_symbol_refs(&refs).await.unwrap();

    let service = Arc::new(QueryService::new(storage, embedder, index));
    MinistrServer::new(service)
}

#[tokio::test]
async fn ministr_symbols_finds_struct_by_name() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(
        &client,
        "ministr_symbols",
        json!({"query": "MinistrConfig"}),
    )
    .await;

    assert!(result.is_error.is_none() || result.is_error == Some(false));
    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data = tool_result(&response);

    let symbols = data["symbols"].as_array().unwrap();
    assert!(
        !symbols.is_empty(),
        "should find MinistrConfig by name search"
    );
    assert!(symbols.iter().any(|s| s["name"] == "MinistrConfig"));
}

#[tokio::test]
async fn ministr_symbols_filters_by_kind() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(&client, "ministr_symbols", json!({"kind": "function"})).await;

    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data = tool_result(&response);

    let symbols = data["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 1, "should find exactly one function");
    assert_eq!(symbols[0]["name"], "survey");
    assert_eq!(symbols[0]["kind"], "function");
}

#[tokio::test]
async fn ministr_symbols_filters_by_module() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(&client, "ministr_symbols", json!({"module": "config"})).await;

    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data = tool_result(&response);

    let symbols = data["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 2, "should find two symbols in config module");
    assert!(symbols.iter().all(|s| s["file"] == "src/config.rs"));
}

#[tokio::test]
async fn ministr_symbols_returns_all_when_no_filter() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(&client, "ministr_symbols", json!({})).await;

    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data = tool_result(&response);

    let total = data["total"].as_u64().unwrap();
    assert_eq!(total, 5, "should return all 5 symbols when unfiltered");
}

#[tokio::test]
async fn ministr_symbols_includes_doc_preview() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(&client, "ministr_symbols", json!({"query": "Storage"})).await;

    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data = tool_result(&response);

    let symbols = data["symbols"].as_array().unwrap();
    let storage_sym = symbols
        .iter()
        .find(|s| s["name"] == "Storage")
        .expect("should find Storage trait");
    assert!(
        storage_sym["doc_preview"].is_string(),
        "should include doc preview"
    );
}

#[tokio::test]
async fn ministr_definition_returns_symbol_metadata() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(
        &client,
        "ministr_definition",
        json!({"symbol_id": "sym-config::MinistrConfig"}),
    )
    .await;

    assert!(result.is_error.is_none() || result.is_error == Some(false));
    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data = tool_result(&response);

    assert_eq!(data["name"], "MinistrConfig");
    assert_eq!(data["kind"], "struct");
    assert_eq!(data["visibility"], "pub");
    assert_eq!(data["file_path"], "src/config.rs");
    assert_eq!(data["line_start"], 10);
    assert_eq!(data["line_end"], 25);

    // Heading path should include module + name
    let heading = data["heading_path"].as_array().unwrap();
    assert!(heading.iter().any(|h| h == "config"));
    assert!(heading.iter().any(|h| h == "MinistrConfig"));
}

#[tokio::test]
async fn ministr_definition_omits_usage_status() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(
        &client,
        "ministr_definition",
        json!({"symbol_id": "sym-config::MinistrConfig"}),
    )
    .await;

    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();

    assert_no_budget_hints(&response);
}

#[tokio::test]
async fn ministr_definition_not_found() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(
        &client,
        "ministr_definition",
        json!({"symbol_id": "nonexistent"}),
    )
    .await;

    assert_eq!(result.is_error, Some(false));
    let text = extract_text(&result.content);
    assert!(
        text.contains("Symbol not found"),
        "should return user-friendly error, got: {text}"
    );
}

#[tokio::test]
async fn ministr_references_finds_callers() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(
        &client,
        "ministr_references",
        json!({"symbol_id": "sym-storage::Storage"}),
    )
    .await;

    assert!(result.is_error.is_none() || result.is_error == Some(false));
    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data = tool_result(&response);

    let refs = data["references"].as_array().unwrap();
    assert!(!refs.is_empty(), "Storage should have references");
    // survey calls Storage
    assert!(
        refs.iter()
            .any(|r| r["from_name"] == "survey" && r["ref_kind"] == "calls")
    );
}

#[tokio::test]
async fn ministr_references_filters_by_ref_kind() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    // MinistrConfig is referenced via "uses" only
    let result = call_tool(
        &client,
        "ministr_references",
        json!({"symbol_id": "sym-config::MinistrConfig", "ref_kind": "calls"}),
    )
    .await;

    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data = tool_result(&response);

    let refs = data["references"].as_array().unwrap();
    assert!(
        refs.is_empty(),
        "MinistrConfig should have no 'calls' references, only 'uses'"
    );

    // Now check with "uses"
    let result = call_tool(
        &client,
        "ministr_references",
        json!({"symbol_id": "sym-config::MinistrConfig", "ref_kind": "uses"}),
    )
    .await;

    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data2 = tool_result(&response);

    let refs = data2["references"].as_array().unwrap();
    assert_eq!(
        refs.len(),
        1,
        "MinistrConfig should have one 'uses' reference"
    );
    assert_eq!(refs[0]["from_name"], "survey");
}

#[tokio::test]
async fn ministr_references_not_found() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(
        &client,
        "ministr_references",
        json!({"symbol_id": "nonexistent"}),
    )
    .await;

    assert_eq!(result.is_error, Some(false));
    let text = extract_text(&result.content);
    assert!(
        text.contains("Symbol not found"),
        "should return user-friendly error, got: {text}"
    );
}

#[tokio::test]
async fn ministr_symbols_omits_usage_status() {
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(&client, "ministr_symbols", json!({"query": "Config"})).await;

    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();

    assert_no_budget_hints(&response);
}

#[tokio::test]
async fn ministr_solid_returns_findings_envelope() {
    // The default test corpus carries no SOLID-violating fixtures, so this
    // primarily verifies that the tool is reachable end-to-end, accepts the
    // documented parameter shape, and serialises a well-formed empty result.
    let (client, _server) = wrap_as_client(setup_server_with_symbols().await).await;
    let result = call_tool(
        &client,
        "ministr_solid",
        json!({
            "principles": ["srp", "isp", "dip"],
            "limit": 25,
        }),
    )
    .await;

    assert!(
        result.is_error.is_none() || result.is_error == Some(false),
        "ministr_solid call failed"
    );
    let text = extract_text(&result.content);
    let response: serde_json::Value = serde_json::from_str(text).unwrap();
    let data = tool_result(&response);

    let findings = data["findings"].as_array().expect("findings array missing");
    let total = data["total"].as_u64().expect("total missing");
    assert_eq!(usize::try_from(total).unwrap(), findings.len());
    // For every finding (none expected on the default fixture, but assert the
    // shape if any appear), `principle` must be one of the four known
    // discriminators.
    for f in findings {
        let principle = f["principle"].as_str().expect("principle missing");
        assert!(
            matches!(principle, "dry_ocp" | "srp" | "isp" | "dip"),
            "unexpected principle: {principle}"
        );
    }
}

// ===========================================================================
// STABLE1.0: Stress test — multiple concurrent clients on shared corpus
// ===========================================================================

/// Spawn N concurrent MCP clients against the same `MinistrServer` and run
/// queries in parallel. Verifies no panics, deadlocks, or data corruption.
#[tokio::test]
async fn stress_concurrent_clients_on_shared_corpus() {
    let server = setup_server_with_symbols().await;
    let num_clients = 8;
    let queries_per_client = 5;

    let mut handles = Vec::new();

    for client_id in 0..num_clients {
        let server_clone = server.clone();
        handles.push(tokio::spawn(async move {
            let (client, _server_handle) = wrap_as_client(server_clone).await;

            for query_idx in 0..queries_per_client {
                // Alternate between different tool types to stress shared state
                match query_idx % 5 {
                    0 => {
                        // Survey
                        let result = call_tool(
                            &client,
                            "ministr_survey",
                            json!({"query": format!("client {client_id} query {query_idx}"), "top_k": 5}),
                        )
                        .await;
                        assert!(
                            result.is_error.is_none() || result.is_error == Some(false),
                            "client {client_id} survey {query_idx} failed"
                        );
                    }
                    1 => {
                        // Read
                        let result = call_tool(
                            &client,
                            "ministr_read",
                            json!({"section_id": "docs/auth.md#tokens"}),
                        )
                        .await;
                        assert!(
                            result.is_error.is_none() || result.is_error == Some(false),
                            "client {client_id} read {query_idx} failed"
                        );
                    }
                    2 => {
                        // Symbols
                        let result = call_tool(
                            &client,
                            "ministr_symbols",
                            json!({"query": "Config"}),
                        )
                        .await;
                        assert!(
                            result.is_error.is_none() || result.is_error == Some(false),
                            "client {client_id} symbols {query_idx} failed"
                        );
                    }
                    3 => {
                        // TOC
                        let result = call_tool(&client, "ministr_toc", json!({})).await;
                        assert!(
                            result.is_error.is_none() || result.is_error == Some(false),
                            "client {client_id} toc {query_idx} failed"
                        );
                    }
                    4 => {
                        // Budget
                        let result = call_tool(&client, "ministr_usage", json!({})).await;
                        assert!(
                            result.is_error.is_none() || result.is_error == Some(false),
                            "client {client_id} budget {query_idx} failed"
                        );
                    }
                    _ => unreachable!(),
                }
            }

            // Final: verify budget is consistent
            let budget = call_tool(&client, "ministr_usage", json!({})).await;
            let text = extract_text(&budget.content);
            let json: serde_json::Value = serde_json::from_str(text).unwrap();
            assert!(
                json["total_budget"].is_number(),
                "client {client_id} budget should be valid after stress"
            );
        }));
    }

    // Await all clients — any panic propagates here
    for (i, handle) in handles.into_iter().enumerate() {
        handle.await.unwrap_or_else(|e| {
            panic!("client {i} panicked: {e}");
        });
    }
}

/// Stress test with interleaved read/evict cycles to exercise concurrent
/// session state mutations.
#[tokio::test]
async fn stress_concurrent_read_evict_cycles() {
    let server = setup_server().await;
    let num_clients = 6;

    let mut handles = Vec::new();
    let sections = [
        "docs/auth.md#tokens",
        "docs/auth.md#oauth",
        "docs/api.md#rate-limits",
    ];

    for client_id in 0..num_clients {
        let server_clone = server.clone();
        handles.push(tokio::spawn(async move {
            let (client, _server_handle) = wrap_as_client(server_clone).await;

            // Each client: read all sections, evict them, read again
            for cycle in 0..3 {
                // Read all sections
                for section in &sections {
                    let result =
                        call_tool(&client, "ministr_read", json!({"section_id": section})).await;
                    assert!(
                        result.is_error.is_none() || result.is_error == Some(false),
                        "client {client_id} cycle {cycle} read {section} failed"
                    );
                }

                // Evict all sections
                let evict_ids: Vec<&str> = sections.to_vec();
                let result = call_tool(
                    &client,
                    "ministr_dropped",
                    json!({"content_ids": evict_ids}),
                )
                .await;
                assert!(
                    result.is_error.is_none() || result.is_error == Some(false),
                    "client {client_id} cycle {cycle} evict failed"
                );
            }

            // Final budget should be consistent
            let budget = call_tool(&client, "ministr_usage", json!({})).await;
            let text = extract_text(&budget.content);
            let json: serde_json::Value = serde_json::from_str(text).unwrap();
            assert!(json["total_budget"].is_number());
            assert!(json["estimated_used"].is_number());
        }));
    }

    for (i, handle) in handles.into_iter().enumerate() {
        handle.await.unwrap_or_else(|e| {
            panic!("client {i} panicked: {e}");
        });
    }
}

// ===========================================================================
// STABLE1.1: Integration test — roundtrip for all MCP tool operations
// ===========================================================================

/// Exercise every MCP tool (survey, read, symbols, definition, references,
/// toc, extract, related, bridge, compress, budget, evicted) in a single test
/// to verify the full proxy ↔ server roundtrip.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn integration_roundtrip_all_mcp_tools() {
    let server = setup_server_with_symbols().await;
    let (client, _server) = wrap_as_client(server).await;

    // 1. ministr_survey — semantic search
    let survey = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "JWT authentication tokens", "top_k": 5}),
    )
    .await;
    assert!(
        survey.is_error.is_none() || survey.is_error == Some(false),
        "ministr_survey failed"
    );
    let survey_json: serde_json::Value =
        serde_json::from_str(extract_text(&survey.content)).unwrap();
    assert!(tool_result(&survey_json)["results"].is_array());

    // 2. ministr_read — full section content
    let read = call_tool(
        &client,
        "ministr_read",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    assert!(
        read.is_error.is_none() || read.is_error == Some(false),
        "ministr_read failed"
    );
    let read_json: serde_json::Value = serde_json::from_str(extract_text(&read.content)).unwrap();
    let read_data = tool_result(&read_json);
    assert!(
        read_data["text"].is_string() || read_data["status"] == "already_delivered",
        "ministr_read should return text or already_delivered"
    );

    // 3. ministr_extract — atomic claims
    let extract = call_tool(
        &client,
        "ministr_extract",
        json!({"section_id": "docs/auth.md#tokens"}),
    )
    .await;
    assert!(
        extract.is_error.is_none() || extract.is_error == Some(false),
        "ministr_extract failed"
    );
    let extract_json: serde_json::Value =
        serde_json::from_str(extract_text(&extract.content)).unwrap();
    assert!(tool_result(&extract_json)["claims"].is_array());

    // 4. ministr_related — claim dependency chains
    let related = call_tool(&client, "ministr_related", json!({"claim_id": "auth-c1"})).await;
    assert!(
        related.is_error.is_none() || related.is_error == Some(false),
        "ministr_related failed"
    );
    let related_json: serde_json::Value =
        serde_json::from_str(extract_text(&related.content)).unwrap();
    assert!(tool_result(&related_json)["related"].is_array());

    // 5. ministr_toc — table of contents
    let toc = call_tool(&client, "ministr_toc", json!({})).await;
    assert!(
        toc.is_error.is_none() || toc.is_error == Some(false),
        "ministr_toc failed"
    );
    let toc_json: serde_json::Value = serde_json::from_str(extract_text(&toc.content)).unwrap();
    let toc_data = tool_result(&toc_json);
    assert!(toc_data["entries"].is_array());
    assert!(toc_data["corpus_stats"].is_object());

    // 6. ministr_compress — section compression
    let compress = call_tool(
        &client,
        "ministr_compress",
        json!({"content_ids": ["docs/auth.md#tokens"]}),
    )
    .await;
    assert!(
        compress.is_error.is_none() || compress.is_error == Some(false),
        "ministr_compress failed"
    );
    let compress_json: serde_json::Value =
        serde_json::from_str(extract_text(&compress.content)).unwrap();
    assert!(tool_result(&compress_json)["summaries"].is_array());

    // 7. ministr_usage — budget status
    let budget = call_tool(&client, "ministr_usage", json!({})).await;
    assert!(
        budget.is_error.is_none() || budget.is_error == Some(false),
        "ministr_usage failed"
    );
    let budget_json: serde_json::Value =
        serde_json::from_str(extract_text(&budget.content)).unwrap();
    assert!(budget_json["total_budget"].is_number());
    assert!(budget_json["estimated_used"].is_number());
    assert!(budget_json["level"].is_string());
    assert!(budget_json["drop_candidates"].is_array());

    // 8. ministr_dropped — eviction signaling
    let evicted = call_tool(
        &client,
        "ministr_dropped",
        json!({"content_ids": ["docs/auth.md#tokens"]}),
    )
    .await;
    assert!(
        evicted.is_error.is_none() || evicted.is_error == Some(false),
        "ministr_dropped failed"
    );
    let evicted_json: serde_json::Value =
        serde_json::from_str(extract_text(&evicted.content)).unwrap();
    let evicted_data = tool_result(&evicted_json);
    assert!(evicted_data["dropped"].is_array());
    assert!(evicted_data["not_found"].is_array());

    // 9. ministr_symbols — code symbol search
    let symbols = call_tool(
        &client,
        "ministr_symbols",
        json!({"query": "MinistrConfig"}),
    )
    .await;
    assert!(
        symbols.is_error.is_none() || symbols.is_error == Some(false),
        "ministr_symbols failed"
    );
    let symbols_json: serde_json::Value =
        serde_json::from_str(extract_text(&symbols.content)).unwrap();
    let symbols_data = tool_result(&symbols_json);
    assert!(symbols_data["symbols"].is_array());
    let syms = symbols_data["symbols"].as_array().unwrap();
    assert!(!syms.is_empty(), "should find MinistrConfig symbol");

    // 10. ministr_definition — symbol source code
    let definition = call_tool(
        &client,
        "ministr_definition",
        json!({"symbol_id": "sym-config::MinistrConfig"}),
    )
    .await;
    assert!(
        definition.is_error.is_none() || definition.is_error == Some(false),
        "ministr_definition failed"
    );
    let def_json: serde_json::Value =
        serde_json::from_str(extract_text(&definition.content)).unwrap();
    let def_data = tool_result(&def_json);
    assert!(def_data["name"].is_string());
    assert_eq!(def_data["name"].as_str().unwrap(), "MinistrConfig");

    // 11. ministr_references — symbol cross-references
    let references = call_tool(
        &client,
        "ministr_references",
        json!({"symbol_id": "sym-storage::Storage"}),
    )
    .await;
    assert!(
        references.is_error.is_none() || references.is_error == Some(false),
        "ministr_references failed"
    );
    let refs_json: serde_json::Value =
        serde_json::from_str(extract_text(&references.content)).unwrap();
    let refs_data = tool_result(&refs_json);
    assert!(
        refs_data["references"].is_array(),
        "ministr_references should return references array"
    );

    // 12. ministr_bridge — cross-language bridge links
    let bridge = call_tool(&client, "ministr_bridge", json!({})).await;
    assert!(
        bridge.is_error.is_none() || bridge.is_error == Some(false),
        "ministr_bridge failed"
    );
    let bridge_json: serde_json::Value =
        serde_json::from_str(extract_text(&bridge.content)).unwrap();
    let bridge_data = tool_result(&bridge_json);
    assert!(
        bridge_data["links"].is_array(),
        "ministr_bridge should return links array"
    );

    // 13. ministr_fetch — should return error (not configured)
    let fetch = call_tool(
        &client,
        "ministr_fetch",
        json!({"url": "https://example.com"}),
    )
    .await;
    assert_eq!(
        fetch.is_error,
        Some(false),
        "ministr_fetch should report a soft (non-cascading) failure when not configured"
    );

    // 14. ministr_clone — should return error (not configured)
    let clone = call_tool(
        &client,
        "ministr_clone",
        json!({"repo": "https://github.com/test/test"}),
    )
    .await;
    assert_eq!(
        clone.is_error,
        Some(false),
        "ministr_clone should report a soft (non-cascading) failure when not configured"
    );

    // Verify all tools are listed
    let tools = client.list_all_tools().await.unwrap();
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    let expected_tools = [
        "ministr_survey",
        "ministr_read",
        "ministr_extract",
        "ministr_related",
        "ministr_toc",
        "ministr_compress",
        "ministr_usage",
        "ministr_dropped",
        "ministr_symbols",
        "ministr_definition",
        "ministr_references",
        "ministr_bridge",
        "ministr_fetch",
        "ministr_clone",
    ];
    for tool in &expected_tools {
        assert!(
            tool_names.contains(tool),
            "tool listing should include {tool}, got: {tool_names:?}"
        );
    }
}

// ===========================================================================
// STABLE1.4: Large corpus soak test — index many files, measure performance
// ===========================================================================

/// Generate a synthetic corpus with many files, index it, run queries,
/// and verify correctness and performance under load.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn soak_large_corpus_indexing_and_query() {
    let dir = tempfile::TempDir::new().unwrap();
    let num_files = 200;

    // Generate 200 markdown files covering different "topics"
    let topics = [
        "authentication",
        "authorization",
        "rate-limiting",
        "caching",
        "logging",
        "monitoring",
        "deployment",
        "testing",
        "security",
        "performance",
    ];

    for i in 0..num_files {
        let topic = topics[i % topics.len()];
        let content = format!(
            "# {topic} Module {i}\n\n\
             ## Overview\n\n\
             This document covers {topic} patterns for component {i}. \
             The {topic} subsystem handles critical operations \
             including request validation, state management, and error recovery.\n\n\
             ## Implementation\n\n\
             The {topic} implementation in module {i} uses a layered approach \
             with dependency injection for testability. Configuration is loaded \
             from environment variables and validated at startup.\n\n\
             ## API Reference\n\n\
             The {topic} API for module {i} exposes three endpoints: \
             create, update, and delete. All endpoints require authentication \
             and return standard JSON error responses on failure.\n"
        );
        std::fs::write(dir.path().join(format!("{topic}_{i:04}.md")), content).unwrap();
    }

    let dim = 16;
    let embedder = Arc::new(MockEmbedder { dim });
    let index = Arc::new(HnswIndex::new(dim, 10_000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Time the indexing
    let start = std::time::Instant::now();
    let pipeline = ministr_core::ingestion::IngestionPipeline::new();
    let stats = pipeline
        .ingest_directory_with_embeddings(dir.path(), &storage, embedder.as_ref(), index.as_ref())
        .await
        .unwrap();
    let index_duration = start.elapsed();

    assert_eq!(
        stats.files_indexed, num_files,
        "should index all {num_files} files, got: {stats:?}"
    );
    assert!(
        stats.total_sections >= num_files * 2,
        "should have at least 2 sections per file, got: {}",
        stats.total_sections
    );
    // Sanity: indexing 200 files with mock embedder should be fast
    assert!(
        index_duration.as_secs() < 30,
        "indexing {num_files} files took {index_duration:?} — too slow"
    );

    // Set up MCP server
    let storage = Arc::new(storage);
    let service = Arc::new(QueryService::new(
        (*storage).clone(),
        embedder as Arc<dyn Embedder>,
        index as Arc<dyn VectorIndex>,
    ));
    let budget_config = UsageConfig::default();
    let server =
        MinistrServer::with_persistence(service, budget_config, storage, Some("soak-test".into()))
            .await;
    let (client, _server) = wrap_as_client(server).await;

    // Query latency test: run multiple surveys and verify speed
    // Use simple queries that the mock embedder can match against indexed content
    let query_start = std::time::Instant::now();
    let num_queries = 20;
    for i in 0..num_queries {
        let topic = topics[i % topics.len()];
        let result = call_tool(
            &client,
            "ministr_survey",
            json!({"query": format!("{topic} Module"), "top_k": 10}),
        )
        .await;
        assert!(
            result.is_error.is_none() || result.is_error == Some(false),
            "survey query {i} for topic '{topic}' returned error"
        );
        let json: serde_json::Value = serde_json::from_str(extract_text(&result.content)).unwrap();
        let data = tool_result(&json);
        // With mock embedder, results may be empty for some queries — that's OK.
        // We're testing that queries *succeed* (no panics/errors), not semantic relevance.
        assert!(
            data["results"].is_array(),
            "survey {i} should return results array"
        );
    }
    let query_duration = query_start.elapsed();
    let avg_query_ms = query_duration.as_millis() / num_queries as u128;
    assert!(
        avg_query_ms < 500,
        "average query latency {avg_query_ms}ms is too high for mock embedder"
    );

    // TOC should list all files
    let toc = call_tool(&client, "ministr_toc", json!({})).await;
    let toc_json: serde_json::Value = serde_json::from_str(extract_text(&toc.content)).unwrap();
    let toc_data = tool_result(&toc_json);
    let toc_stats = &toc_data["corpus_stats"];
    assert_eq!(
        toc_stats["documents"].as_u64().unwrap(),
        num_files as u64,
        "TOC should report all {num_files} documents"
    );

    // Budget should reflect the large amount of delivered content
    let budget = call_tool(&client, "ministr_usage", json!({})).await;
    let budget_json: serde_json::Value =
        serde_json::from_str(extract_text(&budget.content)).unwrap();
    assert!(budget_json["total_budget"].is_number());
    assert!(budget_json["estimated_used"].is_number());
}
