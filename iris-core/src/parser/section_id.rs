//! Stable hierarchical section ID generation.
//!
//! Section IDs are deterministic and derived from the file path and heading
//! hierarchy, so the same document content always produces the same IDs.
//! Format: `{source_path}#{slug-chain}` where the slug chain is the
//! kebab-case heading text joined by `/`.

/// Generate a stable section ID from a source path and heading hierarchy.
///
/// # Examples
///
/// ```
/// use iris_core::parser::generate_section_id;
///
/// let id = generate_section_id("docs/auth.md", &["Getting Started", "Error Handling"]);
/// assert_eq!(id, "docs/auth.md#getting-started/error-handling");
/// ```
///
/// For documents without headings, pass an empty heading path to get the
/// document root section:
///
/// ```
/// use iris_core::parser::generate_section_id;
///
/// let id = generate_section_id("notes.md", &[]);
/// assert_eq!(id, "notes.md#root");
/// ```
#[must_use]
pub fn generate_section_id(source_path: &str, heading_path: &[&str]) -> String {
    if heading_path.is_empty() {
        return format!("{source_path}#root");
    }

    let slug_chain: String = heading_path
        .iter()
        .map(|h| slugify(h))
        .collect::<Vec<_>>()
        .join("/");

    format!("{source_path}#{slug_chain}")
}

/// Convert a heading string to a URL-safe kebab-case slug.
///
/// Strips non-alphanumeric characters (except hyphens and spaces),
/// lowercases, and joins words with hyphens.
fn slugify(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut prev_was_separator = false;

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if prev_was_separator && !slug.is_empty() {
                slug.push('-');
            }
            for lower in ch.to_lowercase() {
                slug.push(lower);
            }
            prev_was_separator = false;
        } else {
            // Spaces, hyphens, underscores, punctuation all become separator boundaries
            prev_was_separator = true;
        }
    }

    slug
}

/// Generate a section ID for a code symbol.
///
/// Code section IDs use the pattern `{source_path}#{module_path::SymbolName}`,
/// preserving the original Rust naming (no slugification).
///
/// # Examples
///
/// ```
/// use iris_core::parser::generate_code_section_id;
///
/// let id = generate_code_section_id("src/config.rs", &["config"], "IrisConfig");
/// assert_eq!(id, "src/config.rs#config::IrisConfig");
///
/// let id = generate_code_section_id("src/lib.rs", &[], "main");
/// assert_eq!(id, "src/lib.rs#main");
/// ```
#[must_use]
pub fn generate_code_section_id(
    source_path: &str,
    module_path: &[&str],
    symbol_name: &str,
) -> String {
    if module_path.is_empty() {
        return format!("{source_path}#{symbol_name}");
    }
    let module = module_path.join("::");
    format!("{source_path}#{module}::{symbol_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_section_id() {
        let id = generate_section_id("docs/api.md", &["Authentication"]);
        assert_eq!(id, "docs/api.md#authentication");
    }

    #[test]
    fn nested_heading_path() {
        let id = generate_section_id("docs/auth.md", &["Getting Started", "Error Handling"]);
        assert_eq!(id, "docs/auth.md#getting-started/error-handling");
    }

    #[test]
    fn deeply_nested() {
        let id = generate_section_id(
            "docs/api.md",
            &["Chapter 3", "Section 3.2", "Error Handling"],
        );
        assert_eq!(id, "docs/api.md#chapter-3/section-3-2/error-handling");
    }

    #[test]
    fn empty_heading_path_gives_root() {
        let id = generate_section_id("notes.md", &[]);
        assert_eq!(id, "notes.md#root");
    }

    #[test]
    fn special_characters_stripped() {
        let id = generate_section_id("doc.md", &["What's New?", "v2.0 (Release)"]);
        assert_eq!(id, "doc.md#what-s-new/v2-0-release");
    }

    #[test]
    fn unicode_headings() {
        let id = generate_section_id("doc.md", &["Über Uns"]);
        assert_eq!(id, "doc.md#über-uns");
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("Error Handling"), "error-handling");
    }

    #[test]
    fn slugify_multiple_spaces() {
        assert_eq!(slugify("a   b   c"), "a-b-c");
    }

    #[test]
    fn slugify_leading_trailing() {
        assert_eq!(slugify("  hello  "), "hello");
    }

    #[test]
    fn slugify_numbers() {
        assert_eq!(slugify("Section 3.2"), "section-3-2");
    }

    #[test]
    fn code_section_id_with_module() {
        let id = generate_code_section_id("src/config.rs", &["config"], "IrisConfig");
        assert_eq!(id, "src/config.rs#config::IrisConfig");
    }

    #[test]
    fn code_section_id_nested_module() {
        let id = generate_code_section_id("src/ingestion.rs", &["ingestion"], "IngestionPipeline");
        assert_eq!(id, "src/ingestion.rs#ingestion::IngestionPipeline");
    }

    #[test]
    fn code_section_id_no_module() {
        let id = generate_code_section_id("src/lib.rs", &[], "main");
        assert_eq!(id, "src/lib.rs#main");
    }

    #[test]
    fn code_section_id_deep_module_path() {
        let id = generate_code_section_id("src/a.rs", &["a", "b", "c"], "Foo");
        assert_eq!(id, "src/a.rs#a::b::c::Foo");
    }
}
