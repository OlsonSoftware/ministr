//! MCP server implementation for iris.
//!
//! Implements the rmcp `ServerHandler` trait with `#[tool]` macro-based
//! tool registration. The server exposes iris tools (`iris_survey`,
//! `iris_read`, `iris_extract`) over the MCP protocol.

use std::sync::Arc;

use rmcp::ServerHandler;
use rmcp::model::{
    CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::schemars;
use rmcp::tool;
use serde::Deserialize;

use iris_core::service::QueryService;

/// MCP server that exposes iris context-cache tools to LLM agents.
///
/// `IrisServer` adapts the [`QueryService`] to the MCP protocol.
/// It handles tool registration, request routing, and response formatting.
#[derive(Clone)]
pub struct IrisServer {
    service: Arc<QueryService>,
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
    #[tool(
        name = "iris_survey",
        description = "Search the indexed corpus for sections relevant to a natural language query. Returns ranked summaries with relevance scores."
    )]
    async fn survey(
        &self,
        #[tool(aggr)] params: SurveyParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        let top_k = params.top_k.unwrap_or(10);

        match self.service.survey(&params.query, top_k).await {
            Ok(results) => {
                let json = serde_json::to_string_pretty(&results)
                    .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {e}\"}}"));
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    /// Read the full text of a section by its hierarchical ID.
    ///
    /// Returns the complete section content with heading path and
    /// the number of claims available for extraction.
    #[tool(
        name = "iris_read",
        description = "Read the full text of a section by its hierarchical ID. Returns content with heading path and available claims count."
    )]
    async fn read(&self, #[tool(aggr)] params: ReadParams) -> Result<CallToolResult, rmcp::Error> {
        match self.service.read_section(&params.section_id).await {
            Ok(detail) => {
                let json = serde_json::to_string_pretty(&detail)
                    .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {e}\"}}"));
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
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
        match self
            .service
            .extract_claims(&params.section_id, params.query.as_deref())
            .await
        {
            Ok(claims) => {
                let json = serde_json::to_string_pretty(&claims)
                    .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {e}\"}}"));
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }
}

impl IrisServer {
    /// Create a new iris MCP server instance backed by the given query service.
    #[must_use]
    pub fn new(service: Arc<QueryService>) -> Self {
        Self { service }
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

    async fn setup_server() -> IrisServer {
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let doc = DocumentTree {
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
        };
        storage.insert_document(&doc).await.unwrap();

        // Insert vectors
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

    #[tokio::test]
    async fn survey_returns_json_results() {
        let server = setup_server().await;
        let params = SurveyParams {
            query: "JWT authentication tokens".to_string(),
            top_k: Some(5),
        };
        let result = server.survey(params).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));
        assert!(!result.content.is_empty());

        // The content should be valid JSON
        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("survey output should be valid JSON: {e}\n{text}"));
        assert!(parsed.is_array(), "survey should return a JSON array");
    }

    #[tokio::test]
    async fn read_returns_section_json() {
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
    async fn extract_returns_claims_json() {
        let server = setup_server().await;
        let params = ExtractParams {
            section_id: "docs/auth.md#tokens".to_string(),
            query: Some("signing algorithm".to_string()),
        };
        let result = server.extract(params).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));

        let text = extract_text(&result.content);
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        // With a query, claims should have relevance scores
        assert!(arr[0]["relevance"].is_number());
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
}
