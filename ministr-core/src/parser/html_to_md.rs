//! HTML-to-markdown converter with readability-style content extraction.
//!
//! Provides [`ContentExtractor`] for isolating main content from web pages
//! (stripping navigation, sidebars, footers) and [`HtmlToMarkdown`] for
//! converting clean HTML into well-formatted markdown text.
//!
//! The convenience function [`html_to_markdown`] chains both steps:
//! extract main content, then convert to markdown.
//!
//! # Examples
//!
//! ```
//! use ministr_core::parser::html_to_md::html_to_markdown;
//!
//! let html = "<html><body><h1>Hello</h1><p>World.</p></body></html>";
//! let md = html_to_markdown(html);
//! assert!(md.contains("# Hello"));
//! assert!(md.contains("World."));
//! ```

use std::fmt::Write as _;
use std::sync::OnceLock;

use scraper::{ElementRef, Html, Node, Selector};

/// Parse a static CSS selector string, caching the result in a `OnceLock`.
macro_rules! static_selector {
    ($name:ident, $sel:expr) => {
        fn $name() -> &'static Selector {
            static SEL: OnceLock<Selector> = OnceLock::new();
            SEL.get_or_init(|| Selector::parse($sel).expect($sel))
        }
    };
}

static_selector!(sel_body, "body");
static_selector!(sel_p, "p");
static_selector!(sel_a, "a");
static_selector!(sel_code, "code");
static_selector!(sel_tr, "tr");
static_selector!(sel_th, "th");
static_selector!(sel_td, "td");

/// Readability-style main content extractor.
///
/// Strips boilerplate elements (nav, header, footer, sidebar, scripts, styles)
/// and identifies the primary content container using semantic HTML elements
/// (`<main>`, `<article>`) or text-density scoring as a fallback.
pub struct ContentExtractor;

impl ContentExtractor {
    /// Extract the main content HTML from a full web page.
    ///
    /// Returns cleaned HTML containing only the primary content. If no
    /// semantic container (`<main>`, `<article>`) is found, falls back to
    /// text-density scoring of `<div>` elements.
    ///
    /// # Panics
    ///
    /// Panics if the static CSS selector `"body"` fails to parse, which
    /// cannot happen with a well-formed `scraper` installation.
    #[must_use]
    pub fn extract(html: &str) -> String {
        let document = Html::parse_document(html);

        // Try semantic containers first: <main>, then <article>
        for selector_str in &["main", "article"] {
            if let Ok(sel) = Selector::parse(selector_str)
                && let Some(element) = document.select(&sel).next()
            {
                return element.html();
            }
        }

        // Fall back to text-density scoring on the body
        let Some(body) = document.select(sel_body()).next() else {
            return String::new();
        };

        // Score direct children of body (typically divs)
        let mut best_score: f64 = 0.0;
        let mut best_html = String::new();

        for child in body.children().filter_map(ElementRef::wrap) {
            let tag = child.value().name();
            if matches!(
                tag,
                "nav" | "header" | "footer" | "aside" | "script" | "style"
            ) {
                continue;
            }

            let score = score_element(child);
            if score > best_score {
                best_score = score;
                best_html = child.html();
            }
        }

        if best_html.is_empty() {
            body.html()
        } else {
            best_html
        }
    }
}

/// Score an element by text density for readability extraction.
///
/// Higher scores indicate more likely main content. Factors:
/// - Total text length (more text = more likely content)
/// - Paragraph count (bonus for `<p>` tags)
/// - Link density penalty (high ratio of linked text = likely navigation)
#[allow(
    clippy::cast_precision_loss,
    reason = "text lengths fit comfortably in f64"
)]
fn score_element(element: ElementRef<'_>) -> f64 {
    let all_text: String = element.text().collect();
    let text_len = all_text.trim().len() as f64;

    if text_len < 25.0 {
        return 0.0;
    }

    let p_count = element.select(sel_p()).count() as f64;

    let link_text_len: f64 = element
        .select(sel_a())
        .map(|a| a.text().collect::<String>().len() as f64)
        .sum();
    let link_density = if text_len > 0.0 {
        link_text_len / text_len
    } else {
        0.0
    };

    text_len + (p_count * 10.0) - (link_density * text_len * 2.0)
}

