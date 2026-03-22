//! HTML parser implementation using the `scraper` crate.
//!
//! Parses HTML documents into a [`DocumentTree`] by extracting heading elements
//! (h1–h6) as section boundaries and collecting text content, code blocks,
//! tables, and lists as structural nodes.

use std::path::Path;

use scraper::{ElementRef, Html, Selector};

use super::DocumentParser;
use super::common::{RawSection, build_section_tree};
use crate::error::ParseError;
use crate::types::{ContentId, DocumentTree, StructuralNode};

/// HTML document parser backed by the `scraper` crate.
///
/// Extracts sections from semantic HTML using heading elements (h1–h6) as
/// section boundaries, mirroring the approach used by [`super::MarkdownParser`].
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use iris_core::parser::{HtmlParser, DocumentParser};
///
/// let parser = HtmlParser::new();
/// let tree = parser.parse(
///     Path::new("docs/example.html"),
///     "<html><body><h1>Hello</h1><p>World.</p></body></html>",
/// ).unwrap();
///
/// assert_eq!(tree.title, "Hello");
/// assert_eq!(tree.sections.len(), 1);
/// ```
pub struct HtmlParser;

impl HtmlParser {
    /// Create a new HTML parser.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for HtmlParser {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentParser for HtmlParser {
    fn parse(&self, path: &Path, content: &str) -> Result<DocumentTree, ParseError> {
        let source_path = path.to_string_lossy().to_string();
        let document = Html::parse_document(content);

        let mut collector = HtmlSectionCollector::new();
        collector.walk(&document);
        collector.finalize();

        let title = collector.title.take().unwrap_or_else(|| {
            // Try <title> element as fallback
            extract_title_element(&document).unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
        });

        let doc_id = ContentId(source_path.clone());
        let sections = build_section_tree(&source_path, collector.into_raw_sections());

        Ok(DocumentTree {
            id: doc_id,
            title,
            source_path,
            sections,
            summary: None,
        })
    }
}

