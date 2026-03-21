//! Markdown parser implementation using comrak.
//!
//! Parses `CommonMark` / GFM markdown into a [`DocumentTree`] by walking
//! the comrak AST and splitting on heading nodes to build a hierarchical
//! section tree with typed structural nodes.

use std::path::Path;

use comrak::nodes::NodeValue;
use comrak::{Arena, Options, parse_document};

use super::DocumentParser;
use super::section_id::generate_section_id;
use crate::error::ParseError;
use crate::types::{ContentId, DocumentTree, Section, SectionId, StructuralNode};

/// Markdown document parser backed by comrak.
///
/// Supports `CommonMark` and GitHub Flavored Markdown (tables, task lists,
/// strikethrough, autolinks). Frontmatter (YAML) is skipped if present.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use iris_core::parser::{MarkdownParser, DocumentParser};
///
/// let parser = MarkdownParser::new();
/// let tree = parser.parse(
///     Path::new("docs/example.md"),
///     "# Hello\n\nWorld.\n",
/// ).unwrap();
///
/// assert_eq!(tree.title, "Hello");
/// assert_eq!(tree.sections.len(), 1);
/// ```
pub struct MarkdownParser {
    options: Options<'static>,
}

impl MarkdownParser {
    /// Create a new markdown parser with GFM extensions enabled.
    #[must_use]
    pub fn new() -> Self {
        let mut options = Options::default();
        options.extension.table = true;
        options.extension.tasklist = true;
        options.extension.strikethrough = true;
        options.extension.autolink = true;
        options.extension.front_matter_delimiter = Some("---".into());
        Self { options }
    }
}

impl Default for MarkdownParser {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentParser for MarkdownParser {
    fn parse(&self, path: &Path, content: &str) -> Result<DocumentTree, ParseError> {
        let source_path = path.to_string_lossy().to_string();
        let arena = Arena::new();
        let root = parse_document(&arena, content, &self.options);

        let mut collector = SectionCollector::new(&source_path);
        collector.walk(root);
        collector.finalize();

        let title = collector.title.take().unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });

        let doc_id = ContentId(source_path.clone());
        let sections = collector.into_section_tree();

        Ok(DocumentTree {
            id: doc_id,
            title,
            source_path,
            sections,
            summary: None,
        })
    }
}

/// Intermediate representation of a section being built.
struct RawSection {
    heading_path: Vec<String>,
    depth: u32,
    text_parts: Vec<String>,
    structural_nodes: Vec<StructuralNode>,
}

/// Collects sections from a comrak AST walk.
struct SectionCollector<'a> {
    source_path: &'a str,
    title: Option<String>,
    sections: Vec<RawSection>,
    /// Current heading stack for building heading paths.
    heading_stack: Vec<(u32, String)>,
}

impl<'a> SectionCollector<'a> {
    fn new(source_path: &'a str) -> Self {
        Self {
            source_path,
            title: None,
            sections: Vec::new(),
            heading_stack: Vec::new(),
        }
    }