/// HTML-to-markdown converter using the `scraper` crate.
///
/// Walks the DOM tree and emits corresponding markdown syntax for
/// headings, paragraphs, code blocks, lists, tables, links, images,
/// and inline formatting.
pub struct HtmlToMarkdown {
    output: String,
    /// Track list nesting depth for indentation.
    list_depth: usize,
}

impl HtmlToMarkdown {
    /// Convert an HTML string to markdown.
    #[must_use]
    pub fn convert(html: &str) -> String {
        let document = Html::parse_fragment(html);
        let mut converter = Self {
            output: String::new(),
            list_depth: 0,
        };

        let root = document.root_element();
        converter.process_children(root);
        converter.cleanup()
    }

    /// Process all children of an element.
    fn process_children(&mut self, element: ElementRef<'_>) {
        for child in element.children() {
            match child.value() {
                Node::Element(_) => {
                    if let Some(el) = ElementRef::wrap(child) {
                        self.process_element(el);
                    }
                }
                Node::Text(text) => {
                    let t = text.text.as_ref();
                    if !t.trim().is_empty() {
                        let collapsed = collapse_whitespace(t);
                        self.output.push_str(&collapsed);
                    }
                }
                _ => {}
            }
        }
    }

    /// Process a single HTML element and its children.
    fn process_element(&mut self, element: ElementRef<'_>) {
        let tag = element.value().name();

        match tag {
            "script" | "style" | "nav" | "header" | "footer" | "aside" | "head" => {}
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.emit_heading(element, tag),
            "p" => self.emit_paragraph(element),
            "br" => self.output.push('\n'),
            "blockquote" => self.emit_blockquote(element),
            "pre" => self.emit_code_block(element),
            "ul" => self.emit_list(element, false),
            "ol" => self.emit_list(element, true),
            "table" => {
                self.ensure_block_boundary();
                self.process_table(element);
            }
            "a" => self.emit_link(element),
            "img" => self.emit_image(element),
            "strong" | "b" => {
                self.output.push_str("**");
                self.process_children(element);
                self.output.push_str("**");
            }
            "em" | "i" => {
                self.output.push('*');
                self.process_children(element);
                self.output.push('*');
            }
            "code" => {
                self.output.push('`');
                let text: String = element.text().collect();
                self.output.push_str(&text);
                self.output.push('`');
            }
            "hr" => {
                self.ensure_block_boundary();
                self.output.push_str("---\n");
            }
            "dl" => {
                self.ensure_block_boundary();
                self.process_children(element);
            }
            "dt" => {
                self.ensure_block_boundary();
                self.output.push_str("**");
                self.process_children(element);
                self.output.push_str("**\n");
            }
            "dd" => {
                self.output.push_str(": ");
                self.process_children(element);
                self.output.push('\n');
            }
            // Structural containers and unknown elements: recurse
            _ => self.process_children(element),
        }
    }

    /// Emit a markdown heading (`# `, `## `, etc.).
    fn emit_heading(&mut self, element: ElementRef<'_>, tag: &str) {
        // SAFETY: tag is matched against "h1"-"h6", so tag[1..] is always "1"-"6".
        let level: usize = tag[1..].parse().expect("h1-h6 digit");
        let hashes = "#".repeat(level);
        let text = collect_inline_text(element);
        self.ensure_block_boundary();
        self.output.push_str(&hashes);
        self.output.push(' ');
        self.output.push_str(text.trim());
        self.output.push('\n');
    }

    /// Emit a paragraph with blank line separation.
    fn emit_paragraph(&mut self, element: ElementRef<'_>) {
        self.ensure_block_boundary();
        self.process_children(element);
        self.output.push('\n');
    }

