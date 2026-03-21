//! Core domain types for the iris context cache controller.
//!
//! These types model the multi-resolution document index: documents contain
//! sections, sections contain claims. Each level has a unique ID and can be
//! independently retrieved and embedded.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Unique identifier for any content node in the index.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// The resolution level at which content was indexed or delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Resolution {
    /// Compressed summary of a document or section (~50–400 tokens).
    Summary,
    /// Full section text with structural context (~200–2000 tokens).
    Section,
    /// Atomic factual statement (~10–50 tokens).
    Claim,
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Summary => f.write_str("summary"),
            Self::Section => f.write_str("section"),
            Self::Claim => f.write_str("claim"),
        }
    }
}

/// A parsed document represented as a tree of sections.
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
}