    /// Walk the top-level children of the document root.
    fn walk<'b>(
        &mut self,
        root: &'b comrak::arena_tree::Node<'b, std::cell::RefCell<comrak::nodes::Ast>>,
    ) {
        for node in root.children() {
            let data = node.data.borrow();
            match &data.value {
                NodeValue::Heading(heading) => {
                    let level = u32::from(heading.level);
                    let text = collect_inline_text(node);

                    // First heading becomes the document title
                    if self.title.is_none() {
                        self.title = Some(text.clone());
                    }

                    // Pop heading stack to the parent level
                    while self.heading_stack.last().is_some_and(|(d, _)| *d >= level) {
                        self.heading_stack.pop();
                    }
                    self.heading_stack.push((level, text.clone()));

                    let heading_path: Vec<String> =
                        self.heading_stack.iter().map(|(_, t)| t.clone()).collect();

                    self.sections.push(RawSection {
                        heading_path,
                        depth: level,
                        text_parts: Vec::new(),
                        structural_nodes: Vec::new(),
                    });
                }
                NodeValue::FrontMatter(_) => {
                    // Skip YAML frontmatter
                }
                _ => {
                    // Content node — append to current section or create implicit root section
                    self.ensure_current_section();
                    let current = self.sections.last_mut().expect("section ensured");

                    // Check for structural nodes
                    match &data.value {
                        NodeValue::CodeBlock(cb) => {
                            let language = cb.info.trim().to_string();
                            let code = cb.literal.clone();
                            current.structural_nodes.push(StructuralNode::CodeBlock {
                                language,
                                code: code.clone(),
                            });
                            current.text_parts.push(code);
                        }
                        NodeValue::Table(_) => {
                            let (headers, rows) = collect_table(node);
                            current.structural_nodes.push(StructuralNode::Table {
                                headers: headers.clone(),
                                rows: rows.clone(),
                            });
                            // Flatten table to text for full-text representation
                            let mut table_text = headers.join(" | ");
                            for row in &rows {
                                table_text.push('\n');
                                table_text.push_str(&row.join(" | "));
                            }
                            current.text_parts.push(table_text);
                        }
                        NodeValue::List(list) => {
                            let ordered = list.list_type == comrak::nodes::ListType::Ordered;
                            let items = collect_list_items(node);
                            current.structural_nodes.push(StructuralNode::ListBlock {
                                ordered,
                                items: items.clone(),
                            });
                            current.text_parts.push(items.join("\n"));
                        }
                        _ => {
                            let text = collect_inline_text(node);
                            if !text.is_empty() {
                                current.text_parts.push(text);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Ensure there is a current section to append content to.
    /// Creates an implicit root section for documents without headings.
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

    /// Finalize: trim empty sections.
    fn finalize(&mut self) {
        self.sections
            .retain(|s| !s.text_parts.is_empty() || !s.structural_nodes.is_empty());
    }

    /// Convert flat raw sections into a nested section tree based on heading depth.
    fn into_section_tree(self) -> Vec<Section> {
        let source_path = self.source_path;
        let raw_sections: Vec<Section> = self
            .sections
            .into_iter()
            .map(|raw| {
                let heading_path_refs: Vec<&str> =
                    raw.heading_path.iter().map(String::as_str).collect();
                let id = SectionId(generate_section_id(source_path, &heading_path_refs));

                Section {
                    id,
                    heading_path: raw.heading_path,
                    depth: raw.depth,
                    text: raw.text_parts.join("\n\n"),
                    structural_nodes: raw.structural_nodes,
                    children: Vec::new(),
                    claims: Vec::new(),
                    summary: None,
                }
            })
            .collect();

        nest_sections(raw_sections)
    }
}

/// Build a nested section tree from a flat depth-ordered list.
///
/// Sections with greater depth are nested as children of the preceding
/// section with lesser depth.
fn nest_sections(flat: Vec<Section>) -> Vec<Section> {
    let mut result: Vec<Section> = Vec::new();
    let mut stack: Vec<Section> = Vec::new();

    for section in flat {
        // Pop sections from the stack that are at the same or deeper level
        while stack.last().is_some_and(|top| top.depth >= section.depth) {
            let popped = stack.pop().expect("stack non-empty");
            if let Some(parent) = stack.last_mut() {
                parent.children.push(popped);
            } else {
                result.push(popped);
            }
        }
        stack.push(section);
    }

    // Drain remaining stack
    while let Some(popped) = stack.pop() {
        if let Some(parent) = stack.last_mut() {
            parent.children.push(popped);
        } else {
            result.push(popped);
        }
    }

    result
}

/// Recursively collect all inline text content from a node and its descendants.
fn collect_inline_text<'a>(
    node: &'a comrak::arena_tree::Node<'a, std::cell::RefCell<comrak::nodes::Ast>>,
) -> String {
    let mut text = String::new();
    collect_inline_text_recursive(node, &mut text);
    text
}

fn collect_inline_text_recursive<'a>(
    node: &'a comrak::arena_tree::Node<'a, std::cell::RefCell<comrak::nodes::Ast>>,
    out: &mut String,
) {
    let data = node.data.borrow();
    match &data.value {
        NodeValue::Text(t) => out.push_str(t),
        NodeValue::Code(c) => {
            out.push('`');
            out.push_str(&c.literal);
            out.push('`');
        }
        NodeValue::SoftBreak | NodeValue::LineBreak => out.push(' '),
        _ => {}
    }
    drop(data);
    for child in node.children() {
        collect_inline_text_recursive(child, out);
    }
}

/// Collect table headers and data rows.
fn collect_table<'a>(
    table_node: &'a comrak::arena_tree::Node<'a, std::cell::RefCell<comrak::nodes::Ast>>,
) -> (Vec<String>, Vec<Vec<String>>) {
    let mut headers = Vec::new();
    let mut rows = Vec::new();

    for row_node in table_node.children() {
        let row_data = row_node.data.borrow();
        let is_header = matches!(row_data.value, NodeValue::TableRow(true));
        drop(row_data);

        let cells: Vec<String> = row_node
            .children()
            .map(|cell| collect_inline_text(cell).trim().to_string())
            .collect();

        if is_header {
            headers = cells;
        } else {
            rows.push(cells);
        }
    }

    (headers, rows)
}

/// Collect list item texts from a list node.
fn collect_list_items<'a>(
    list_node: &'a comrak::arena_tree::Node<'a, std::cell::RefCell<comrak::nodes::Ast>>,
) -> Vec<String> {
    list_node
        .children()
        .map(|item| collect_inline_text(item).trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_md(content: &str) -> DocumentTree {
        let parser = MarkdownParser::new();
        parser.parse(Path::new("test.md"), content).unwrap()
    }

    // --- Heading hierarchy tests ---

    #[test]
    fn single_heading_with_paragraph() {
        let tree = parse_md("# Hello\n\nWorld.\n");
        assert_eq!(tree.title, "Hello");
        assert_eq!(tree.sections.len(), 1);
        assert_eq!(tree.sections[0].heading_path, vec!["Hello"]);
        assert_eq!(tree.sections[0].depth, 1);
        assert_eq!(tree.sections[0].text, "World.");
    }

    #[test]
    fn nested_headings_build_tree() {
        let tree =
            parse_md("# Top\n\nIntro.\n\n## Sub A\n\nContent A.\n\n## Sub B\n\nContent B.\n");
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
    fn deeply_nested_headings() {
        let tree = parse_md("# H1\n\nA.\n\n## H2\n\nB.\n\n### H3\n\nC.\n\n## H2b\n\nD.\n");
        let h1 = &tree.sections[0];
        assert_eq!(h1.children.len(), 2);
        assert_eq!(h1.children[0].children.len(), 1);
        assert_eq!(
            h1.children[0].children[0].heading_path,
            vec!["H1", "H2", "H3"]
        );
    }

    #[test]
    fn multiple_top_level_headings() {
        let tree = parse_md("# First\n\nA.\n\n# Second\n\nB.\n");
        assert_eq!(tree.sections.len(), 2);
        assert_eq!(tree.sections[0].heading_path, vec!["First"]);
        assert_eq!(tree.sections[1].heading_path, vec!["Second"]);
    }

    // --- Section ID tests ---

    #[test]
    fn section_ids_are_stable() {
        let tree = parse_md("# Getting Started\n\n## Error Handling\n\nContent.\n");
        let parser = MarkdownParser::new();
        let tree2 = parser
            .parse(
                Path::new("test.md"),
                "# Getting Started\n\n## Error Handling\n\nContent.\n",
            )
            .unwrap();

        assert_eq!(tree.sections[0].id, tree2.sections[0].id);
    }

    #[test]
    fn section_id_format() {
        let tree = parse_md("# Auth\n\nIntro.\n\n## Error Handling\n\nContent.\n");
        let child = &tree.sections[0].children[0];
        assert_eq!(child.id.0, "test.md#auth/error-handling");
    }

    // --- Code block tests ---

    #[test]
    fn fenced_code_block() {
        let tree = parse_md("# Code\n\n```rust\nfn main() {}\n```\n");
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
    fn code_block_without_language() {
        let tree = parse_md("# Code\n\n```\nplain code\n```\n");
        let section = &tree.sections[0];
        match &section.structural_nodes[0] {
            StructuralNode::CodeBlock { language, code } => {
                assert!(language.is_empty());
                assert!(code.contains("plain code"));
            }
            other => panic!("expected CodeBlock, got {other:?}"),
        }
    }

    // --- Table tests ---

    #[test]
    fn gfm_table() {
        let tree = parse_md("# Data\n\n| Name | Value |\n|------|-------|\n| a | 1 |\n| b | 2 |\n");
        let section = &tree.sections[0];
        assert_eq!(section.structural_nodes.len(), 1);
        match &section.structural_nodes[0] {
            StructuralNode::Table { headers, rows } => {
                assert_eq!(headers, &["Name", "Value"]);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0], &["a", "1"]);
                assert_eq!(rows[1], &["b", "2"]);
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    // --- List tests ---

    #[test]
    fn unordered_list() {
        let tree = parse_md("# List\n\n- item one\n- item two\n- item three\n");
        let section = &tree.sections[0];
        assert_eq!(section.structural_nodes.len(), 1);
        match &section.structural_nodes[0] {
            StructuralNode::ListBlock { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], "item one");
            }
            other => panic!("expected ListBlock, got {other:?}"),
        }
    }

    #[test]
    fn ordered_list() {
        let tree = parse_md("# Steps\n\n1. first\n2. second\n3. third\n");
        let section = &tree.sections[0];
        match &section.structural_nodes[0] {
            StructuralNode::ListBlock { ordered, items } => {
                assert!(ordered);
                assert_eq!(items.len(), 3);
            }
            other => panic!("expected ListBlock, got {other:?}"),
        }
    }

    // --- Edge cases ---

    #[test]
    fn document_without_headings() {
        let tree = parse_md("Just a paragraph.\n\nAnother paragraph.\n");
        assert_eq!(tree.title, "test");
        assert_eq!(tree.sections.len(), 1);
        assert_eq!(tree.sections[0].depth, 0);
        assert!(tree.sections[0].text.contains("Just a paragraph."));
        assert!(tree.sections[0].text.contains("Another paragraph."));
        assert_eq!(tree.sections[0].id.0, "test.md#root");
    }

    #[test]
    fn empty_document() {
        let tree = parse_md("");
        assert!(tree.sections.is_empty());
    }

    #[test]
    fn heading_only_no_content() {
        let tree = parse_md("# Title\n");
        // Heading with no content below it should be pruned
        assert!(tree.sections.is_empty());
        assert_eq!(tree.title, "Title");
    }

    #[test]
    fn frontmatter_is_skipped() {
        let tree = parse_md("---\ntitle: Test\nauthor: Alice\n---\n\n# Actual Title\n\nContent.\n");
        assert_eq!(tree.title, "Actual Title");
        assert_eq!(tree.sections.len(), 1);
        // Frontmatter should not appear in section text
        assert!(!tree.sections[0].text.contains("author"));
    }

    #[test]
    fn mixed_structural_nodes() {
        let md = "\
# Mixed\n\
\n\
Some text.\n\
\n\
```python\nprint('hello')\n```\n\
\n\
- a\n\
- b\n\
\n\
| x | y |\n\
|---|---|\n\
| 1 | 2 |\n";

        let tree = parse_md(md);
        let section = &tree.sections[0];
        assert_eq!(section.structural_nodes.len(), 3);
        assert!(matches!(
            section.structural_nodes[0],
            StructuralNode::CodeBlock { .. }
        ));
        assert!(matches!(
            section.structural_nodes[1],
            StructuralNode::ListBlock { .. }
        ));
        assert!(matches!(
            section.structural_nodes[2],
            StructuralNode::Table { .. }
        ));
    }

    #[test]
    fn source_path_preserved() {
        let parser = MarkdownParser::new();
        let tree = parser
            .parse(Path::new("docs/api/auth.md"), "# Auth\n\nContent.\n")
            .unwrap();
        assert_eq!(tree.source_path, "docs/api/auth.md");
        assert_eq!(tree.id.0, "docs/api/auth.md");
    }

    #[test]
    fn content_id_is_source_path() {
        let tree = parse_md("# X\n\nY.\n");
        assert_eq!(tree.id.0, "test.md");
    }
}