    /// Emit a blockquote with `> ` prefixed lines.
    fn emit_blockquote(&mut self, element: ElementRef<'_>) {
        self.ensure_block_boundary();
        let inner = Self::convert(&element.inner_html());
        for line in inner.trim().lines() {
            self.output.push_str("> ");
            self.output.push_str(line);
            self.output.push('\n');
        }
    }

    /// Emit a fenced code block with optional language annotation.
    fn emit_code_block(&mut self, element: ElementRef<'_>) {
        self.ensure_block_boundary();
        let (language, code) = if let Some(code_el) = element.select(sel_code()).next() {
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

        self.output.push_str("```");
        self.output.push_str(&language);
        self.output.push('\n');
        self.output.push_str(&code);
        if !code.ends_with('\n') {
            self.output.push('\n');
        }
        self.output.push_str("```\n");
    }

    /// Emit a list (ordered or unordered).
    fn emit_list(&mut self, element: ElementRef<'_>, ordered: bool) {
        self.ensure_block_boundary();
        let children: Vec<ElementRef<'_>> = element
            .children()
            .filter_map(ElementRef::wrap)
            .filter(|el| el.value().name() == "li")
            .collect();

        for (i, li) in children.iter().enumerate() {
            let indent = "  ".repeat(self.list_depth);
            self.output.push_str(&indent);
            if ordered {
                let _ = write!(self.output, "{}. ", i + 1);
            } else {
                self.output.push_str("- ");
            }
            self.list_depth += 1;
            self.process_li_children(*li);
            self.list_depth -= 1;
            if !self.output.ends_with('\n') {
                self.output.push('\n');
            }
        }
    }

    /// Emit a markdown link.
    fn emit_link(&mut self, element: ElementRef<'_>) {
        let href = element.value().attr("href").unwrap_or("");
        let text = collect_inline_text(element);
        let text = text.trim();
        if text.is_empty() {
            // Skip empty links
        } else if href.is_empty() || href.starts_with('#') {
            self.output.push_str(text);
        } else {
            self.output.push('[');
            self.output.push_str(text);
            self.output.push_str("](");
            self.output.push_str(href);
            self.output.push(')');
        }
    }

    /// Emit a markdown image.
    fn emit_image(&mut self, element: ElementRef<'_>) {
        let alt = element.value().attr("alt").unwrap_or("");
        let src = element.value().attr("src").unwrap_or("");
        if !alt.is_empty() || !src.is_empty() {
            self.output.push_str("![");
            self.output.push_str(alt);
            self.output.push_str("](");
            self.output.push_str(src);
            self.output.push(')');
        }
    }

    /// Process the children of a `<li>` element, handling nested lists specially.
    fn process_li_children(&mut self, li: ElementRef<'_>) {
        for child in li.children() {
            match child.value() {
                Node::Element(_) => {
                    if let Some(el) = ElementRef::wrap(child) {
                        let child_tag = el.value().name();
                        if child_tag == "ul" || child_tag == "ol" {
                            if !self.output.ends_with('\n') {
                                self.output.push('\n');
                            }
                            self.process_element(el);
                        } else {
                            self.process_element(el);
                        }
                    }
                }
                Node::Text(text) => {
                    let t = text.text.as_ref().trim();
                    if !t.is_empty() {
                        self.output.push_str(t);
                    }
                }
                _ => {}
            }
        }
    }

    /// Process a `<table>` element into a GFM pipe table.
    fn process_table(&mut self, table: ElementRef<'_>) {
        let mut headers: Vec<String> = Vec::new();
        let mut rows: Vec<Vec<String>> = Vec::new();

        for row in table.select(sel_tr()) {
            let ths: Vec<String> = row
                .select(sel_th())
                .map(|th| collect_inline_text(th).trim().to_string())
                .collect();

            if !ths.is_empty() {
                headers = ths;
                continue;
            }

            let tds: Vec<String> = row
                .select(sel_td())
                .map(|td| collect_inline_text(td).trim().to_string())
                .collect();

            if !tds.is_empty() {
                rows.push(tds);
            }
        }

        if headers.is_empty() && rows.is_empty() {
            return;
        }

        if !headers.is_empty() {
            self.output.push_str("| ");
            self.output.push_str(&headers.join(" | "));
            self.output.push_str(" |\n");

            self.output.push_str("| ");
            let separators: Vec<&str> = headers.iter().map(|_| "---").collect();
            self.output.push_str(&separators.join(" | "));
            self.output.push_str(" |\n");
        }

        for row in &rows {
            self.output.push_str("| ");
            self.output.push_str(&row.join(" | "));
            self.output.push_str(" |\n");
        }
    }

    /// Ensure there is a blank line before the next block element.
    fn ensure_block_boundary(&mut self) {
        let trimmed = self.output.trim_end();
        if trimmed.is_empty() {
            return;
        }
        let trailing_newlines = self.output.len() - trimmed.len();
        if trailing_newlines < 2 {
            for _ in 0..(2 - trailing_newlines) {
                self.output.push('\n');
            }
        }
    }

    /// Clean up the final output: normalize blank lines, trim.
    fn cleanup(self) -> String {
        let mut result = String::with_capacity(self.output.len());
        let mut consecutive_blank = 0;

        for line in self.output.lines() {
            if line.trim().is_empty() {
                consecutive_blank += 1;
                if consecutive_blank <= 1 {
                    result.push('\n');
                }
            } else {
                consecutive_blank = 0;
                result.push_str(line);
                result.push('\n');
            }
        }

        let trimmed = result.trim().to_string();
        if trimmed.is_empty() {
            return trimmed;
        }
        format!("{trimmed}\n")
    }
}

/// Convert HTML to markdown with automatic content extraction.
///
/// First runs [`ContentExtractor`] to isolate main content, then
/// converts the result to markdown via [`HtmlToMarkdown`].
///
/// # Examples
///
/// ```
/// use ministr_core::parser::html_to_md::html_to_markdown;
///
/// let html = r#"<html><body>
///   <nav><a href="/">Home</a></nav>
///   <main><h1>Title</h1><p>Content here.</p></main>
///   <footer>Copyright 2026</footer>
/// </body></html>"#;
///
/// let md = html_to_markdown(html);
/// assert!(md.contains("# Title"));
/// assert!(md.contains("Content here."));
/// assert!(!md.contains("Home"));
/// assert!(!md.contains("Copyright"));
/// ```
#[must_use]
pub fn html_to_markdown(html: &str) -> String {
    let extracted = ContentExtractor::extract(html);
    HtmlToMarkdown::convert(&extracted)
}

/// Collect all text from an element and its descendants (for inline content).
fn collect_inline_text(element: ElementRef<'_>) -> String {
    element.text().collect()
}

/// Collapse multiple whitespace characters into a single space.
fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_was_space = false;

    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(ch);
            prev_was_space = false;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== ContentExtractor tests =====

