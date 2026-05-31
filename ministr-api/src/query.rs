//! Query API types — request and response types for search and retrieval.
//!
//! These mirror the ministr MCP tool parameters and results in a
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
    /// Resolution level: `"summary"`, `"section"`, `"claim"`, `"symbol_stub"`, or `"symbol_full"`.
    /// Matches `Resolution`'s `snake_case` serialization in `ministr-core/src/types.rs`.
    pub resolution: String,
    /// Relevance score (higher is better, 0.0-1.0).
    pub score: f32,
    /// Content text at this resolution level.
    pub text: String,
    /// Heading path for section-level results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<Vec<String>>,
    /// F6.3-a — corpus that produced this hit. `None` for single-corpus
    /// queries; `Some(corpus_id)` for cross-corpus `corpus_ids` fan-out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_corpus: Option<String>,
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
    pub usage_status: Option<crate::session::SessionUsageResponse>,
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
    /// Session the call belongs to, if any. Included so the daemon can
    /// advance the session's turn counter on every tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
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
// Impact / Dead code
// ---------------------------------------------------------------------------

/// Risk level returned by `ministr_impact`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImpactRisk {
    Low,
    Medium,
    High,
}

/// One transitive caller surfaced by `ministr_impact`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ImpactCaller {
    pub symbol_id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub depth: u32,
}

/// Impact analysis response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ImpactResponse {
    pub target_symbol_id: String,
    pub depth: u32,
    pub symbols: usize,
    pub files: usize,
    pub tests: usize,
    pub risk: ImpactRisk,
    pub callers: Vec<ImpactCaller>,
}

/// A dead-code candidate returned by `ministr_dead`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeadSymbol {
    pub symbol_id: String,
    pub name: String,
    pub kind: String,
    pub visibility: String,
    pub file: String,
    pub line: u32,
    pub lines: u32,
}

/// Dead-code response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeadCodeResponse {
    pub symbols: Vec<DeadSymbol>,
    pub total: usize,
}

/// Parameters for the `/dead` daemon endpoint (POST body).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct DeadCodeRequest {
    pub kind: Option<String>,
    pub module: Option<String>,
    pub min_lines: Option<u32>,
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// SOLID
// ---------------------------------------------------------------------------

/// Which SOLID concern a finding pertains to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SolidPrinciple {
    /// DRY + OCP — near-duplicate symbols that probably want an extracted abstraction.
    DryOcp,
    /// Single-Responsibility — container with low cohesion across its methods.
    Srp,
    /// Interface-Segregation — fat interface used partially by most implementors.
    Isp,
    /// Dependency-Inversion — concrete cross-package dependency with an abstraction available.
    Dip,
    /// Fowler's Shotgun Surgery — same `(name, kind)` symbol fanned out across many files
    /// with disjoint internals (parallel dispatch family).
    ShotgunSurgery,
    /// Architectural cyclic dependency — a strongly-connected component in the package-level
    /// import graph.
    CyclicDependency,
}

/// Minimal symbol summary embedded inside a `SolidFinding`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SolidSymbolRef {
    pub symbol_id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
}

/// One cohesion component inside an SRP finding.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SolidComponent {
    pub size: usize,
    pub members: Vec<SolidSymbolRef>,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub members_omitted: usize,
}

/// One package-to-package edge surfaced inside a `CyclicDependency` finding.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SolidEdge {
    pub from: String,
    pub to: String,
    pub example_from: SolidSymbolRef,
    pub example_to: SolidSymbolRef,
}

// Serde's `skip_serializing_if` requires `&T`, so the by-ref signature is
// forced — silence the pedantic by-value suggestion.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}

/// A single SOLID-violation finding.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SolidFinding {
    /// Near-duplicate cluster suggesting DRY/OCP extraction.
    Redundancy {
        principle: SolidPrinciple,
        members: Vec<SolidSymbolRef>,
        #[serde(default, skip_serializing_if = "is_zero_usize")]
        members_omitted: usize,
        members_total: usize,
        canonical: SolidSymbolRef,
        avg_cosine: f32,
        avg_jaccard: f32,
        cross_module: bool,
    },
    /// Container with multiple cohesion components (SRP candidate split).
    LowCohesion {
        principle: SolidPrinciple,
        container: SolidSymbolRef,
        components: Vec<SolidComponent>,
        method_count: usize,
    },
    /// Fat interface most implementors only partially use (ISP).
    FatInterface {
        principle: SolidPrinciple,
        interface: SolidSymbolRef,
        method_count: usize,
        unused_methods: Vec<String>,
        #[serde(default, skip_serializing_if = "is_zero_usize")]
        unused_methods_omitted: usize,
        under_using_implementors: Vec<SolidSymbolRef>,
        #[serde(default, skip_serializing_if = "is_zero_usize")]
        under_using_implementors_omitted: usize,
    },
    /// Concrete cross-package dependency where an abstraction is available (DIP).
    ConcreteDependency {
        principle: SolidPrinciple,
        consumer: SolidSymbolRef,
        concrete_target: SolidSymbolRef,
        suggested_abstraction: Option<SolidSymbolRef>,
    },
    /// Parallel dispatch family — Fowler's Shotgun Surgery.
    ShotgunSurgery {
        principle: SolidPrinciple,
        name: String,
        kind: String,
        sites: Vec<SolidSymbolRef>,
        #[serde(default, skip_serializing_if = "is_zero_usize")]
        sites_omitted: usize,
        sites_total: usize,
        avg_jaccard: f32,
    },
    /// Strongly-connected component in the package-level import graph.
    CyclicDependency {
        principle: SolidPrinciple,
        packages: Vec<String>,
        edge_count: usize,
        example_edges: Vec<SolidEdge>,
        #[serde(default, skip_serializing_if = "is_zero_usize")]
        example_edges_omitted: usize,
    },
}

