//! Core domain types for the ministr code intelligence server.
//!
//! These types model the multi-resolution document index: documents contain
//! sections, sections contain claims. Each level has a unique ID and can be
//! independently retrieved and embedded.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Unique identifier for any content node in the index.
///
/// # Examples
///
/// ```
/// use ministr_core::types::ContentId;
///
/// let id = ContentId::from("doc-api".to_string());
/// assert_eq!(id.to_string(), "doc-api");
/// assert_eq!(id.as_ref(), "doc-api");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContentId(pub String);

/// Stable name used for the local corpus when no linked project or daemon
/// corpus id was supplied. Persisted legacy session rows are migrated into
/// this namespace so they keep their historical local-dedup behaviour without
/// suppressing matching ids in linked or Atlas corpora.
pub const PRIMARY_CORPUS_ID: &str = "primary";

/// Durable identity for one delivered representation of corpus content.
///
/// Resolution is intentionally a string rather than [`Resolution`]: delivery
/// representations include transport-level variants such as
/// `section_excerpt`, `section_full`, and `symbol_outline` which are not vector
/// index resolutions. Different resolutions are independent deliveries.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct DeliveryIdentity {
    /// Canonical corpus id (`primary`, linked-project label, daemon id, or
    /// Atlas slug).
    pub corpus_id: String,
    /// Content id within the corpus.
    pub content_id: String,
    /// Delivered representation (`summary`, `section_excerpt`,
    /// `section_full`, `symbol_stub`, `symbol_full`, ...).
    pub resolution: String,
}

impl DeliveryIdentity {
    /// Construct a delivery identity.
    #[must_use]
    pub fn new(
        corpus_id: impl Into<String>,
        content_id: impl Into<String>,
        resolution: impl Into<String>,
    ) -> Self {
        Self {
            corpus_id: corpus_id.into(),
            content_id: content_id.into(),
            resolution: resolution.into(),
        }
    }

    /// Construct an identity in the compatibility namespace used by local
    /// sessions created before corpus-aware identities existed.
    #[must_use]
    pub fn primary(content_id: impl Into<String>, resolution: impl Into<String>) -> Self {
        Self::new(PRIMARY_CORPUS_ID, content_id, resolution)
    }

    /// Unambiguous storage/cache key. JSON serialization avoids delimiter
    /// collisions when ids themselves contain punctuation.
    ///
    /// # Panics
    ///
    /// `DeliveryIdentity` contains only strings, so serialization cannot
    /// fail. A panic would indicate a broken serde implementation.
    #[must_use]
    pub fn storage_key(&self) -> String {
        serde_json::to_string(self).expect("DeliveryIdentity is always serializable")
    }

    /// Decode a structured key, falling back to a legacy bare content id in
    /// the primary corpus at the supplied resolution.
    #[must_use]
    pub fn from_storage_key(key: &str, legacy_resolution: &str) -> Self {
        serde_json::from_str(key)
            .unwrap_or_else(|_| Self::primary(key.to_string(), legacy_resolution.to_string()))
    }
}

impl From<ContentId> for DeliveryIdentity {
    fn from(content_id: ContentId) -> Self {
        Self::primary(content_id.0, "legacy")
    }
}

/// Executable location of a result, including both durable identity and the
/// routing hints needed by a follow-up tool call.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ResultLocator {
    /// Durable corpus/content/resolution identity.
    pub identity: DeliveryIdentity,
    /// Linked-project label, when the corpus is routed through `.ministr.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Cross-corpus/Atlas source id, when different from `project`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_corpus: Option<String>,
    /// Tenant routing context for hosted deployments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

impl ResultLocator {
    /// Local-primary locator.
    #[must_use]
    pub fn primary(content_id: impl Into<String>, resolution: impl Into<String>) -> Self {
        Self {
            identity: DeliveryIdentity::primary(content_id, resolution),
            project: None,
            source_corpus: None,
            tenant: None,
        }
    }

    /// Retarget a locator to a linked or cross-corpus route.
    #[must_use]
    pub fn routed(
        mut self,
        corpus_id: impl Into<String>,
        project: Option<String>,
        source_corpus: Option<String>,
        tenant: Option<String>,
    ) -> Self {
        self.identity.corpus_id = corpus_id.into();
        self.project = project;
        self.source_corpus = source_corpus;
        self.tenant = tenant;
        self
    }
}

/// How the text in a discovery result represents the underlying content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextRepresentation {
    /// Entire content at the advertised resolution.
    Full,
    /// Stored extractive/abstractive summary chosen because it matched well.
    StoredSummary,
    /// Query-centred excerpt from a larger body.
    QueryExcerpt,
    /// Signature/doc stub for a symbol.
    SymbolStub,
}