    #[test]
    fn extract_finds_main_element() {
        let html = "<html><body>\
            <nav><a href=\"/\">Home</a><a href=\"/about\">About</a></nav>\
            <main><h1>Title</h1><p>Main content.</p></main>\
            <footer>Copyright 2026</footer>\
        </body></html>";

        let extracted = ContentExtractor::extract(html);
        assert!(extracted.contains("Main content."));
        assert!(extracted.contains("Title"));
        assert!(!extracted.contains("Home"));
        assert!(!extracted.contains("Copyright"));
    }

    #[test]
    fn extract_finds_article_element() {
        let html = "<html><body>\
            <aside>Sidebar content</aside>\
            <article><h1>Article</h1><p>Article body.</p></article>\
        </body></html>";

        let extracted = ContentExtractor::extract(html);
        assert!(extracted.contains("Article body."));
        assert!(!extracted.contains("Sidebar"));
    }

    #[test]
    fn extract_prefers_main_over_article() {
        let html = "<html><body>\
            <article><p>Article text.</p></article>\
            <main><p>Main text.</p></main>\
        </body></html>";

        let extracted = ContentExtractor::extract(html);
        assert!(extracted.contains("Main text."));
    }

    #[test]
    fn extract_falls_back_to_text_density() {
        let html = "<html><body>\
            <div><a href=\"/\">Home</a> <a href=\"/about\">About</a> <a href=\"/contact\">Contact</a></div>\
            <div>\
                <p>This is a substantial paragraph with real content that should score higher \
                because it has more text and less link density than the navigation div above.</p>\
                <p>Another paragraph with even more content to boost the score.</p>\
            </div>\
            <div><a href=\"/privacy\">Privacy</a> <a href=\"/terms\">Terms</a></div>\
        </body></html>";

        let extracted = ContentExtractor::extract(html);
        assert!(extracted.contains("substantial paragraph"));
        assert!(extracted.contains("Another paragraph"));
    }