/// Parameters for the `/solid` daemon endpoint (POST body).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct SolidRequest {
    /// Filter candidates by kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Filter candidates by module path prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Which principles to evaluate. Empty = all four.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub principles: Vec<SolidPrinciple>,
    /// Override container kinds (defaults: `impl`, `class`, `struct`, `mod`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub container_kinds: Vec<String>,
    /// Override interface kinds (defaults: `trait`, `interface`, `protocol`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interface_kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similarity_threshold: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jaccard_threshold: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub srp_cohesion_threshold: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isp_min_methods: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isp_max_overlap_fraction: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_lines: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pairs: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representative_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shotgun_min_sites: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shotgun_max_jaccard: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shotgun_min_packages: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shotgun_skip_conventional_names: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cyclic_min_edges_per_direction: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cyclic_skip_test_paths: Option<bool>,
}

/// SOLID-detection response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SolidResponse {
    pub findings: Vec<SolidFinding>,
    pub total: usize,
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
    pub usage_status: Option<crate::session::SessionUsageResponse>,
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
    /// Full heading hierarchy path for section entries (not just the leaf
    /// `title`). `#[serde(default)]` keeps the wire format back-compatible
    /// with older daemons that only sent `title`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heading_path: Vec<String>,
    /// Number of claims available for extraction from this section.
    #[serde(default)]
    pub claims_available: usize,
    /// Approximate token count of the section text.
    #[serde(default)]
    pub token_count: usize,
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

/// F3.6-a — one node in the bridge graph wire shape. Backs the
/// `/api/v1/corpora/{id}/bridge/graph` endpoint that the F3.6-b web
/// visualizer consumes.
///
/// `id` is unique within a single graph response (the daemon builds
/// it from `{file}::{symbol}::{line}` so two same-named symbols in
/// different files / on different lines collide-resistant).
/// `lang` is the symbol's language slug (unconstrained string — live
/// corpora have ~20 languages so the marketing hero's narrower union
/// of `rust|typescript|python` widens here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BridgeNode {
    /// Stable id used as the `from`/`to` reference in edges.
    pub id: String,
    /// Display label (the symbol name).
    pub label: String,
    /// Source file path the symbol lives in.
    pub file: String,
    /// Language slug — drives the node colour in F3.6-b.
    pub lang: String,
    /// Line number of the symbol's definition.
    pub line: u32,
    /// F3.6-c-ii-b — symbol id when the bridge endpoint matches an
    /// indexed symbol on `(file, name)` whose line range contains
    /// the endpoint line. Consumers can hand this to
    /// `GET /api/v1/corpora/{id}/definition/{sym}` (F3.6-c-ii-c will
    /// wire the side-panel source viewer). `None` when the symbol
    /// indexer hadn't covered the file or no row matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<String>,
}

/// One edge in the bridge graph wire shape.
///
/// `from` and `to` reference [`BridgeNode::id`] values. `kind` is one
/// of the 13 bridge kinds (`tauri_command`, `tauri_event`, `napi`,
/// `pyo3`, `wasm_bindgen`, `uni_ffi`, `jni`, `cgo`, `ffi`, `grpc`,
/// `http_route`, `flutter_channel`, `electron_ipc`) — unconstrained
/// string so a future detector lands without a schema migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BridgeEdge {
    /// Source node id (the export side).
    pub from: String,
    /// Target node id (the import side).
    pub to: String,
    /// Bridge mechanism kind.
    pub kind: String,
    /// Match confidence in `[0.0, 1.0]`.
    pub confidence: f32,
}

/// Bridge graph wire shape returned by
/// `GET /api/v1/corpora/{id}/bridge/graph`. Nodes are unique across
/// the graph; an edge references exactly two nodes by id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BridgeGraph {
    /// All distinct symbols that participate in at least one edge.
    pub nodes: Vec<BridgeNode>,
    /// One entry per bridge link.
    pub edges: Vec<BridgeEdge>,
}

// ---------------------------------------------------------------------------
// Ask (sub-inference)
// ---------------------------------------------------------------------------

/// Request for `ministr_ask` — synthesize an answer using sub-inference.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AskRequest {
    /// Natural-language question to answer from the corpus.
    pub query: String,
    /// Session the call belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Response from `ministr_ask`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AskResponse {
    /// The synthesized answer.
    pub answer: String,
    /// Section IDs that contributed to the answer (for provenance).
    pub source_ids: Vec<String>,
    /// Whether the answer was served from cache.
    pub cached: bool,
    /// Model used for synthesis.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
}
