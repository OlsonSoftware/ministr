//! PDF parser implementation using the `pdf-extract` crate.
//!
//! Parses PDF documents into a [`DocumentTree`] by extracting text per page.
//! Each page becomes a section at depth 1 with a heading path of `["Page N"]`.
//! This is a simple page-boundary approach; heading detection from font-size
//! heuristics may be added in a future iteration.

use std::path::Path;

use crate::error::ParseError;
use crate::types::{ContentId, DocumentTree};

use super::DocumentParser;
use super::common::{RawSection, build_section_tree};

/// PDF document parser backed by the `pdf-extract` crate.
///
/// Extracts text from each page of a PDF and creates one section per page.
/// The document title is derived from the first non-empty page text or the
/// filename.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use iris_core::parser::{PdfParser, DocumentParser};
///
/// let parser = PdfParser::new();
/// let pdf_bytes = std::fs::read("docs/manual.pdf").unwrap();
/// let content = String::from_utf8_lossy(&pdf_bytes);
/// let tree = parser.parse(Path::new("docs/manual.pdf"), &content).unwrap();
/// ```
pub struct PdfParser;

impl PdfParser {
    /// Create a new PDF parser.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for PdfParser {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentParser for PdfParser {
    fn parse(&self, path: &Path, content: &str) -> Result<DocumentTree, ParseError> {
        let source_path = path.to_string_lossy().to_string();

        // pdf-extract works with bytes; content may have been read as raw bytes
        // and converted to a lossy UTF-8 string by the ingestion pipeline.
        let bytes = content.as_bytes();

        let page_texts =
            pdf_extract::extract_text_from_mem_by_pages(bytes).map_err(|e| ParseError::Failed {
                path: path.to_path_buf(),
                reason: format!("failed to extract PDF text: {e}"),
            })?;

        let sections: Vec<RawSection> = page_texts
            .into_iter()
            .enumerate()
            .filter_map(|(idx, text)| {
                let text = text.trim().to_string();
                if text.is_empty() {
                    return None;
                }
                let page_num = idx + 1;
                let heading = format!("Page {page_num}");
                Some(RawSection {
                    heading_path: vec![heading],
                    depth: 1,
                    text_parts: vec![text],
                    structural_nodes: Vec::new(),
                })
            })
            .collect();

        // Title: first line of first page, or filename
        let title = sections
            .first()
            .and_then(|s| s.text_parts.first())
            .and_then(|text| {
                text.lines()
                    .next()
                    .map(|line| line.trim().to_string())
                    .filter(|line| !line.is_empty() && line.len() <= 200)
            })
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            });

        let doc_id = ContentId(source_path.clone());
        let built_sections = build_section_tree(&source_path, sections);

        Ok(DocumentTree {
            id: doc_id,
            title,
            source_path,
            sections: built_sections,
            summary: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_parser_invalid_content() {
        let parser = PdfParser::new();
        let result = parser.parse(Path::new("bad.pdf"), "not a valid pdf");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("failed to extract PDF text"));
    }

    #[test]
    fn pdf_parser_new() {
        let _parser = PdfParser::new();
    }
}