    #[test]
    fn extract_empty_body() {
        let html = "<html><body></body></html>";
        let extracted = ContentExtractor::extract(html);
        assert!(extracted.trim().is_empty() || extracted.contains("body"));
    }

    // ===== HtmlToMarkdown tests =====

    #[test]
    fn convert_headings() {
        let html = "<h1>Title</h1><h2>Section</h2><h3>Sub</h3>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("## Section"));
        assert!(md.contains("### Sub"));
    }

    #[test]
    fn convert_paragraphs() {
        let html = "<p>First paragraph.</p><p>Second paragraph.</p>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("First paragraph."));
        assert!(md.contains("Second paragraph."));
        assert!(md.contains("First paragraph.\n\nSecond paragraph."));
    }

    #[test]
    fn convert_code_block_with_language() {
        let html = "<pre><code class=\"language-rust\">fn main() {\n    println!(\"hello\");\n}</code></pre>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("```rust\n"));
        assert!(md.contains("fn main()"));
        assert!(md.contains("```\n"));
    }

    #[test]
    fn convert_code_block_without_language() {
        let html = "<pre><code>some code</code></pre>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("```\nsome code\n```"));
    }

    #[test]
    fn convert_unordered_list() {
        let html = "<ul><li>Alpha</li><li>Beta</li><li>Gamma</li></ul>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("- Alpha\n"));
        assert!(md.contains("- Beta\n"));
        assert!(md.contains("- Gamma\n"));
    }

    #[test]
    fn convert_ordered_list() {
        let html = "<ol><li>First</li><li>Second</li><li>Third</li></ol>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("1. First\n"));
        assert!(md.contains("2. Second\n"));
        assert!(md.contains("3. Third\n"));
    }

    #[test]
    fn convert_nested_list() {
        let html = "<ul><li>Top<ul><li>Nested</li></ul></li></ul>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("- Top\n"));
        assert!(md.contains("  - Nested\n"));
    }

    #[test]
    fn convert_table() {
        let html = "<table>\
            <tr><th>Name</th><th>Value</th></tr>\
            <tr><td>foo</td><td>42</td></tr>\
            <tr><td>bar</td><td>99</td></tr>\
        </table>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("| Name | Value |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| foo | 42 |"));
        assert!(md.contains("| bar | 99 |"));
    }

    #[test]
    fn convert_links() {
        let html = "<p>Visit <a href=\"https://example.com\">Example</a> for more.</p>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("[Example](https://example.com)"));
    }

    #[test]
    fn convert_images() {
        let html = "<img src=\"photo.jpg\" alt=\"A photo\">";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("![A photo](photo.jpg)"));
    }

    #[test]
    fn convert_inline_formatting() {
        let html = "<p><strong>bold</strong> and <em>italic</em> and <code>code</code></p>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("**bold**"));
        assert!(md.contains("*italic*"));
        assert!(md.contains("`code`"));
    }