/// Explicit size and continuation metadata for bounded result text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TextDeliveryMetadata {
    /// True when returned text is not the full underlying body.
    pub truncated: bool,
    /// Original body size in UTF-8 bytes.
    pub original_bytes: usize,
    /// Original body size using ministr's token estimator.
    pub original_tokens: usize,
    /// Returned text size in UTF-8 bytes.
    pub returned_bytes: usize,
    /// Returned text size using ministr's token estimator.
    pub returned_tokens: usize,
    /// Representation selected for this response.
    pub representation: TextRepresentation,
    /// Locator for the full read/definition when this result is abbreviated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<ResultLocator>,
}

/// Coarse source provenance used for ranking and trust decisions.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
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

/// Compact, machine-readable evidence behind a survey result's final score.
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

/// Standard status for a tool/backend operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Ok,
    Partial,
    Error,
}

/// Stable error information carried alongside any successful partial data.
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

/// Index completeness verdict. Absence is conclusive only for `Complete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompletenessState {
    Complete,
    Partial,
    Stale,
    Unavailable,
}

/// Machine-readable index completeness for one corpus.
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

impl Completeness {
    /// Complete index with conclusive negative results.
    #[must_use]
    pub fn complete(indexed_items: usize) -> Self {
        Self {
            completeness: CompletenessState::Complete,
            indexed_items,
            estimated_total_items: Some(indexed_items),
            affected_capabilities: Vec::new(),
            index_generation: None,
            absence_is_conclusive: true,
            retry_guidance: None,
        }
    }

    /// Actively ingesting index; negative results are not conclusive.
    #[must_use]
    pub fn partial(indexed_items: usize, estimated_total_items: Option<usize>) -> Self {
        Self {
            completeness: CompletenessState::Partial,
            indexed_items,
            estimated_total_items,
            affected_capabilities: vec!["search".to_string(), "code_navigation".to_string()],
            index_generation: None,
            absence_is_conclusive: false,
            retry_guidance: Some("Retry after indexing completes.".to_string()),
        }
    }
}

/// Completeness/status of one member in a fan-out operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CorpusOperationStatus {
    pub corpus_id: String,
    pub status: ResponseStatus,
    pub completeness: Completeness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

/// Standard bounded-collection metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ContentId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for ContentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Hierarchical section identifier (e.g. `docs/auth.md#error-handling`).
///
/// # Examples
///
/// ```
/// use ministr_core::types::SectionId;
///
/// let id = SectionId::from("docs/auth.md#error-handling".to_string());
/// assert_eq!(id.to_string(), "docs/auth.md#error-handling");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SectionId(pub String);

impl fmt::Display for SectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for SectionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for SectionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Unique identifier for a code symbol in the symbol index.
///
/// # Examples
///
/// ```
/// use ministr_core::types::SymbolId;
///
/// let id = SymbolId::from("sym-config::MinistrConfig".to_string());
/// assert_eq!(id.to_string(), "sym-config::MinistrConfig");
/// assert_eq!(id.as_ref(), "sym-config::MinistrConfig");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SymbolId(pub String);

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for SymbolId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for SymbolId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// The kind of reference between two code symbols.
///
/// # Examples
///
/// ```
/// use ministr_core::types::RefKind;
///
/// assert_eq!(RefKind::Calls.as_str(), "calls");
/// assert_eq!(RefKind::parse("implements"), Some(RefKind::Implements));
/// assert_eq!(RefKind::parse("bridge"), Some(RefKind::Bridge));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// Symbol A calls symbol B (function/method invocation).
    Calls,
    /// Symbol A implements symbol B (trait impl, interface).
    Implements,
    /// Symbol A imports symbol B (use declaration).
    Imports,
    /// Symbol A uses symbol B (type reference, field access).
    Uses,
    /// Cross-language bridge link between symbols in different languages.
    Bridge,
}

impl RefKind {
    /// Returns the string representation of this reference kind.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Implements => "implements",
            Self::Imports => "imports",
            Self::Uses => "uses",
            Self::Bridge => "bridge",
        }
    }

    /// Parse a reference kind from a string.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "calls" => Some(Self::Calls),
            "implements" => Some(Self::Implements),
            "imports" => Some(Self::Imports),
            "uses" => Some(Self::Uses),
            "bridge" => Some(Self::Bridge),
            _ => None,
        }
    }
}

impl fmt::Display for RefKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unique identifier for an atomic claim within a section.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClaimId(pub String);

