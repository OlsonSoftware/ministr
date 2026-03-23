//! Core domain types for the iris context cache controller.
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
/// use iris_core::types::ContentId;
///
/// let id = ContentId::from("doc-api".to_string());
/// assert_eq!(id.to_string(), "doc-api");
/// assert_eq!(id.as_ref(), "doc-api");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContentId(pub String);

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
/// use iris_core::types::SectionId;
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
/// use iris_core::types::SymbolId;
///
/// let id = SymbolId::from("sym-config::IrisConfig".to_string());
/// assert_eq!(id.to_string(), "sym-config::IrisConfig");
/// assert_eq!(id.as_ref(), "sym-config::IrisConfig");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
/// use iris_core::types::RefKind;
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
/// use iris_core::types::parent_section_id;
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
/// use iris_core::types::Resolution;
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
/// use iris_core::types::{VectorId, Resolution};
///
/// let vid = VectorId::doc_summary("doc-api");
/// assert_eq!(vid.as_str(), "doc-summary::doc-api");
/// assert_eq!(vid.resolution(), Resolution::Summary);
/// assert_eq!(vid.content_id(), "doc-api");
///
/// let parsed = VectorId::parse("claim::c42").unwrap();
/// assert_eq!(parsed.resolution(), Resolution::Claim);
///
/// let sym = VectorId::symbol_stub("sym-config::IrisConfig");
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
/// use iris_core::types::StructuralNode;
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
/// use iris_core::types::{DocumentTree, Section, ContentId, SectionId};
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
    /// use iris_core::types::RelationType;
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
/// use iris_core::types::{TocEntry, ContentId, SectionId};
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
        let vid = VectorId::symbol_stub("sym-config::IrisConfig");
        assert_eq!(vid.as_str(), "symbol-stub::sym-config::IrisConfig");
        assert_eq!(vid.resolution(), Resolution::SymbolStub);
        assert_eq!(vid.content_id(), "sym-config::IrisConfig");
    }

    #[test]
    fn vector_id_symbol_full() {
        let vid = VectorId::symbol_full("sym-config::IrisConfig");
        assert_eq!(vid.as_str(), "symbol-full::sym-config::IrisConfig");
        assert_eq!(vid.resolution(), Resolution::SymbolFull);
        assert_eq!(vid.content_id(), "sym-config::IrisConfig");
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
}