    #[test]
    fn convert_blockquote() {
        let html = "<blockquote><p>A wise quote.</p></blockquote>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("> A wise quote."));
    }

    #[test]
    fn convert_horizontal_rule() {
        let html = "<p>Before</p><hr><p>After</p>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("---"));
    }

    #[test]
    fn convert_skips_script_and_style() {
        let html = "<p>Real content.</p><script>alert('xss')</script><style>body{}</style>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("Real content."));
        assert!(!md.contains("alert"));
        assert!(!md.contains("body{}"));
    }

    #[test]
    fn convert_empty_html() {
        let md = HtmlToMarkdown::convert("");
        assert!(md.is_empty());
    }

    #[test]
    fn convert_fragment_only_links_emit_text() {
        let html = "<a href=\"#section\">Jump</a>";
        let md = HtmlToMarkdown::convert(html);
        assert!(md.contains("Jump"));
        assert!(!md.contains("[Jump]"));
    }

    // ===== html_to_markdown integration test =====

    #[test]
    fn full_page_to_markdown() {
        let html = "<!DOCTYPE html>\
<html>\
<head><title>Docs</title></head>\
<body>\
    <nav>\
        <a href=\"/\">Home</a>\
        <a href=\"/docs\">Docs</a>\
        <a href=\"/api\">API</a>\
    </nav>\
    <main>\
        <h1>Getting Started</h1>\
        <p>Welcome to the <strong>documentation</strong>.</p>\
        <h2>Installation</h2>\
        <p>Run the following command:</p>\
        <pre><code class=\"language-bash\">cargo install ministr</code></pre>\
        <h2>Configuration</h2>\
        <p>Create a config file with these settings:</p>\
        <table>\
            <tr><th>Key</th><th>Default</th></tr>\
            <tr><td>timeout</td><td>30</td></tr>\
            <tr><td>retries</td><td>3</td></tr>\
        </table>\
        <h3>Advanced</h3>\
        <ul>\
            <li>Option A: for power users</li>\
            <li>Option B: for simplicity</li>\
        </ul>\
    </main>\
    <footer>\
        <p>Copyright 2026 Ministr Project</p>\
    </footer>\
</body>\
</html>";

        let md = html_to_markdown(html);

        assert!(md.contains("# Getting Started"));
        assert!(md.contains("## Installation"));
        assert!(md.contains("## Configuration"));
        assert!(md.contains("### Advanced"));
        assert!(md.contains("**documentation**"));
        assert!(md.contains("```bash\ncargo install ministr\n```"));
        assert!(md.contains("| Key | Default |"));
        assert!(md.contains("- Option A: for power users"));

        assert!(!md.contains("Home"));
        assert!(!md.contains("Copyright"));
    }

    #[test]
    fn documentation_page_with_sidebar() {
        let html = "<html><body>\
            <div class=\"sidebar\">\
                <a href=\"/intro\">Introduction</a>\
                <a href=\"/guide\">Guide</a>\
                <a href=\"/api\">API Reference</a>\
                <a href=\"/faq\">FAQ</a>\
            </div>\
            <div class=\"content\">\
                <h1>API Reference</h1>\
                <p>This page documents the public API surface.</p>\
                <h2>Authentication</h2>\
                <p>All endpoints require a bearer token.</p>\
                <pre><code class=\"language-bash\">curl -H \"Authorization: Bearer TOKEN\" https://api.example.com/v1/data</code></pre>\
                <h2>Endpoints</h2>\
                <ol>\
                    <li>GET /users - list users</li>\
                    <li>POST /users - create user</li>\
                    <li>GET /users/:id - get user</li>\
                </ol>\
            </div>\
        </body></html>";

        let md = html_to_markdown(html);

        assert!(md.contains("API Reference"));
        assert!(md.contains("bearer token"));
        assert!(md.contains("```bash"));
    }
}
