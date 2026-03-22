//! llms.txt fetcher and parser.
//!
//! Implements the [llms.txt specification](https://llmstxt.org/) for discovering
//! and parsing LLM-friendly site descriptions. Given a domain, the fetcher tries
//! `https://{domain}/llms-full.txt` first (returning raw markdown), then falls
//! back to `https://{domain}/llms.txt` (returning a parsed structure of title,
//! description, and categorized link lists).

use crate::error::WebError;
use crate::web::HttpClient;

/// Content retrieved from a domain's llms.txt endpoint.
///
/// # Examples
///
/// ```
/// use iris_core::llms_txt::LlmsTxtContent;
///
/// let full = LlmsTxtContent::Full("# My Site\n\nAll content here.".into());
/// assert!(matches!(full, LlmsTxtContent::Full(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmsTxtContent {
    /// Raw markdown content from `llms-full.txt`.
    Full(String),
    /// Parsed structured content from `llms.txt`.
    Parsed(LlmsTxt),
}

/// A parsed llms.txt file with title, description, and categorized link sections.
///
/// # Examples
///
/// ```
/// use iris_core::llms_txt::LlmsTxt;
///
/// let txt = LlmsTxt {
///     title: "My Project".into(),
///     description: Some("A cool project.".into()),
///     details: None,
///     sections: vec![],
/// };
/// assert_eq!(txt.title, "My Project");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmsTxt {
    /// Site or project title (from the H1 heading).
    pub title: String,
    /// Brief description (from the blockquote following the title).
    pub description: Option<String>,
    /// Additional detail paragraphs before the first H2 section.
    pub details: Option<String>,
    /// Categorized link sections.
    pub sections: Vec<LlmsTxtSection>,
}

/// A categorized section within an llms.txt file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmsTxtSection {
    /// Section heading text (from `## Heading`).
    pub name: String,
    /// Links listed under this section.
    pub links: Vec<LlmsTxtLink>,
    /// Whether this section is marked as optional (heading is "Optional").
    pub is_optional: bool,
}

/// A link entry within an llms.txt section.
///
/// Format: `- [title](url): description` or `- [title](url)`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmsTxtLink {
    /// Display title of the link.
    pub title: String,
    /// Target URL.
    pub url: String,
    /// Optional description text after the colon.
    pub description: Option<String>,
}

/// Parse an llms.txt markdown string into a structured [`LlmsTxt`].
///
/// Follows the [llms.txt specification](https://llmstxt.org/):
/// - H1 heading → title (required)
/// - Blockquote lines after title → description
/// - Non-heading content before first H2 → details
/// - H2 sections with `- [title](url): description` link entries
/// - Section named "Optional" is flagged as optional
///
/// # Examples
///
/// ```
/// use iris_core::llms_txt::parse_llms_txt;
///
/// let content = "# My Site\n\n> A brief description.\n\n## Docs\n\n- [Guide](https://example.com/guide): Getting started\n";
/// let parsed = parse_llms_txt(content);
/// assert_eq!(parsed.title, "My Site");
/// assert_eq!(parsed.description.as_deref(), Some("A brief description."));
/// assert_eq!(parsed.sections.len(), 1);
/// assert_eq!(parsed.sections[0].links.len(), 1);
/// ```
#[must_use]
pub fn parse_llms_txt(content: &str) -> LlmsTxt {
    let mut title = String::new();
    let mut description_lines: Vec<String> = Vec::new();
    let mut detail_lines: Vec<String> = Vec::new();
    let mut sections: Vec<LlmsTxtSection> = Vec::new();

    let mut found_title = false;
    let mut in_description = false;
    let mut in_section = false;

    for line in content.lines() {
        // H1 title
        if !found_title {
            if let Some(h1) = line.strip_prefix("# ") {
                h1.trim().clone_into(&mut title);
                found_title = true;
                in_description = true;
                continue;
            }
            // Skip blank lines before the title
            continue;
        }

        // H2 section header
        if let Some(h2) = line.strip_prefix("## ") {
            let name = h2.trim().to_owned();
            let is_optional = name.eq_ignore_ascii_case("optional");
            sections.push(LlmsTxtSection {
                name,
                links: Vec::new(),
                is_optional,
            });
            in_description = false;
            in_section = true;
            continue;
        }

        // Blockquote description (only right after the title, before any other content)
        if in_description {
            if let Some(quote) = line.strip_prefix("> ") {
                description_lines.push(quote.trim().to_owned());
                continue;
            }
            if line.trim().is_empty() {
                if !description_lines.is_empty() {
                    // Blank line after blockquote ends the description zone
                    in_description = false;
                }
                continue;
            }
            // Non-blank, non-blockquote line → we're past the description
            in_description = false;
        }

        // Link entries inside a section
        if in_section {
            if let Some(link) = parse_link_line(line) {
                if let Some(section) = sections.last_mut() {
                    section.links.push(link);
                }
                continue;
            }
            // Other lines in sections are ignored (blank lines, etc.)
            continue;
        }

        // Detail lines (between description and first H2)
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            detail_lines.push(trimmed.to_owned());
        }
    }

    let description = if description_lines.is_empty() {
        None
    } else {
        Some(description_lines.join("\n"))
    };

    let details = if detail_lines.is_empty() {
        None
    } else {
        Some(detail_lines.join("\n"))
    };

    LlmsTxt {
        title,
        description,
        details,
        sections,
    }
}

