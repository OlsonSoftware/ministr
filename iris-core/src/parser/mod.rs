//! Document parsing and structural extraction.
//!
//! The [`DocumentParser`] trait provides a format-agnostic interface for turning
//! raw document content into a [`DocumentTree`]. Implementations exist for
//! Markdown ([`MarkdownParser`]), HTML ([`HtmlParser`]), and PDF ([`PdfParser`]).
//!
//! Use [`detect_parser_kind`] to auto-detect the parser from a file extension,
//! or [`create_parser`] to instantiate the appropriate parser for a [`ParserKind`].

mod common;
mod html;
pub mod html_to_md;
mod markdown;
mod pdf;
mod section_id;

pub use html::HtmlParser;
pub use html_to_md::{ContentExtractor, HtmlToMarkdown, html_to_markdown};
pub use markdown::MarkdownParser;
pub use pdf::PdfParser;
pub use section_id::generate_section_id;

use std::path::Path;

use serde::{Deserialize, Serialize};

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

/// The kind of document parser to use for a file.
///
/// # Examples
///
/// ```
/// use iris_core::parser::{ParserKind, detect_parser_kind};
/// use std::path::Path;
///
/// assert_eq!(detect_parser_kind(Path::new("doc.md")), Some(ParserKind::Markdown));
/// assert_eq!(detect_parser_kind(Path::new("page.html")), Some(ParserKind::Html));
/// assert_eq!(detect_parser_kind(Path::new("manual.pdf")), Some(ParserKind::Pdf));
/// assert_eq!(detect_parser_kind(Path::new("data.csv")), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserKind {
    /// Markdown (`CommonMark` / GFM).
    Markdown,
    /// HTML.
    Html,
    /// PDF.
    Pdf,
}

/// Detect the parser kind from a file's extension.
///
/// Returns `None` for unsupported extensions.
#[must_use]
pub fn detect_parser_kind(path: &Path) -> Option<ParserKind> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("md" | "markdown" | "mkd" | "mdx") => Some(ParserKind::Markdown),
        Some("html" | "htm" | "xhtml") => Some(ParserKind::Html),
        Some("pdf") => Some(ParserKind::Pdf),
        _ => None,
    }
}

/// Create a boxed parser for the given [`ParserKind`].
///
/// # Examples
///
/// ```
/// use iris_core::parser::{ParserKind, create_parser, DocumentParser};
/// use std::path::Path;
///
/// let parser = create_parser(ParserKind::Markdown);
/// let tree = parser.parse(Path::new("test.md"), "# Hello\n\nWorld.\n").unwrap();
/// assert_eq!(tree.title, "Hello");
/// ```
#[must_use]
pub fn create_parser(kind: ParserKind) -> Box<dyn DocumentParser> {
    match kind {
        ParserKind::Markdown => Box::new(MarkdownParser::new()),
        ParserKind::Html => Box::new(HtmlParser::new()),
        ParserKind::Pdf => Box::new(PdfParser::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_markdown_extensions() {
        assert_eq!(
            detect_parser_kind(Path::new("readme.md")),
            Some(ParserKind::Markdown)
        );
        assert_eq!(
            detect_parser_kind(Path::new("doc.markdown")),
            Some(ParserKind::Markdown)
        );
        assert_eq!(
            detect_parser_kind(Path::new("file.mkd")),
            Some(ParserKind::Markdown)
        );
        assert_eq!(
            detect_parser_kind(Path::new("page.mdx")),
            Some(ParserKind::Markdown)
        );
    }

    #[test]
    fn detect_html_extensions() {
        assert_eq!(
            detect_parser_kind(Path::new("page.html")),
            Some(ParserKind::Html)
        );
        assert_eq!(
            detect_parser_kind(Path::new("page.htm")),
            Some(ParserKind::Html)
        );
        assert_eq!(
            detect_parser_kind(Path::new("page.xhtml")),
            Some(ParserKind::Html)
        );
    }

    #[test]
    fn detect_pdf_extension() {
        assert_eq!(
            detect_parser_kind(Path::new("manual.pdf")),
            Some(ParserKind::Pdf)
        );
    }

    #[test]
    fn detect_unsupported() {
        assert_eq!(detect_parser_kind(Path::new("data.csv")), None);
        assert_eq!(detect_parser_kind(Path::new("image.png")), None);
        assert_eq!(detect_parser_kind(Path::new("no_ext")), None);
    }

    #[test]
    fn create_parser_markdown() {
        let parser = create_parser(ParserKind::Markdown);
        let tree = parser
            .parse(Path::new("test.md"), "# Hi\n\nBody.\n")
            .unwrap();
        assert_eq!(tree.title, "Hi");
    }

    #[test]
    fn create_parser_html() {
        let parser = create_parser(ParserKind::Html);
        let tree = parser
            .parse(
                Path::new("test.html"),
                "<html><body><h1>Hi</h1><p>Body.</p></body></html>",
            )
            .unwrap();
        assert_eq!(tree.title, "Hi");
    }

    #[test]
    fn parser_kind_serialize_roundtrip() {
        let kind = ParserKind::Html;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"html\"");
        let back: ParserKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }
}
