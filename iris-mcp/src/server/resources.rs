//! MCP resource and completion handlers for the iris server.
//!
//! These `impl IrisServer` methods handle resource reading (`mcp://server-card.json`,
//! `iris://status`, `iris://corpus/{path}`) and completion requests for section IDs
//! and corpus paths.

use rmcp::ServerHandler;
use rmcp::model::{CompletionInfo, ErrorData as McpError, ReadResourceResult, ResourceContents};

use iris_core::storage::Storage;
use iris_core::token::count_tokens;

use super::IrisServer;

impl IrisServer {
    // ── Completion helpers ────────────────────────────────────────────

    /// Complete section IDs by fuzzy-matching the partial value.
    pub(super) async fn complete_section_ids(&self, partial: &str) -> Vec<String> {
        let storage = self.service.storage();
        let documents = storage.list_documents().await.unwrap_or_default();
        let lower = partial.to_lowercase();
        let mut results = Vec::new();
        for doc in &documents {
            let sections = storage.list_sections(&doc.id).await.unwrap_or_default();
            for section in sections {
                if section.id.0.to_lowercase().contains(&lower) {
                    results.push(section.id.0);
                    if results.len() >= CompletionInfo::MAX_VALUES {
                        return results;
                    }
                }
            }
        }
        results
    }

    /// Complete corpus document paths for `iris://corpus/{path}` resources.
    pub(super) async fn complete_corpus_paths(&self, partial: &str) -> Vec<String> {
        let storage = self.service.storage();
        let documents = storage.list_documents().await.unwrap_or_default();
        let lower = partial.to_lowercase();
        documents
            .into_iter()
            .filter(|d| d.source_path.to_lowercase().contains(&lower))
            .take(CompletionInfo::MAX_VALUES)
            .map(|d| d.source_path)
            .collect()
    }

    /// Compute the total token overhead of all registered tool schemas.
    ///
    /// Concatenates tool names, descriptions, and parameter descriptions,
    /// then counts tokens using `cl100k_base`. Cached via `OnceLock` since
    /// schemas are immutable after initialization.
    pub(super) fn schema_token_overhead(&self) -> (usize, usize) {
        use std::sync::OnceLock;
        static CACHED: OnceLock<(usize, usize)> = OnceLock::new();
        *CACHED.get_or_init(|| {
            let tools = self.tool_router.list_all();
            let tool_count = tools.len();
            let mut schema_text = String::new();
            for tool in &tools {
                schema_text.push_str(&tool.name);
                schema_text.push(' ');
                if let Some(desc) = &tool.description {
                    schema_text.push_str(desc);
                    schema_text.push(' ');
                }
                if let Ok(json) = serde_json::to_string(&*tool.input_schema) {
                    schema_text.push_str(&json);
                    schema_text.push(' ');
                }
            }
            let tokens = count_tokens(&schema_text);
            (tokens, tool_count)
        })
    }

    /// Build the `mcp://server-card.json` server card (SEP-1649).
    ///
    /// Returns a structured metadata document describing this server's identity,
    /// protocol version, capabilities, extensions, and full tool catalog. Clients
    /// can read this resource to discover server features without completing the
    /// initialization handshake.
    pub(super) fn build_server_card(&self) -> serde_json::Value {
        let info = self.get_info();

        let tools: Vec<serde_json::Value> = self
            .tool_router
            .list_all()
            .into_iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                })
            })
            .collect();

        let mut capabilities = serde_json::json!({});
        if info.capabilities.tools.is_some() {
            capabilities["tools"] = serde_json::json!({ "listChanged": true });
        }
        if let Some(ref res) = info.capabilities.resources {
            capabilities["resources"] = serde_json::json!({
                "subscribe": res.subscribe.unwrap_or(false),
                "listChanged": res.list_changed.unwrap_or(false),
            });
        }
        if info.capabilities.prompts.is_some() {
            capabilities["prompts"] = serde_json::json!({ "listChanged": true });
        }
        if info.capabilities.tasks.is_some() {
            capabilities["tasks"] = serde_json::json!({});
        }
        if info.capabilities.completions.is_some() {
            capabilities["completions"] = serde_json::json!({});
        }
        if let Some(ref extensions) = info.capabilities.extensions {
            capabilities["extensions"] = serde_json::to_value(extensions).unwrap_or_default();
        }

        serde_json::json!({
            "$schema": "https://static.modelcontextprotocol.io/schemas/mcp-server-card/v1.json",
            "version": "1.0",
            "protocolVersion": info.protocol_version.to_string(),
            "serverInfo": {
                "name": info.server_info.name,
                "version": info.server_info.version,
                "description": info.server_info.description,
            },
            "capabilities": capabilities,
            "tools": tools,
        })
    }

    /// Read the `mcp://server-card.json` resource content.
    pub(super) fn read_server_card_resource(&self) -> ReadResourceResult {
        let card = self.build_server_card();
        let text = serde_json::to_string_pretty(&card).unwrap_or_default();
        ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
            meta: None,
            uri: "mcp://server-card.json".to_string(),
            mime_type: Some("application/json".to_string()),
            text,
        }])
    }

    /// Build the `iris://status` resource content.
    pub(super) async fn read_status_resource(&self) -> Result<ReadResourceResult, McpError> {
        let index = self.service.index();
        let reg = self.registry.lock().await;
        let entry = reg
            .get_session(&self.active_session_id)
            .expect("active session exists");

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
                "id": entry.session.id.to_string(),
                "delivered_count": entry.session.delivered_count(),
                "federation": {
                    "total_sessions": reg.session_count(),
                    "session_ids": reg.session_ids(),
                },
            },
            "budget": entry.budget.budget_status(),
        });

        if let Some(stats) = analytics_stats {
            status["analytics"] = serde_json::json!({
                "total_accesses": stats.total_accesses,
                "unique_sections_accessed": stats.unique_sections_accessed,
                "co_access_pairs": stats.co_access_pairs,
            });
        }

        let text = serde_json::to_string_pretty(&status).unwrap_or_default();
        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                meta: None,
                uri: "iris://status".to_string(),
                mime_type: Some("application/json".to_string()),
                text,
            },
        ]))
    }

    /// Build the `iris://corpus/{path}` resource content.
    pub(super) async fn read_corpus_resource(
        &self,
        path: &str,
    ) -> Result<ReadResourceResult, McpError> {
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
        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                meta: None,
                uri,
                mime_type: Some("application/json".to_string()),
                text,
            },
        ]))
    }
}
