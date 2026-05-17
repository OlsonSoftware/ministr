//! MCP elicitation types and helpers for interactive agent workflows.
//!
//! Provides typed elicitation schemas for three use cases:
//! - **Budget pressure**: ask which sections to evict when pressure is elevated
//! - **Compression mode**: confirm before expensive abstractive compression
//! - **Search disambiguation**: refine ambiguous survey queries
//!
//! All types implement `JsonSchema` + `Deserialize` and are marked with
//! `elicit_safe!()` for use with rmcp's typed `peer.elicit::<T>()` API.

use rmcp::RoleServer;
use rmcp::service::{ElicitationError, ElicitationMode, Peer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Elicitation schema types
// ---------------------------------------------------------------------------

/// Agent's choice of which sections to evict under budget pressure.
///
/// Uses a comma-separated string for content IDs because MCP elicitation
/// schemas require flat objects with primitive properties only.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EvictionChoice {
    /// Comma-separated content IDs to evict (from the candidates list).
    #[schemars(description = "Comma-separated content IDs to drop from context")]
    pub content_ids: String,
}
rmcp::elicit_safe!(EvictionChoice);

impl EvictionChoice {
    /// Parse the comma-separated `content_ids` into individual IDs.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.content_ids
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Confirmation before running expensive abstractive compression.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CompressionConfirmation {
    /// Whether to proceed with compression.
    #[schemars(description = "true to proceed, false to skip")]
    pub proceed: bool,
    /// Preferred compression mode: "abstractive" (LLM-assisted, 90%+) or
    /// "extractive" (TF-IDF, 60-80%).
    #[schemars(description = "Compression mode: 'abstractive' or 'extractive'")]
    pub mode: String,
}
rmcp::elicit_safe!(CompressionConfirmation);

/// Refinement for an ambiguous search query.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchRefinement {
    /// A more specific version of the original query, or empty to keep results.
    #[schemars(description = "Refined query string, or empty to accept current results")]
    pub refined_query: String,
}
rmcp::elicit_safe!(SearchRefinement);

// ---------------------------------------------------------------------------
// Helper: try_elicit with graceful fallback
// ---------------------------------------------------------------------------