impl fmt::Display for ClaimId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ClaimId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for ClaimId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Extract the parent section ID from a claim content ID.
///
/// Claim IDs are formatted as `{section_id}:c{N}` (e.g. `docs/auth.md#tokens:c0`).
/// This strips the `:cN` suffix to recover the section ID.
///
/// Returns `None` if the string does not end with a `:c{digits}` suffix.
///
/// # Examples
///
/// ```
/// use ministr_core::types::parent_section_id;
///
/// assert_eq!(parent_section_id("docs/auth.md#tokens:c0"), Some("docs/auth.md#tokens"));
/// assert_eq!(parent_section_id("docs/api.md#rate-limits:c12"), Some("docs/api.md#rate-limits"));
/// assert_eq!(parent_section_id("docs/auth.md#tokens"), None);
/// assert_eq!(parent_section_id("no-colon"), None);
/// ```
#[must_use]
pub fn parent_section_id(claim_content_id: &str) -> Option<&str> {
    let (prefix, suffix) = claim_content_id.rsplit_once(":c")?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(prefix)
}

/// The resolution level at which content was indexed or delivered.
///
/// # Examples
///
/// ```
/// use ministr_core::types::Resolution;
///
/// assert_eq!(Resolution::Summary.to_string(), "summary");
/// assert_eq!(Resolution::Section.to_string(), "section");
/// assert_eq!(Resolution::Claim.to_string(), "claim");
/// assert_eq!(Resolution::SymbolStub.to_string(), "symbol_stub");
/// assert_eq!(Resolution::SymbolFull.to_string(), "symbol_full");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Resolution {
    /// Compressed summary of a document or section (~50–400 tokens).
    Summary,
    /// Full section text with structural context (~200–2000 tokens).
    Section,
    /// Atomic factual statement (~10–50 tokens).
    Claim,
    /// Code symbol stub: signature + doc comment (~20–100 tokens).
    SymbolStub,
    /// Code symbol full source (~50–500 tokens).
    SymbolFull,
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Summary => f.write_str("summary"),
            Self::Section => f.write_str("section"),
            Self::Claim => f.write_str("claim"),
            Self::SymbolStub => f.write_str("symbol_stub"),
            Self::SymbolFull => f.write_str("symbol_full"),
        }
    }
}

/// A vector ID that encodes both resolution level and content identifier.
///
/// Format: `{resolution}::{content_id}` where resolution is one of
/// `doc-summary`, `sec-summary`, `section`, `claim`, `symbol-stub`, or `symbol-full`.
///
/// # Examples
///
/// ```
/// use ministr_core::types::{VectorId, Resolution};
///
/// let vid = VectorId::doc_summary("doc-api");
/// assert_eq!(vid.as_str(), "doc-summary::doc-api");
/// assert_eq!(vid.resolution(), Resolution::Summary);
/// assert_eq!(vid.content_id(), "doc-api");
///
/// let parsed = VectorId::parse("claim::c42").unwrap();
/// assert_eq!(parsed.resolution(), Resolution::Claim);
///
/// let sym = VectorId::symbol_stub("sym-config::MinistrConfig");
/// assert_eq!(sym.resolution(), Resolution::SymbolStub);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VectorId(String);

impl VectorId {
    /// Create a vector ID for a document-level summary.
    #[must_use]
    pub fn doc_summary(doc_id: &str) -> Self {
        Self(format!("doc-summary::{doc_id}"))
    }

    /// Create a vector ID for a section-level summary.
    #[must_use]
    pub fn sec_summary(section_id: &str) -> Self {
        Self(format!("sec-summary::{section_id}"))
    }

    /// Create a vector ID for a full section embedding.
    #[must_use]
    pub fn section(section_id: &str) -> Self {
        Self(format!("section::{section_id}"))
    }

    /// Create a vector ID for a claim embedding.
    #[must_use]
    pub fn claim(claim_id: &str) -> Self {
        Self(format!("claim::{claim_id}"))
    }

    /// Create a vector ID for a code symbol stub (signature + doc comment).
    #[must_use]
    pub fn symbol_stub(symbol_id: &str) -> Self {
        Self(format!("symbol-stub::{symbol_id}"))
    }

    /// Create a vector ID for a code symbol's full source.
    #[must_use]
    pub fn symbol_full(symbol_id: &str) -> Self {
        Self(format!("symbol-full::{symbol_id}"))
    }

    /// Parse a vector ID string into a `VectorId`.
    ///
    /// Returns `None` if the string does not match the expected format.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let (prefix, _content) = s.split_once("::")?;
        match prefix {
            "doc-summary" | "sec-summary" | "section" | "claim" | "symbol-stub" | "symbol-full" => {
                Some(Self(s.to_string()))
            }
            _ => None,
        }
    }

    /// The resolution level encoded in this vector ID.
    #[must_use]
    pub fn resolution(&self) -> Resolution {
        match self.0.split_once("::").map(|(p, _)| p) {
            Some("doc-summary" | "sec-summary") => Resolution::Summary,
            Some("section") => Resolution::Section,
            Some("claim") => Resolution::Claim,
            Some("symbol-stub") => Resolution::SymbolStub,
            Some("symbol-full") => Resolution::SymbolFull,
            _ => unreachable!("VectorId always has a valid prefix"),
        }
    }

    /// Whether this is a document-level summary (as opposed to section-level).
    #[must_use]
    pub fn is_doc_summary(&self) -> bool {
        self.0.starts_with("doc-summary::")
    }

    /// The content ID portion (after the `::` separator).
    #[must_use]
    pub fn content_id(&self) -> &str {
        self.0.split_once("::").map_or("", |(_, id)| id)
    }

    /// The full vector ID string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VectorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A typed structural element within a section.
