//! Query API types — request and response types for search and retrieval.
//!
//! These mirror the iris MCP tool parameters and results in a
//! transport-agnostic format suitable for HTTP JSON APIs.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Survey
// ---------------------------------------------------------------------------

/// Semantic search across the corpus at multiple resolution levels.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SurveyRequest {
    /// Natural-language search query.
    pub query: String,
    /// Maximum number of results to return (default: 10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<usize>,
    /// Session ID for dedup and budget tracking (omit for stateless queries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A single survey search result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SurveyResult {
    /// Content identifier from the vector index.
    pub content_id: String,
    /// Resolution level: `"document"`, `"section"`, `"claim"`, or `"symbol"`.
    pub resolution: String,
    /// Relevance score (higher is better, 0.0-1.0).
    pub score: f32,
    /// Content text at this resolution level.
    pub text: String,
    /// Heading path for section-level results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<Vec<String>>,
}

/// Survey search response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SurveyResponse {
    /// Ranked search results.
    pub results: Vec<SurveyResult>,
    /// Number of results deduplicated (already delivered in this session).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deduplicated_count: Option<usize>,
    /// Budget status snapshot (present when `session_id` was provided).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_status: Option<crate::session::SessionBudgetResponse>,
}

// ---------------------------------------------------------------------------
// Symbols
// ---------------------------------------------------------------------------

/// Search for code symbols by name, kind, or module.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SymbolsRequest {
    /// Search query (matched against symbol name).
    pub query: String,
    /// Filter by symbol kind (e.g. `"function"`, `"struct"`, `"trait"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Filter by module path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Filter by visibility (e.g. `"pub"`, `"pub(crate)"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /// Maximum results (default: 20).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// A code symbol definition with source context.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SymbolDefinition {
    /// Symbol identifier.
    pub id: String,
    /// Symbol name.
    pub name: String,
    /// Symbol kind.
    pub kind: String,
    /// Visibility modifier.
    pub visibility: String,
    /// Declaration signature.
    pub signature: String,
    /// Doc comment, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_comment: Option<String>,
    /// Source file path.
    pub file_path: String,
    /// Start line (1-based).
    pub line_start: u32,
    /// End line (1-based, inclusive).
    pub line_end: u32,
    /// Module hierarchy path.
    pub heading_path: Vec<String>,
    /// Source code with surrounding context.
    pub source_context: String,
}

/// Symbols search response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SymbolsResponse {
    /// Matching symbols.
    pub symbols: Vec<SymbolDefinition>,
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

/// A cross-reference between two symbols.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SymbolReference {
    /// The referencing symbol's ID.
    pub from_symbol_id: String,
    /// Name of the referencing symbol.
    pub from_name: String,
    /// File containing the reference.
    pub from_file: String,
    /// Line of the reference.
    pub from_line: u32,
    /// The referenced symbol's ID.
    pub to_symbol_id: String,
    /// Name of the referenced symbol.
    pub to_name: String,
    /// File containing the referenced symbol.
    pub to_file: String,
    /// Line of the referenced symbol.
    pub to_line: u32,
    /// Reference kind (e.g. `"calls"`, `"imports"`, `"implements"`).
    pub ref_kind: String,
}

/// References response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReferencesResponse {
    /// All references to the queried symbol.
    pub references: Vec<SymbolReference>,
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Full section content.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SectionDetail {
    /// Section identifier.
    pub section_id: String,
    /// Heading hierarchy.
    pub heading_path: Vec<String>,
    /// Full section text.
    pub text: String,
    /// Section summary, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Number of claims available for extraction.
    pub claims_available: usize,
    /// Delivery status: `"already_delivered"` when session dedup detects a re-read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Budget status snapshot (present when `session_id` was provided).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_status: Option<crate::session::SessionBudgetResponse>,
}

// ---------------------------------------------------------------------------
// Extract
// ---------------------------------------------------------------------------

/// Extract atomic claims from a section.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExtractRequest {
    /// Section ID to extract claims from.
    pub section_id: String,
    /// Optional query to filter claims by relevance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Session ID for delivery tracking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A single extracted claim.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClaimResult {
    /// Claim identifier.
    pub claim_id: String,
    /// Claim text.
    pub text: String,
    /// Relevance score when filtered by query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance: Option<f32>,
}

/// Extract response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExtractResponse {
    /// Extracted claims.
    pub claims: Vec<ClaimResult>,
}

// ---------------------------------------------------------------------------
// Table of Contents
// ---------------------------------------------------------------------------

/// Request for table of contents.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TocRequest {
    /// Optional document ID to scope the TOC.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    /// Pagination offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    /// Maximum entries to return.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Session ID for delivery tracking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A table-of-contents entry.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TocEntry {
    /// Content ID (document or section).
    pub id: String,
    /// Display title or heading.
    pub title: String,
    /// Content kind: `"document"`, `"section"`, or `"symbol"`.
    pub kind: String,
    /// Nesting depth (0 = top-level document).
    pub depth: usize,
    /// Number of child sections.
    pub children: usize,
    /// Source file path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// TOC response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TocResponse {
    /// TOC entries.
    pub entries: Vec<TocEntry>,
    /// Total number of entries (for pagination).
    pub total: usize,
}

// ---------------------------------------------------------------------------
// Related claims
// ---------------------------------------------------------------------------

/// Request for related claims.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RelatedRequest {
    /// Claim ID to find related claims for.
    pub claim_id: String,
    /// Filter by relationship types.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_types: Vec<String>,
    /// Session ID for delivery tracking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A related claim result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RelatedClaimResult {
    /// The related claim's ID.
    pub claim_id: String,
    /// The related claim's text.
    pub text: String,
    /// Relationship type.
    pub relation_type: String,
    /// Source section containing the claim.
    pub source_section: String,
    /// Confidence score (0.0-1.0).
    pub confidence: f32,
}

/// Related claims response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RelatedResponse {
    /// Related claims.
    pub claims: Vec<RelatedClaimResult>,
}

// ---------------------------------------------------------------------------
// Bridge (cross-language) queries
// ---------------------------------------------------------------------------

/// Request for cross-language bridge links.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BridgeRequest {
    /// Search query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Filter by bridge kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Filter by source language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_language: Option<String>,
    /// Maximum results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Session ID for delivery tracking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// A cross-language bridge link.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BridgeLink {
    /// Bridge kind (e.g. `"ffi"`, `"napi"`, `"pyo3"`).
    pub kind: String,
    /// Source-side symbol or identifier.
    pub source: String,
    /// Source language.
    pub source_language: String,
    /// Target-side symbol or identifier.
    pub target: String,
    /// Target language.
    pub target_language: String,
    /// Confidence score.
    pub confidence: f32,
}

/// Bridge query response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BridgeResponse {
    /// Bridge links.
    pub links: Vec<BridgeLink>,
}
