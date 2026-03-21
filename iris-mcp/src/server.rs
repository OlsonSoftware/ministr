//! MCP server implementation for iris.
//!
//! Implements the rmcp `ServerHandler` trait with `#[tool]` macro-based
//! tool registration. The server exposes iris tools (`iris_survey`,
//! `iris_read`, `iris_extract`) over the MCP protocol.

// Tool stubs are async per rmcp's #[tool] contract; allow until wired to real services.
#![allow(clippy::unused_async)]

use rmcp::ServerHandler;
use rmcp::model::{
    CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::schemars;
use rmcp::tool;
use serde::Deserialize;

/// MCP server that exposes iris context-cache tools to LLM agents.
///
/// `IrisServer` adapts iris-core service traits to the MCP protocol.
/// It handles tool registration, request routing, and response formatting.
#[derive(Clone)]
pub struct IrisServer;

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
        Ok(CallToolResult::success(vec![Content::text(format!(
            "iris_survey not yet implemented. query={:?}, top_k={top_k}",
            params.query,
        ))]))
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
        Ok(CallToolResult::success(vec![Content::text(format!(
            "iris_read not yet implemented. section_id={:?}",
            params.section_id,
        ))]))
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
        Ok(CallToolResult::success(vec![Content::text(format!(
            "iris_extract not yet implemented. section_id={:?}, query={:?}",
            params.section_id, params.query,
        ))]))
    }
}

impl IrisServer {
    /// Create a new iris MCP server instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for IrisServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_info_has_correct_name_and_version() {
        let server = IrisServer::new();
        let info = server.get_info();

        assert_eq!(info.server_info.name, "iris");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn server_info_enables_tools_capability() {
        let server = IrisServer::new();
        let info = server.get_info();

        assert!(
            info.capabilities.tools.is_some(),
            "tools capability should be enabled"
        );
    }

    #[test]
    fn server_info_has_instructions() {
        let server = IrisServer::new();
        let info = server.get_info();

        let instructions = info.instructions.expect("instructions should be set");
        assert!(
            instructions.contains("iris_survey"),
            "instructions should mention iris_survey"
        );
        assert!(
            instructions.contains("iris_read"),
            "instructions should mention iris_read"
        );
        assert!(
            instructions.contains("iris_extract"),
            "instructions should mention iris_extract"
        );
    }

    #[test]
    fn server_info_uses_latest_protocol() {
        let server = IrisServer::new();
        let info = server.get_info();

        assert_eq!(info.protocol_version, ProtocolVersion::LATEST);
    }

    #[tokio::test]
    async fn survey_returns_placeholder() {
        let server = IrisServer::new();
        let params = SurveyParams {
            query: "test query".to_string(),
            top_k: Some(5),
        };
        let result = server.survey(params).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));
        assert!(!result.content.is_empty());
    }

    #[tokio::test]
    async fn read_returns_placeholder() {
        let server = IrisServer::new();
        let params = ReadParams {
            section_id: "doc.md#section".to_string(),
        };
        let result = server.read(params).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));
        assert!(!result.content.is_empty());
    }

    #[tokio::test]
    async fn extract_returns_placeholder() {
        let server = IrisServer::new();
        let params = ExtractParams {
            section_id: "doc.md#section".to_string(),
            query: Some("test".to_string()),
        };
        let result = server.extract(params).await.unwrap();

        assert!(result.is_error.is_none() || result.is_error == Some(false));
        assert!(!result.content.is_empty());
    }
}