///
/// Structural nodes preserve the semantic type of content blocks (code, tables,
/// lists) so downstream processing can handle them differently from plain text.
///
/// # Examples
///
/// ```
/// use ministr_core::types::StructuralNode;
///
/// let code = StructuralNode::CodeBlock {
///     language: "rust".into(),
///     code: "fn main() {}".into(),
/// };
/// assert!(matches!(code, StructuralNode::CodeBlock { .. }));
///
/// let list = StructuralNode::ListBlock {
///     ordered: true,
///     items: vec!["First".into(), "Second".into()],
/// };
/// assert!(matches!(list, StructuralNode::ListBlock { ordered: true, .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructuralNode {
    /// A fenced or indented code block.
    CodeBlock {
        /// Language annotation (e.g. `"rust"`, `"python"`). Empty if unspecified.
        language: String,
        /// The code content.
        code: String,
    },
    /// A table with headers and data rows.
    Table {
        /// Column header texts.
        headers: Vec<String>,
        /// Data rows, each row is a vec of cell texts.
        rows: Vec<Vec<String>>,
    },
    /// An ordered or unordered list.
    ListBlock {
        /// `true` for ordered (numbered) lists.
        ordered: bool,
        /// List item texts.
        items: Vec<String>,
    },
}

/// A parsed document represented as a tree of sections.
///
/// # Examples
///
/// ```
/// use ministr_core::types::{DocumentTree, Section, ContentId, SectionId};
///
/// let tree = DocumentTree {
///     id: ContentId("doc-api".into()),
///     title: "API Reference".into(),
///     source_path: "docs/api.md".into(),
///     sections: vec![Section {
///         id: SectionId("docs/api.md#intro".into()),
///         heading_path: vec!["Introduction".into()],
///         depth: 1,
///         text: "Welcome to the API.".into(),
///         structural_nodes: vec![],
///         children: vec![],
///         claims: vec![],
///         summary: None,
///     }],
///     summary: Some("Full API reference.".into()),
/// };
///
/// assert_eq!(tree.sections.len(), 1);
/// assert_eq!(tree.title, "API Reference");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentTree {
    /// Unique content ID for the whole document.
    pub id: ContentId,
    /// Document title (derived from first heading or filename).
    pub title: String,
    /// Source file path relative to the corpus root.
    pub source_path: String,
    /// Top-level sections in document order.
    pub sections: Vec<Section>,
    /// Pre-generated document-level summary.
    pub summary: Option<String>,
}

impl DocumentTree {
    /// Ensure every section and claim ID in the tree is unique.
    ///
    /// Duplicate section IDs arise when a source file contains multiple
    /// symbols with the same fully-qualified name (e.g. `fn rss_bytes()`
    /// under different `#[cfg]` blocks). This renames collisions by
    /// appending `-2`, `-3`, etc., and updates child claim IDs to match.
    ///
    /// Call this after parsing and enrichment, before storage, embedding,
    /// or relationship detection — so every downstream consumer sees
    /// consistent, unique IDs.
    pub fn deduplicate_ids(&mut self) {
        let mut seen = std::collections::HashSet::new();
        dedup_section_ids_recursive(&mut self.sections, &mut seen);
    }
}

fn dedup_section_ids_recursive(
    sections: &mut [Section],
    seen: &mut std::collections::HashSet<String>,
) {
    for section in sections.iter_mut() {
        let original = section.id.as_ref().to_string();
        if seen.contains(&original) {
            let deduped = (2u64..u64::MAX)
                .map(|n| format!("{original}-{n}"))
                .find(|candidate| !seen.contains(candidate))
                .expect("dedup always finds a candidate");

            section.id = SectionId(deduped.clone());
            for (i, claim) in section.claims.iter_mut().enumerate() {
                claim.id = ClaimId(format!("{deduped}:c{i}"));
                claim.section_id = section.id.clone();
            }
            seen.insert(deduped);
        } else {
            seen.insert(original);
        }
        dedup_section_ids_recursive(&mut section.children, seen);
    }
}

