//! Document parsing and structural extraction.
//!
//! The [`DocumentParser`] trait provides a format-agnostic interface for turning
//! raw document content into a [`DocumentTree`]. The [`MarkdownParser`] implementation
//! uses comrak to parse `CommonMark` / GFM markdown into a structural section tree.

mod markdown;
mod section_id;

pub use markdown::MarkdownParser;
pub use section_id::generate_section_id;

use std::path::Path;

use crate::error::ParseError;
use crate::types::DocumentTree;

/// Format-agnostic document parser.
///
/// Implementations turn raw text content into a [`DocumentTree`] with
/// hierarchical sections and typed structural nodes. The caller is
/// responsible for reading the file; the parser works on `&str` content
/// so it is testable without file I/O.
pub trait DocumentParser: Send + Sync {
    /// Parse document content into a structured tree.
    ///
    /// # Arguments
    /// * `path` — Source file path (relative to corpus root), used for ID
    ///   generation and error messages.
    /// * `content` — Raw text content of the document.
    ///
    /// # Errors
    /// Returns [`ParseError`] if the document cannot be parsed or produces
    /// no extractable sections.
    fn parse(&self, path: &Path, content: &str) -> Result<DocumentTree, ParseError>;
}