/// Attempt to elicit a typed response from the agent via MCP elicitation.
///
/// Returns `Ok(Some(T))` if the agent accepted and provided valid data,
/// `Ok(None)` if elicitation is unavailable, the agent declined/cancelled,
/// or any error occurred (logged but not propagated — callers fall back to
/// default behavior).
pub async fn try_elicit<T>(peer: &Peer<RoleServer>, message: &str) -> Option<T>
where
    T: rmcp::service::ElicitationSafe + for<'de> Deserialize<'de>,
{
    if !peer
        .supported_elicitation_modes()
        .contains(&ElicitationMode::Form)
    {
        debug!("client does not support form elicitation, skipping");
        return None;
    }

    match peer.elicit::<T>(message).await {
        Ok(Some(data)) => {
            debug!("elicitation accepted");
            Some(data)
        }
        Ok(None) => {
            debug!("elicitation returned no content");
            None
        }
        Err(ElicitationError::UserDeclined) => {
            debug!("agent declined elicitation");
            None
        }
        Err(ElicitationError::UserCancelled) => {
            debug!("agent cancelled elicitation");
            None
        }
        Err(ElicitationError::CapabilityNotSupported) => {
            debug!("client does not support elicitation capability");
            None
        }
        Err(e) => {
            warn!(error = %e, "elicitation failed, falling back to default behavior");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Schema generation tests — verify JSON Schema output matches MCP expectations
    // -----------------------------------------------------------------------

    #[test]
    fn eviction_choice_schema_is_object() {
        let schema = schemars::schema_for!(EvictionChoice);
        let root = serde_json::to_value(schema).unwrap();
        assert_eq!(root["type"], "object");
        assert!(root["properties"]["content_ids"].is_object());
    }

    #[test]
    fn compression_confirmation_schema_is_object() {
        let schema = schemars::schema_for!(CompressionConfirmation);
        let root = serde_json::to_value(schema).unwrap();
        assert_eq!(root["type"], "object");
        assert!(root["properties"]["proceed"].is_object());
        assert!(root["properties"]["mode"].is_object());
    }

    #[test]
    fn search_refinement_schema_is_object() {
        let schema = schemars::schema_for!(SearchRefinement);
        let root = serde_json::to_value(schema).unwrap();
        assert_eq!(root["type"], "object");
        assert!(root["properties"]["refined_query"].is_object());
    }

    // -----------------------------------------------------------------------
    // Schema property details — descriptions are propagated correctly
    // -----------------------------------------------------------------------

    #[test]
    fn eviction_choice_schema_has_description() {
        let schema = schemars::schema_for!(EvictionChoice);
        let root = serde_json::to_value(schema).unwrap();
        let desc = root["properties"]["content_ids"]["description"]
            .as_str()
            .unwrap_or("");
        assert!(
            desc.contains("drop"),
            "expected description to mention dropping content, got: {desc}"
        );
    }

    #[test]
    fn eviction_choice_content_ids_is_string_type() {
        let schema = schemars::schema_for!(EvictionChoice);
        let root = serde_json::to_value(schema).unwrap();
        assert_eq!(
            root["properties"]["content_ids"]["type"], "string",
            "content_ids must be a flat string for MCP elicitation compatibility"
        );
    }

    #[test]
    fn compression_confirmation_schema_has_required_fields() {
        let schema = schemars::schema_for!(CompressionConfirmation);
        let root = serde_json::to_value(schema).unwrap();
        let required = root["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"proceed"), "missing required 'proceed'");
        assert!(names.contains(&"mode"), "missing required 'mode'");
    }

    // -----------------------------------------------------------------------
    // Roundtrip serialization — simulates client response parsing
    // -----------------------------------------------------------------------

    #[test]
    fn eviction_choice_roundtrip() {
        let original = EvictionChoice {
            content_ids: "docs/auth.md#tokens, docs/api.md#rate-limits".into(),
        };
        let json = serde_json::to_value(&original).unwrap();
        let parsed: EvictionChoice = serde_json::from_value(json).unwrap();
        let ids = parsed.ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "docs/auth.md#tokens");
        assert_eq!(ids[1], "docs/api.md#rate-limits");
    }

    #[test]
    fn compression_confirmation_roundtrip() {
        let original = CompressionConfirmation {
            proceed: true,
            mode: "abstractive".into(),
        };
        let json = serde_json::to_value(&original).unwrap();
        let parsed: CompressionConfirmation = serde_json::from_value(json).unwrap();
        assert!(parsed.proceed);
        assert_eq!(parsed.mode, "abstractive");
    }

    #[test]
    fn compression_confirmation_extractive() {
        let json = serde_json::json!({
            "proceed": true,
            "mode": "extractive"
        });
        let parsed: CompressionConfirmation = serde_json::from_value(json).unwrap();
        assert!(parsed.proceed);
        assert_eq!(parsed.mode, "extractive");
    }

    #[test]
    fn compression_confirmation_decline() {
        let json = serde_json::json!({
            "proceed": false,
            "mode": "abstractive"
        });
        let parsed: CompressionConfirmation = serde_json::from_value(json).unwrap();
        assert!(!parsed.proceed);
    }

    #[test]
    fn search_refinement_roundtrip() {
        let original = SearchRefinement {
            refined_query: "rust async traits".into(),
        };
        let json = serde_json::to_value(&original).unwrap();
        let parsed: SearchRefinement = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.refined_query, "rust async traits");
    }

    #[test]
    fn search_refinement_empty_keeps_results() {
        let json = serde_json::json!({
            "refined_query": ""
        });
        let parsed: SearchRefinement = serde_json::from_value(json).unwrap();
        assert!(parsed.refined_query.is_empty());
    }

    #[test]
    fn eviction_choice_empty_is_valid() {
        let json = serde_json::json!({
            "content_ids": ""
        });
        let parsed: EvictionChoice = serde_json::from_value(json).unwrap();
        assert!(parsed.ids().is_empty());
    }

    #[test]
    fn eviction_choice_ids_trims_whitespace() {
        let choice = EvictionChoice {
            content_ids: " foo , bar , baz ".into(),
        };
        let ids = choice.ids();
        assert_eq!(ids, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn eviction_choice_ids_single() {
        let choice = EvictionChoice {
            content_ids: "docs/auth.md#tokens".into(),
        };
        let ids = choice.ids();
        assert_eq!(ids, vec!["docs/auth.md#tokens"]);
    }

    // -----------------------------------------------------------------------
    // ElicitationSchema compatibility — types produce valid rmcp schemas
    // -----------------------------------------------------------------------

    #[test]
    fn eviction_choice_rmcp_schema() {
        let schema = rmcp::model::ElicitationSchema::from_type::<EvictionChoice>();
        assert!(
            schema.is_ok(),
            "EvictionChoice should produce a valid ElicitationSchema"
        );
    }

    #[test]
    fn compression_confirmation_rmcp_schema() {
        let schema = rmcp::model::ElicitationSchema::from_type::<CompressionConfirmation>();
        assert!(
            schema.is_ok(),
            "CompressionConfirmation should produce a valid ElicitationSchema"
        );
    }

    #[test]
    fn search_refinement_rmcp_schema() {
        let schema = rmcp::model::ElicitationSchema::from_type::<SearchRefinement>();
        assert!(
            schema.is_ok(),
            "SearchRefinement should produce a valid ElicitationSchema"
        );
    }
}