/// A structural section within a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// Unique section identifier.
    pub id: SectionId,
    /// Heading hierarchy path (e.g. `["Chapter 3", "Section 3.2", "Error Handling"]`).
    pub heading_path: Vec<String>,
    /// Heading depth (1 = top-level, 2 = subsection, etc.).
    pub depth: u32,
    /// Full text content of the section.
    pub text: String,
    /// Typed structural elements (code blocks, tables, lists) in document order.
    pub structural_nodes: Vec<StructuralNode>,
    /// Child sections nested under this one.
    pub children: Vec<Section>,
    /// Atomic claims extracted from this section.
    pub claims: Vec<Claim>,
    /// Pre-generated section-level summary.
    pub summary: Option<String>,
}

/// An atomic factual statement extracted from a section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// Unique claim identifier.
    pub id: ClaimId,
    /// The claim text as a standalone statement.
    pub text: String,
    /// ID of the section this claim belongs to.
    pub section_id: SectionId,
}

/// The type of relationship between two claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    /// Claim A mentions a concept that claim B defines or elaborates.
    References,
    /// Claims assert opposing things about the same subject.
    Contradicts,
    /// Claim A requires knowledge from claim B to be understood.
    DependsOn,
    /// Claim A supersedes or modifies the information in claim B.
    Updates,
}

impl fmt::Display for RelationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::References => f.write_str("references"),
            Self::Contradicts => f.write_str("contradicts"),
            Self::DependsOn => f.write_str("depends_on"),
            Self::Updates => f.write_str("updates"),
        }
    }
}

impl RelationType {
    /// Parse a relation type from a string.
    ///
    /// # Examples
    ///
    /// ```
    /// use ministr_core::types::RelationType;
    ///
    /// assert_eq!(RelationType::parse("references"), Some(RelationType::References));
    /// assert_eq!(RelationType::parse("depends_on"), Some(RelationType::DependsOn));
    /// assert_eq!(RelationType::parse("unknown"), None);
    /// ```
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "references" => Some(Self::References),
            "contradicts" => Some(Self::Contradicts),
            "depends_on" => Some(Self::DependsOn),
            "updates" => Some(Self::Updates),
            _ => None,
        }
    }
}

/// A directed relationship between two claims.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimRelationship {
    /// The source claim.
    pub source_claim_id: ClaimId,
    /// The target claim.
    pub target_claim_id: ClaimId,
    /// The type of relationship.
    pub relation_type: RelationType,
    /// Confidence score (0.0–1.0) from the relationship detector.
    pub confidence: f32,
}

/// A metadata-only entry in the corpus table of contents.
///
/// Contains structural information about a section without its text content,
/// suitable for giving agents a quick overview of the indexed corpus.
///
/// # Examples
///
/// ```
/// use ministr_core::types::{TocEntry, ContentId, SectionId};
///
/// let entry = TocEntry {
///     document_id: ContentId("docs/api.md".into()),
///     section_id: SectionId("docs/api.md#auth".into()),
///     heading_path: vec!["API Reference".into(), "Authentication".into()],
///     depth: 2,
///     claims_available: 5,
///     token_count: 320,
/// };
/// assert_eq!(entry.depth, 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TocEntry {
    /// Parent document ID.
    pub document_id: ContentId,
    /// Section identifier.
    pub section_id: SectionId,
    /// Heading hierarchy path.
    pub heading_path: Vec<String>,
    /// Heading depth (1 = top-level).
    pub depth: u32,
    /// Number of claims available for extraction.
    pub claims_available: usize,
    /// Approximate token count of the section text.
    pub token_count: usize,
}

/// The kind of a corpus root source.
///
/// # Examples
///
/// ```
/// use ministr_core::types::RootKind;
///
/// let kind = RootKind::Local;
/// assert_eq!(kind.as_str(), "local");
/// assert_eq!(RootKind::parse("git"), RootKind::Git);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RootKind {
    /// A local filesystem directory.
    Local,
    /// A web URL fetched via `WebFetcher`.
    Web,
    /// A git repository cloned via `GitFetcher`.
    Git,
}

impl RootKind {
    /// Return the string representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Web => "web",
            Self::Git => "git",
        }
    }

    /// Parse from a string, defaulting to `Local` for unknown values.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "web" => Self::Web,
            "git" => Self::Git,
            _ => Self::Local,
        }
    }
}