/// Extract the text content of the `<title>` element, if present.
fn extract_title_element(document: &Html) -> Option<String> {
    let selector = Selector::parse("title").ok()?;
    document
        .select(&selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Collects sections from an HTML document by walking body elements.
struct HtmlSectionCollector {
    title: Option<String>,
    sections: Vec<RawSection>,
    heading_stack: Vec<(u32, String)>,
}

impl HtmlSectionCollector {
    fn new() -> Self {
        Self {
            title: None,
            sections: Vec::new(),
            heading_stack: Vec::new(),
        }
    }

    /// Walk the body of the HTML document, processing top-level elements.
    fn walk(&mut self, document: &Html) {
        // Try to find <body>, fall back to root element
        let body_selector = Selector::parse("body").expect("valid selector");
        let root_elements: Vec<ElementRef<'_>> =
            if let Some(body) = document.select(&body_selector).next() {
                body.children().filter_map(ElementRef::wrap).collect()
            } else {
                document
                    .root_element()
                    .children()
                    .filter_map(ElementRef::wrap)
                    .collect()
            };

        for element in root_elements {
            self.process_element(element);
        }
    }

    /// Process a single element, recursing into structural containers.
    fn process_element(&mut self, element: ElementRef<'_>) {
        let tag = element.value().name();

        match tag {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = tag[1..].parse::<u32>().expect("valid heading level");
                let text = element.text().collect::<String>().trim().to_string();

                if self.title.is_none() && !text.is_empty() {
                    self.title = Some(text.clone());
                }

                // Pop heading stack to the parent level
                while self.heading_stack.last().is_some_and(|(d, _)| *d >= level) {
                    self.heading_stack.pop();
                }
                self.heading_stack.push((level, text));

                let heading_path: Vec<String> =
                    self.heading_stack.iter().map(|(_, t)| t.clone()).collect();

                self.sections.push(RawSection {
                    heading_path,
                    depth: level,
                    text_parts: Vec::new(),
                    structural_nodes: Vec::new(),
                });
            }
            // Recurse into structural container elements
            "article" | "section" | "main" | "div" => {
                for child in element.children().filter_map(ElementRef::wrap) {
                    self.process_element(child);
                }
            }
            "pre" => {
                self.ensure_current_section();
                let current = self.sections.last_mut().expect("section ensured");

                // Look for <code> inside <pre>
                let code_selector = Selector::parse("code").expect("valid selector");
                let (language, code) = if let Some(code_el) = element.select(&code_selector).next()
                {
                    let lang = code_el
                        .value()
                        .attr("class")
                        .and_then(|c| {
                            c.split_whitespace()
                                .find(|cls| cls.starts_with("language-"))
                                .map(|cls| cls.trim_start_matches("language-").to_string())
                        })
                        .unwrap_or_default();
                    let text = code_el.text().collect::<String>();
                    (lang, text)
                } else {
                    (String::new(), element.text().collect::<String>())
                };

                current.structural_nodes.push(StructuralNode::CodeBlock {
                    language,
                    code: code.clone(),
                });
                current.text_parts.push(code);
            }
            "table" => {
                self.ensure_current_section();
                let current = self.sections.last_mut().expect("section ensured");

                let (headers, rows) = extract_table(element);
                current.structural_nodes.push(StructuralNode::Table {
                    headers: headers.clone(),
                    rows: rows.clone(),
                });

                let mut table_text = headers.join(" | ");
                for row in &rows {
                    table_text.push('\n');
                    table_text.push_str(&row.join(" | "));
                }
                current.text_parts.push(table_text);
            }
            "ul" => {
                self.ensure_current_section();
                let current = self.sections.last_mut().expect("section ensured");
                let items = extract_list_items(element);
                current.structural_nodes.push(StructuralNode::ListBlock {
                    ordered: false,
                    items: items.clone(),
                });
                current.text_parts.push(items.join("\n"));
            }
            "ol" => {
                self.ensure_current_section();
                let current = self.sections.last_mut().expect("section ensured");
                let items = extract_list_items(element);
                current.structural_nodes.push(StructuralNode::ListBlock {
                    ordered: true,
                    items: items.clone(),
                });
                current.text_parts.push(items.join("\n"));
            }
            // Skip non-content elements
            "script" | "style" | "nav" | "footer" | "header" | "aside" | "head" => {}
            _ => {
                // Paragraph, span, or other text-bearing elements
                let text: String = element.text().collect::<String>().trim().to_string();
                if !text.is_empty() {
                    self.ensure_current_section();
                    let current = self.sections.last_mut().expect("section ensured");
                    current.text_parts.push(text);
                }
            }
        }
    }

    /// Ensure there is a current section to append content to.
    fn ensure_current_section(&mut self) {
        if self.sections.is_empty() {
            self.sections.push(RawSection {
                heading_path: Vec::new(),
                depth: 0,
                text_parts: Vec::new(),
                structural_nodes: Vec::new(),
            });
        }
    }

    /// Finalize: remove empty sections.
    fn finalize(&mut self) {
        self.sections
            .retain(|s| !s.text_parts.is_empty() || !s.structural_nodes.is_empty());
    }

    /// Consume the collector and return accumulated raw sections.
    fn into_raw_sections(self) -> Vec<RawSection> {
        self.sections
    }
}

/// Extract table headers and data rows from a `<table>` element.
fn extract_table(table: ElementRef<'_>) -> (Vec<String>, Vec<Vec<String>>) {
    let mut headers = Vec::new();
    let mut rows = Vec::new();

    let row_sel = Selector::parse("tr").expect("valid selector");
    let header_sel = Selector::parse("th").expect("valid selector");
    let cell_sel = Selector::parse("td").expect("valid selector");

    for row in table.select(&row_sel) {
        let ths: Vec<String> = row
            .select(&header_sel)
            .map(|th| th.text().collect::<String>().trim().to_string())
            .collect();

        if !ths.is_empty() {
            headers = ths;
            continue;
        }

        let tds: Vec<String> = row
            .select(&cell_sel)
            .map(|td| td.text().collect::<String>().trim().to_string())
            .collect();

        if !tds.is_empty() {
            rows.push(tds);
        }
    }

    (headers, rows)
}

/// Extract list item texts from a `<ul>` or `<ol>` element.
fn extract_list_items(list: ElementRef<'_>) -> Vec<String> {
    let li_selector = Selector::parse("li").expect("valid selector");
    list.select(&li_selector)
        .map(|li| li.text().collect::<String>().trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_html(content: &str) -> DocumentTree {
        let parser = HtmlParser::new();
        parser.parse(Path::new("test.html"), content).unwrap()
    }

    #[test]
    fn single_heading_with_paragraph() {
        let tree = parse_html("<html><body><h1>Hello</h1><p>World.</p></body></html>");
        assert_eq!(tree.title, "Hello");
        assert_eq!(tree.sections.len(), 1);
        assert_eq!(tree.sections[0].heading_path, vec!["Hello"]);
        assert_eq!(tree.sections[0].depth, 1);
        assert_eq!(tree.sections[0].text, "World.");
    }

    #[test]
    fn nested_headings_build_tree() {
        let html = "\
            <html><body>\
            <h1>Top</h1><p>Intro.</p>\
            <h2>Sub A</h2><p>Content A.</p>\
            <h2>Sub B</h2><p>Content B.</p>\
            </body></html>";
        let tree = parse_html(html);
        assert_eq!(tree.title, "Top");
        assert_eq!(tree.sections.len(), 1);

        let top = &tree.sections[0];
        assert_eq!(top.heading_path, vec!["Top"]);
        assert_eq!(top.text, "Intro.");
        assert_eq!(top.children.len(), 2);

        assert_eq!(top.children[0].heading_path, vec!["Top", "Sub A"]);
        assert_eq!(top.children[0].text, "Content A.");
        assert_eq!(top.children[1].heading_path, vec!["Top", "Sub B"]);
        assert_eq!(top.children[1].text, "Content B.");
    }

    #[test]
    fn code_block_extraction() {
        let html = "\
            <html><body>\
            <h1>Code</h1>\
            <pre><code class=\"language-rust\">fn main() {}</code></pre>\
            </body></html>";
        let tree = parse_html(html);
        let section = &tree.sections[0];
        assert_eq!(section.structural_nodes.len(), 1);
        match &section.structural_nodes[0] {
            StructuralNode::CodeBlock { language, code } => {
                assert_eq!(language, "rust");
                assert!(code.contains("fn main()"));
            }
            other => panic!("expected CodeBlock, got {other:?}"),
        }
    }

    #[test]
    fn table_extraction() {
        let html = "\
            <html><body>\
            <h1>Data</h1>\
            <table>\
            <tr><th>Name</th><th>Value</th></tr>\
            <tr><td>a</td><td>1</td></tr>\
            <tr><td>b</td><td>2</td></tr>\
            </table>\
            </body></html>";
        let tree = parse_html(html);
        let section = &tree.sections[0];
        assert_eq!(section.structural_nodes.len(), 1);
        match &section.structural_nodes[0] {
            StructuralNode::Table { headers, rows } => {
                assert_eq!(headers, &["Name", "Value"]);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0], &["a", "1"]);
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    #[test]
    fn list_extraction() {
        let html = "\
            <html><body>\
            <h1>List</h1>\
            <ul><li>item one</li><li>item two</li></ul>\
            </body></html>";
        let tree = parse_html(html);
        let section = &tree.sections[0];
        assert_eq!(section.structural_nodes.len(), 1);
        match &section.structural_nodes[0] {
            StructuralNode::ListBlock { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items, &["item one", "item two"]);
            }
            other => panic!("expected ListBlock, got {other:?}"),
        }
    }

    #[test]
    fn ordered_list() {
        let html = "\
            <html><body>\
            <h1>Steps</h1>\
            <ol><li>first</li><li>second</li></ol>\
            </body></html>";
        let tree = parse_html(html);
        match &tree.sections[0].structural_nodes[0] {
            StructuralNode::ListBlock { ordered, .. } => assert!(ordered),
            other => panic!("expected ListBlock, got {other:?}"),
        }
    }

    #[test]
    fn document_without_headings() {
        let tree = parse_html("<html><body><p>Just text.</p></body></html>");
        assert_eq!(tree.title, "test");
        assert_eq!(tree.sections.len(), 1);
        assert_eq!(tree.sections[0].depth, 0);
        assert!(tree.sections[0].text.contains("Just text."));
    }

    #[test]
    fn title_from_title_element() {
        let tree = parse_html(
            "<html><head><title>Page Title</title></head><body><p>Content.</p></body></html>",
        );
        assert_eq!(tree.title, "Page Title");
    }

    #[test]
    fn empty_document() {
        let tree = parse_html("<html><body></body></html>");
        assert!(tree.sections.is_empty());
    }

    #[test]
    fn script_and_style_skipped() {
        let html = "\
            <html><body>\
            <h1>Real</h1>\
            <script>alert('xss')</script>\
            <style>body { color: red }</style>\
            <p>Content.</p>\
            </body></html>";
        let tree = parse_html(html);
        assert_eq!(tree.sections.len(), 1);
        assert!(!tree.sections[0].text.contains("alert"));
        assert!(!tree.sections[0].text.contains("color"));
        assert!(tree.sections[0].text.contains("Content."));
    }

    #[test]
    fn article_and_section_tags_are_traversed() {
        let html = "\
            <html><body>\
            <article>\
            <h1>Article</h1>\
            <p>Intro.</p>\
            <section><h2>Part</h2><p>Detail.</p></section>\
            </article>\
            </body></html>";
        let tree = parse_html(html);
        assert_eq!(tree.title, "Article");
        assert_eq!(tree.sections.len(), 1);
        assert_eq!(tree.sections[0].text, "Intro.");
        assert_eq!(tree.sections[0].children.len(), 1);
        assert_eq!(tree.sections[0].children[0].text, "Detail.");
    }

    #[test]
    fn section_ids_are_stable() {
        let html =
            "<html><body><h1>Auth</h1><p>Intro.</p><h2>Errors</h2><p>Content.</p></body></html>";
        let tree1 = parse_html(html);
        let tree2 = parse_html(html);
        assert_eq!(tree1.sections[0].id, tree2.sections[0].id);
        assert_eq!(
            tree1.sections[0].children[0].id,
            tree2.sections[0].children[0].id
        );
    }

    #[test]
    fn source_path_preserved() {
        let parser = HtmlParser::new();
        let tree = parser
            .parse(
                Path::new("docs/api.html"),
                "<html><body><h1>API</h1><p>Content.</p></body></html>",
            )
            .unwrap();
        assert_eq!(tree.source_path, "docs/api.html");
        assert_eq!(tree.id.0, "docs/api.html");
    }
}
