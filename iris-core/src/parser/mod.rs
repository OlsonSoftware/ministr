//! Document parsing and structural extraction.
//!
//! The [`DocumentParser`] trait provides a format-agnostic interface for turning
//! raw document content into a [`DocumentTree`]. Implementations exist for
//! Markdown ([`MarkdownParser`]), HTML ([`HtmlParser`]), PDF ([`PdfParser`]),
//! and source code ([`CodeParser`]).
//!
//! Use [`detect_parser_kind`] to auto-detect the parser from a file extension,
//! or [`create_parser`] to instantiate the appropriate parser for a [`ParserKind`].

mod code;
mod common;
mod html;
pub mod html_to_md;
mod markdown;
mod pdf;
mod section_id;

pub use code::CodeParser;
pub use html::HtmlParser;
pub use html_to_md::{ContentExtractor, HtmlToMarkdown, html_to_markdown};
pub use markdown::MarkdownParser;
pub use pdf::PdfParser;
pub use section_id::{generate_code_section_id, generate_section_id};

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
/// assert_eq!(detect_parser_kind(Path::new("main.rs")), Some(ParserKind::Code));
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
    /// Source code (parsed via tree-sitter AST).
    Code,
}

/// Detect the parser kind from a file's extension.
///
/// Routes all known code file extensions to [`ParserKind::Code`], falling
/// back to tree-sitter AST parsing when a grammar is available and to
/// text-based heuristics otherwise. Returns `None` for truly unsupported
/// extensions (images, binaries, etc.).
///
/// # Examples
///
/// ```
/// use iris_core::parser::{ParserKind, detect_parser_kind};
/// use std::path::Path;
///
/// assert_eq!(detect_parser_kind(Path::new("lib.rs")), Some(ParserKind::Code));
/// assert_eq!(detect_parser_kind(Path::new("app.tsx")), Some(ParserKind::Code));
/// assert_eq!(detect_parser_kind(Path::new("main.go")), Some(ParserKind::Code));
/// assert_eq!(detect_parser_kind(Path::new("hello.py")), Some(ParserKind::Code));
/// assert_eq!(detect_parser_kind(Path::new("data.csv")), None);
/// ```
#[must_use]
pub fn detect_parser_kind(path: &Path) -> Option<ParserKind> {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext {
            "md" | "markdown" | "mkd" | "mdx" => return Some(ParserKind::Markdown),
            "html" | "htm" | "xhtml" => return Some(ParserKind::Html),
            "pdf" => return Some(ParserKind::Pdf),
            _ => {
                if crate::code::ALL_CODE_EXTENSIONS.contains(&ext) {
                    return Some(ParserKind::Code);
                }
            }
        }
    }

    // Filename-based detection for files without recognized extensions
    let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
    if filename == "Dockerfile"
        || filename.starts_with("Dockerfile.")
        || filename == "Makefile"
        || filename == "Justfile"
        || filename == "justfile"
        || filename == "Rakefile"
        || filename == "Gemfile"
    {
        return Some(ParserKind::Code);
    }

    None
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
        ParserKind::Code => Box::new(CodeParser::new()),
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
    fn detect_code_extensions() {
        for ext in &[
            "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "c", "cpp", "h", "cs", "rb",
            "swift", "kt", "scala", "php", "ex", "hs", "lua", "zig", "ml", "dart", "sh", "sql",
            "yml", "yaml", "toml", "json", "tf", "proto",
        ] {
            let path = format!("file.{ext}");
            assert_eq!(
                detect_parser_kind(Path::new(&path)),
                Some(ParserKind::Code),
                "expected Code for .{ext}"
            );
        }
    }

    #[test]
    fn detect_dockerfile_by_filename() {
        assert_eq!(
            detect_parser_kind(Path::new("Dockerfile")),
            Some(ParserKind::Code)
        );
        assert_eq!(
            detect_parser_kind(Path::new("Makefile")),
            Some(ParserKind::Code)
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
