//! Shared wire metadata for corpus identity, bounded output, and honest status.

use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct DeliveryIdentity {
    pub corpus_id: String,
    pub content_id: String,
    pub resolution: String,
}

impl Default for DeliveryIdentity {
    fn default() -> Self {
        Self {
            corpus_id: "primary".to_string(),
            content_id: String::new(),
            resolution: "unknown".to_string(),
        }
    }
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct ResultLocator {
    pub identity: DeliveryIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_corpus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextRepresentation {
    #[default]
    Full,
    Excerpt,
    StoredSummary,
    Outline,
    SymbolStub,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TextDeliveryMetadata {
    pub truncated: bool,
    pub original_bytes: usize,
    pub original_tokens: usize,
    pub returned_bytes: usize,
    pub returned_tokens: usize,
    pub representation: TextRepresentation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<ResultLocator>,
}

impl Default for TextDeliveryMetadata {
    fn default() -> Self {
        Self {
            truncated: false,
            original_bytes: 0,
            original_tokens: 0,
            returned_bytes: 0,
            returned_tokens: 0,
            representation: TextRepresentation::Full,
            continuation: None,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum ContentProvenance {
    Production,
    Test,
    Generated,
    Fixture,
    Benchmark,
    Vendor,
    Documentation,
    Migration,
    Example,
    #[default]
    Unknown,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScoreExplanation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dense_rank: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dense_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sparse_rank: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sparse_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrf_score: Option<f32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exact_match: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prefix_match: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub identifier_match: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_boost: Option<f32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reranked: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub matryoshka: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diversity_selected: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quota_selected: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub graph_expanded: bool,
    pub final_score: f32,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    #[default]
    Ok,
    Partial,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ResponseError {
    pub error_code: String,
    pub retryable: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corpus_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum CompletenessState {
    #[default]
    Complete,
    Partial,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Completeness {
    pub completeness: CompletenessState,
    pub indexed_items: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_total_items: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_generation: Option<String>,
    pub absence_is_conclusive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_guidance: Option<String>,
}

impl Default for Completeness {
    fn default() -> Self {
        Self {
            completeness: CompletenessState::Complete,
            indexed_items: 0,
            estimated_total_items: None,
            affected_capabilities: Vec::new(),
            index_generation: None,
            absence_is_conclusive: true,
            retry_guidance: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CorpusOperationStatus {
    pub corpus_id: String,
    pub status: ResponseStatus,
    pub completeness: Completeness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct Pagination {
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub total: usize,
    pub has_more: bool,
    pub omitted_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct QueryMetadata {
    #[serde(default)]
    pub status: ResponseStatus,
    #[serde(default)]
    pub completeness: Completeness,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corpora: Vec<CorpusOperationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}