/// Parse a single link line in the format `- [title](url): description` or `- [title](url)`.
fn parse_link_line(line: &str) -> Option<LlmsTxtLink> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("- ")?;

    // Expect [title](url)
    let rest = rest.strip_prefix('[')?;
    let (title, rest) = rest.split_once(']')?;
    let rest = rest.strip_prefix('(')?;
    let (url, rest) = rest.split_once(')')?;

    let title = title.trim().to_owned();
    let url = url.trim().to_owned();

    if title.is_empty() || url.is_empty() {
        return None;
    }

    // Optional `: description`
    let description = rest
        .strip_prefix(':')
        .map(|d| d.trim().to_owned())
        .filter(|d| !d.is_empty());

    Some(LlmsTxtLink {
        title,
        url,
        description,
    })
}

/// Fetch llms.txt content for a domain.
///
/// Tries `https://{domain}/llms-full.txt` first. If that returns 200 OK,
/// the raw body is returned as [`LlmsTxtContent::Full`]. Otherwise, tries
/// `https://{domain}/llms.txt` and parses it as [`LlmsTxtContent::Parsed`].
///
/// # Errors
///
/// Returns [`WebError::LlmsTxtNotFound`] if both endpoints return 404.
/// Other HTTP or transport errors are propagated from the underlying
/// [`HttpClient`].
///
/// # Examples
///
/// ```no_run
/// # use iris_core::web::HttpClient;
/// # use iris_core::llms_txt::fetch_llms_txt;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = HttpClient::with_defaults()?;
/// let content = fetch_llms_txt(&client, "example.com").await?;
/// # Ok(())
/// # }
/// ```
#[tracing::instrument(skip(client), fields(domain = %domain))]
pub async fn fetch_llms_txt(client: &HttpClient, domain: &str) -> Result<LlmsTxtContent, WebError> {
    // Try llms-full.txt first
    let full_url = format!("https://{domain}/llms-full.txt");
    match client.get(&full_url).await {
        Ok(response) => {
            return Ok(LlmsTxtContent::Full(response.body));
        }
        Err(WebError::HttpStatus { status: 404, .. }) => {
            // Fall through to llms.txt
        }
        Err(e) => return Err(e),
    }

    // Try llms.txt
    let txt_url = format!("https://{domain}/llms.txt");
    match client.get(&txt_url).await {
        Ok(response) => {
            let parsed = parse_llms_txt(&response.body);
            Ok(LlmsTxtContent::Parsed(parsed))
        }
        Err(WebError::HttpStatus { status: 404, .. }) => Err(WebError::LlmsTxtNotFound {
            domain: domain.to_owned(),
        }),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_link_line tests --

    #[test]
    fn parse_link_with_description() {
        let link =
            parse_link_line("- [Guide](https://example.com/guide): Getting started guide").unwrap();
        assert_eq!(link.title, "Guide");
        assert_eq!(link.url, "https://example.com/guide");
        assert_eq!(link.description.as_deref(), Some("Getting started guide"));
    }

    #[test]
    fn parse_link_without_description() {
        let link = parse_link_line("- [API](https://example.com/api)").unwrap();
        assert_eq!(link.title, "API");
        assert_eq!(link.url, "https://example.com/api");
        assert!(link.description.is_none());
    }

    #[test]
    fn parse_link_rejects_empty_title() {
        assert!(parse_link_line("- [](https://example.com)").is_none());
    }

    #[test]
    fn parse_link_rejects_empty_url() {
        assert!(parse_link_line("- [Title]()").is_none());
    }

    #[test]
    fn parse_link_rejects_non_link_line() {
        assert!(parse_link_line("Just a regular line").is_none());
        assert!(parse_link_line("").is_none());
        assert!(parse_link_line("  ").is_none());
    }

    // -- parse_llms_txt tests --

    #[test]
    fn parse_minimal_title_only() {
        let content = "# My Site\n";
        let parsed = parse_llms_txt(content);
        assert_eq!(parsed.title, "My Site");
        assert!(parsed.description.is_none());
        assert!(parsed.details.is_none());
        assert!(parsed.sections.is_empty());
    }

    #[test]
    fn parse_title_and_description() {
        let content = "# My Site\n\n> A brief description of the site.\n> With a second line.\n";
        let parsed = parse_llms_txt(content);
        assert_eq!(parsed.title, "My Site");
        assert_eq!(
            parsed.description.as_deref(),
            Some("A brief description of the site.\nWith a second line.")
        );
    }

    #[test]
    fn parse_full_structure() {
        let content = "\
# Anthropic

> Anthropic is an AI safety company.

Some additional details here.

## Docs

- [API Reference](https://docs.anthropic.com/api): Complete API reference
- [User Guide](https://docs.anthropic.com/guide): Getting started guide

## SDKs

- [Python SDK](https://github.com/anthropics/anthropic-sdk-python)
- [TypeScript SDK](https://github.com/anthropics/anthropic-sdk-typescript)

## Optional

- [Blog](https://anthropic.com/blog): Company blog
";
        let parsed = parse_llms_txt(content);
        assert_eq!(parsed.title, "Anthropic");
        assert_eq!(
            parsed.description.as_deref(),
            Some("Anthropic is an AI safety company.")
        );
        assert_eq!(
            parsed.details.as_deref(),
            Some("Some additional details here.")
        );
        assert_eq!(parsed.sections.len(), 3);

        // Docs section
        assert_eq!(parsed.sections[0].name, "Docs");
        assert!(!parsed.sections[0].is_optional);
        assert_eq!(parsed.sections[0].links.len(), 2);
        assert_eq!(parsed.sections[0].links[0].title, "API Reference");
        assert_eq!(
            parsed.sections[0].links[0].url,
            "https://docs.anthropic.com/api"
        );
        assert_eq!(
            parsed.sections[0].links[0].description.as_deref(),
            Some("Complete API reference")
        );

        // SDKs section
        assert_eq!(parsed.sections[1].name, "SDKs");
        assert_eq!(parsed.sections[1].links.len(), 2);
        assert!(parsed.sections[1].links[0].description.is_none());

        // Optional section
        assert_eq!(parsed.sections[2].name, "Optional");
        assert!(parsed.sections[2].is_optional);
        assert_eq!(parsed.sections[2].links.len(), 1);
    }

    #[test]
    fn parse_cursor_style_llms_txt() {
        let content = "\
# Cursor

> The AI Code Editor. Build software faster with AI.

## Documentation

- [Getting Started](https://docs.cursor.com/get-started): Installation and setup
- [Tab](https://docs.cursor.com/tab): Cursor's native autocomplete
- [Chat](https://docs.cursor.com/chat): AI chat features
- [Context](https://docs.cursor.com/context): How context works

## Optional

- [Privacy](https://docs.cursor.com/privacy): Privacy & security details
- [Troubleshooting](https://docs.cursor.com/troubleshooting): Common issues
";
        let parsed = parse_llms_txt(content);
        assert_eq!(parsed.title, "Cursor");
        assert_eq!(
            parsed.description.as_deref(),
            Some("The AI Code Editor. Build software faster with AI.")
        );
        assert!(parsed.details.is_none());
        assert_eq!(parsed.sections.len(), 2);
        assert_eq!(parsed.sections[0].name, "Documentation");
        assert_eq!(parsed.sections[0].links.len(), 4);
        assert!(parsed.sections[1].is_optional);
    }

    #[test]
    fn parse_llms_full_txt_as_raw() {
        // llms-full.txt is just raw markdown, not parsed — but if someone
        // accidentally passes it through parse_llms_txt, it should still
        // extract the title at minimum
        let content = "\
# My Project

> Overview of the project.

## Introduction

This is the full documentation content. It has paragraphs, code blocks,
and other markdown elements that are NOT link lists.

```rust
fn main() {
    println!(\"Hello, world!\");
}
```

## API Reference

The API provides endpoints for managing resources.
";
        let parsed = parse_llms_txt(content);
        assert_eq!(parsed.title, "My Project");
        assert_eq!(
            parsed.description.as_deref(),
            Some("Overview of the project.")
        );
        // Sections exist but have no links (because content isn't link lists)
        assert_eq!(parsed.sections.len(), 2);
        assert!(parsed.sections[0].links.is_empty());
        assert!(parsed.sections[1].links.is_empty());
    }

    #[test]
    fn parse_empty_content() {
        let parsed = parse_llms_txt("");
        assert!(parsed.title.is_empty());
        assert!(parsed.description.is_none());
        assert!(parsed.sections.is_empty());
    }

    #[test]
    fn parse_details_between_description_and_sections() {
        let content = "\
# Site

> Description.

First detail paragraph.
Second detail line.

## Links

- [Home](https://example.com)
";
        let parsed = parse_llms_txt(content);
        assert_eq!(
            parsed.details.as_deref(),
            Some("First detail paragraph.\nSecond detail line.")
        );
    }
}