impl fmt::Display for RootKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A registered corpus root directory with per-root metadata.
///
/// Each root represents one source directory (or URL) in a multi-root corpus.
/// Roots track per-directory file counts and language statistics.
/// Git and web roots include provenance metadata for cache management.
///
/// # Examples
///
/// ```
/// use ministr_core::types::{CorpusRoot, RootKind};
/// use std::collections::HashMap;
///
/// let root = CorpusRoot {
///     id: "abc123".into(),
///     path: "/home/user/project/src".into(),
///     kind: RootKind::Local,
///     display_name: Some("src".into()),
///     file_count: 42,
///     language_stats: HashMap::from([("rust".into(), 30), ("toml".into(), 12)]),
///     repo_url: None,
///     branch: None,
///     commit_sha: None,
///     clone_timestamp: None,
///     sparse_paths: Vec::new(),
/// };
/// assert_eq!(root.file_count, 42);
/// assert_eq!(root.language_stats["rust"], 30);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CorpusRoot {
    /// Stable identifier derived from the root path.
    pub id: String,
    /// Canonical path (or URL) for this root.
    pub path: String,
    /// Source kind: local, web, or git.
    pub kind: RootKind,
    /// Human-readable display name (typically the directory basename).
    pub display_name: Option<String>,
    /// Number of files indexed from this root.
    pub file_count: usize,
    /// Language → file count mapping for this root.
    pub language_stats: std::collections::HashMap<String, usize>,
    /// Remote repository URL (git roots only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    /// Branch that was cloned (git roots only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Commit SHA at clone time (git roots only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    /// Epoch-seconds timestamp of the clone (git roots only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clone_timestamp: Option<String>,
    /// Paths used for sparse checkout (empty = full checkout).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sparse_paths: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_id_display_and_conversion() {
        let id = ContentId::from("doc-001".to_string());
        assert_eq!(id.to_string(), "doc-001");
        assert_eq!(id.as_ref(), "doc-001");
    }

    #[test]
    fn delivery_identity_round_trips_adversarial_ids_without_collisions() {
        let corpora = ["primary", "linked:one", "atlas/react", "組織/資料"];
        let content = ["a:b", "a|b", "{\"x\":1}", "文書#節:c0"];
        let resolutions = ["section_excerpt", "section_full", "symbol_outline"];
        let mut keys = std::collections::HashSet::new();

        for corpus_id in corpora {
            for content_id in content {
                for resolution in resolutions {
                    let identity = DeliveryIdentity::new(corpus_id, content_id, resolution);
                    let key = identity.storage_key();
                    assert!(
                        keys.insert(key.clone()),
                        "identity collision for {identity:?}"
                    );
                    assert_eq!(
                        DeliveryIdentity::from_storage_key(&key, "ignored"),
                        identity
                    );
                }
            }
        }
    }

    #[test]
    fn legacy_bare_delivery_keys_migrate_to_primary_corpus() {
        let migrated = DeliveryIdentity::from_storage_key("docs/a.md#root", "section_full");
        assert_eq!(migrated.corpus_id, PRIMARY_CORPUS_ID);
        assert_eq!(migrated.content_id, "docs/a.md#root");
        assert_eq!(migrated.resolution, "section_full");
    }

    #[test]
    fn same_content_id_is_distinct_by_corpus_and_resolution() {
        let linked = DeliveryIdentity::new("linked", "same", "section_excerpt");
        let atlas = DeliveryIdentity::new("atlas/pkg", "same", "section_excerpt");
        let full = DeliveryIdentity::new("linked", "same", "section_full");
        assert_ne!(linked, atlas);
        assert_ne!(linked, full);
    }

    #[test]
    fn section_id_display_and_conversion() {
        let id = SectionId::from("docs/auth.md#error-handling".to_string());
        assert_eq!(id.to_string(), "docs/auth.md#error-handling");
        assert_eq!(id.as_ref(), "docs/auth.md#error-handling");
    }

    #[test]
    fn claim_id_display_and_conversion() {
        let id = ClaimId::from("claim-42".to_string());
        assert_eq!(id.to_string(), "claim-42");
    }

    #[test]
    fn resolution_display() {
        assert_eq!(Resolution::Summary.to_string(), "summary");
        assert_eq!(Resolution::Section.to_string(), "section");
        assert_eq!(Resolution::Claim.to_string(), "claim");
        assert_eq!(Resolution::SymbolStub.to_string(), "symbol_stub");
        assert_eq!(Resolution::SymbolFull.to_string(), "symbol_full");
    }

    #[test]
    fn root_kind_roundtrip() {
        assert_eq!(RootKind::Local.as_str(), "local");
        assert_eq!(RootKind::Web.as_str(), "web");
        assert_eq!(RootKind::Git.as_str(), "git");
        assert_eq!(RootKind::parse("local"), RootKind::Local);
        assert_eq!(RootKind::parse("web"), RootKind::Web);
        assert_eq!(RootKind::parse("git"), RootKind::Git);
        assert_eq!(RootKind::parse("unknown"), RootKind::Local);
    }

    #[test]
    fn corpus_root_construction() {
        let root = CorpusRoot {
            id: "r1".into(),
            path: "/home/user/project".into(),
            kind: RootKind::Local,
            display_name: Some("project".into()),
            file_count: 5,
            language_stats: std::collections::HashMap::from([("rust".into(), 5)]),
            repo_url: None,
            branch: None,
            commit_sha: None,
            clone_timestamp: None,
            sparse_paths: Vec::new(),
        };
        assert_eq!(root.file_count, 5);
        assert_eq!(root.kind, RootKind::Local);
    }

    #[test]
    fn document_tree_construction() {
        let claim = Claim {
            id: ClaimId("c1".into()),
            text: "Rate limits are 100/min.".into(),
            section_id: SectionId("s1".into()),
        };

        let section = Section {
            id: SectionId("s1".into()),
            heading_path: vec!["API Reference".into(), "Rate Limits".into()],
            depth: 2,
            text: "Rate limits are 100/min per API key.".into(),
            structural_nodes: vec![],
            children: vec![],
            claims: vec![claim],
            summary: Some("Rate limiting details.".into()),
        };

        let tree = DocumentTree {
            id: ContentId("doc-api".into()),
            title: "API Reference".into(),
            source_path: "docs/api.md".into(),
            sections: vec![section],
            summary: Some("Full API reference.".into()),
        };

        assert_eq!(tree.sections.len(), 1);
        assert_eq!(tree.sections[0].claims.len(), 1);
        assert_eq!(tree.sections[0].depth, 2);
    }

    #[test]
    fn relation_type_display_and_parse() {
        assert_eq!(RelationType::References.to_string(), "references");
        assert_eq!(RelationType::Contradicts.to_string(), "contradicts");
        assert_eq!(RelationType::DependsOn.to_string(), "depends_on");
        assert_eq!(RelationType::Updates.to_string(), "updates");

        assert_eq!(
            RelationType::parse("references"),
            Some(RelationType::References)
        );
        assert_eq!(
            RelationType::parse("depends_on"),
            Some(RelationType::DependsOn)
        );
        assert_eq!(RelationType::parse("unknown"), None);
    }

    #[test]
    fn relation_type_serialize_roundtrip() {
        let rt = RelationType::References;
        let json = serde_json::to_string(&rt).unwrap();
        assert_eq!(json, "\"references\"");
        let back: RelationType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rt);
    }

    #[test]
    fn claim_relationship_construction() {
        let rel = ClaimRelationship {
            source_claim_id: ClaimId("c1".into()),
            target_claim_id: ClaimId("c2".into()),
            relation_type: RelationType::References,
            confidence: 0.85,
        };
        assert_eq!(rel.source_claim_id.0, "c1");
        assert_eq!(rel.target_claim_id.0, "c2");
        assert_eq!(rel.relation_type, RelationType::References);
    }

    #[test]
    fn types_serialize_roundtrip() {
        let resolution = Resolution::Claim;
        let json = serde_json::to_string(&resolution).unwrap();
        let back: Resolution = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Resolution::Claim);

        let id = ContentId("test".into());
        let json = serde_json::to_string(&id).unwrap();
        let back: ContentId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    // --- VectorId ---

    #[test]
    fn vector_id_doc_summary() {
        let vid = VectorId::doc_summary("doc-api");
        assert_eq!(vid.as_str(), "doc-summary::doc-api");
        assert_eq!(vid.resolution(), Resolution::Summary);
        assert!(vid.is_doc_summary());
        assert_eq!(vid.content_id(), "doc-api");
    }

    #[test]
    fn vector_id_sec_summary() {
        let vid = VectorId::sec_summary("docs/api.md#auth");
        assert_eq!(vid.as_str(), "sec-summary::docs/api.md#auth");
        assert_eq!(vid.resolution(), Resolution::Summary);
        assert!(!vid.is_doc_summary());
        assert_eq!(vid.content_id(), "docs/api.md#auth");
    }

    #[test]
    fn vector_id_section() {
        let vid = VectorId::section("docs/api.md#auth");
        assert_eq!(vid.as_str(), "section::docs/api.md#auth");
        assert_eq!(vid.resolution(), Resolution::Section);
        assert_eq!(vid.content_id(), "docs/api.md#auth");
    }

    #[test]
    fn vector_id_claim() {
        let vid = VectorId::claim("c42");
        assert_eq!(vid.as_str(), "claim::c42");
        assert_eq!(vid.resolution(), Resolution::Claim);
        assert_eq!(vid.content_id(), "c42");
    }

    #[test]
    fn vector_id_parse_valid() {
        let vid = VectorId::parse("claim::c42").unwrap();
        assert_eq!(vid.resolution(), Resolution::Claim);
        assert_eq!(vid.content_id(), "c42");

        let vid = VectorId::parse("doc-summary::d1").unwrap();
        assert!(vid.is_doc_summary());
    }

    #[test]
    fn vector_id_parse_invalid() {
        assert!(VectorId::parse("unknown::id").is_none());
        assert!(VectorId::parse("no-separator").is_none());
        assert!(VectorId::parse("").is_none());
    }

    #[test]
    fn vector_id_symbol_stub() {
        let vid = VectorId::symbol_stub("sym-config::MinistrConfig");
        assert_eq!(vid.as_str(), "symbol-stub::sym-config::MinistrConfig");
        assert_eq!(vid.resolution(), Resolution::SymbolStub);
        assert_eq!(vid.content_id(), "sym-config::MinistrConfig");
    }

    #[test]
    fn vector_id_symbol_full() {
        let vid = VectorId::symbol_full("sym-config::MinistrConfig");
        assert_eq!(vid.as_str(), "symbol-full::sym-config::MinistrConfig");
        assert_eq!(vid.resolution(), Resolution::SymbolFull);
        assert_eq!(vid.content_id(), "sym-config::MinistrConfig");
    }

    #[test]
    fn vector_id_parse_symbol_variants() {
        let stub = VectorId::parse("symbol-stub::sym-foo").unwrap();
        assert_eq!(stub.resolution(), Resolution::SymbolStub);

        let full = VectorId::parse("symbol-full::sym-bar").unwrap();
        assert_eq!(full.resolution(), Resolution::SymbolFull);
    }

    #[test]
    fn vector_id_display() {
        let vid = VectorId::section("s1");
        assert_eq!(vid.to_string(), "section::s1");
    }

    // --- parent_section_id ---

    #[test]
    fn parent_section_id_strips_claim_suffix() {
        assert_eq!(
            parent_section_id("docs/auth.md#tokens:c0"),
            Some("docs/auth.md#tokens")
        );
        assert_eq!(
            parent_section_id("docs/api.md#rate-limits:c12"),
            Some("docs/api.md#rate-limits")
        );
    }

    #[test]
    fn parent_section_id_returns_none_without_suffix() {
        assert_eq!(parent_section_id("docs/auth.md#tokens"), None);
        assert_eq!(parent_section_id("no-colon"), None);
        assert_eq!(parent_section_id(""), None);
    }

    #[test]
    fn parent_section_id_rejects_non_numeric_suffix() {
        assert_eq!(parent_section_id("section:cabc"), None);
        assert_eq!(parent_section_id("section:c"), None);
    }

    fn make_section(id: &str, claims: &[&str]) -> Section {
        Section {
            id: SectionId(id.into()),
            heading_path: vec![],
            depth: 1,
            text: String::new(),
            structural_nodes: vec![],
            children: vec![],
            claims: claims
                .iter()
                .enumerate()
                .map(|(i, text)| Claim {
                    id: ClaimId(format!("{id}:c{i}")),
                    text: (*text).into(),
                    section_id: SectionId(id.into()),
                })
                .collect(),
            summary: None,
        }
    }

    #[test]
    fn deduplicate_ids_renames_collisions() {
        let mut doc = DocumentTree {
            id: ContentId("doc".into()),
            title: "test".into(),
            source_path: "test.rs".into(),
            sections: vec![
                make_section("test.rs#mod::foo", &["claim A"]),
                make_section("test.rs#mod::foo", &["claim B"]),
                make_section("test.rs#mod::foo", &["claim C"]),
            ],
            summary: None,
        };

        doc.deduplicate_ids();

        // First keeps original, second and third get suffixed.
        assert_eq!(doc.sections[0].id.as_ref(), "test.rs#mod::foo");
        assert_eq!(doc.sections[1].id.as_ref(), "test.rs#mod::foo-2");
        assert_eq!(doc.sections[2].id.as_ref(), "test.rs#mod::foo-3");

        // Claims updated to match their parent section.
        assert_eq!(
            doc.sections[1].claims[0].id.as_ref(),
            "test.rs#mod::foo-2:c0"
        );
        assert_eq!(
            doc.sections[1].claims[0].section_id.as_ref(),
            "test.rs#mod::foo-2"
        );
        assert_eq!(
            doc.sections[2].claims[0].id.as_ref(),
            "test.rs#mod::foo-3:c0"
        );
    }

    #[test]
    fn deduplicate_ids_no_op_when_unique() {
        let mut doc = DocumentTree {
            id: ContentId("doc".into()),
            title: "test".into(),
            source_path: "test.rs".into(),
            sections: vec![
                make_section("test.rs#a", &["x"]),
                make_section("test.rs#b", &["y"]),
            ],
            summary: None,
        };

        doc.deduplicate_ids();

        assert_eq!(doc.sections[0].id.as_ref(), "test.rs#a");
        assert_eq!(doc.sections[1].id.as_ref(), "test.rs#b");
    }
}
