//! Symbol extraction from tree-sitter AST nodes.
//!
//! Extracts rich symbol metadata from Rust source code: visibility modifiers,
//! doc comments, signatures (declaration without body), and module paths.
//! Builds on [`AstParser`] and tree-sitter to produce [`Symbol`] values.

use std::ops::Range;

use crate::code::ast_parser::ItemKind;

/// Visibility level of a Rust symbol.
///
/// # Examples
///
/// ```
/// use iris_core::code::Visibility;
///
/// let vis = Visibility::Public;
/// assert_eq!(vis.as_str(), "pub");
///
/// let vis = Visibility::PubCrate;
/// assert_eq!(vis.as_str(), "pub(crate)");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// `pub` — visible everywhere.
    Public,
    /// `pub(crate)` — visible within the crate.
    PubCrate,
    /// `pub(super)` — visible to the parent module.
    PubSuper,
    /// `pub(in path)` — visible to the specified path.
    PubIn(String),
    /// No visibility modifier — private to the current module.
    Private,
}

impl Visibility {
    /// Returns the string representation of this visibility.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Public => "pub",
            Self::PubCrate => "pub(crate)",
            Self::PubSuper => "pub(super)",
            Self::PubIn(path) => {
                // We can't return a borrowed reference to a formatted string,
                // so callers needing the full `pub(in ...)` form should use Display.
                // For filtering, "pub(in)" is sufficient.
                let _ = path;
                "pub(in)"
            }
            Self::Private => "",
        }
    }

    /// Whether this visibility makes the symbol accessible outside the module.
    #[must_use]
    pub fn is_public(&self) -> bool {
        matches!(self, Self::Public)
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Public => f.write_str("pub"),
            Self::PubCrate => f.write_str("pub(crate)"),
            Self::PubSuper => f.write_str("pub(super)"),
            Self::PubIn(path) => write!(f, "pub(in {path})"),
            Self::Private => Ok(()),
        }
    }
}

/// A symbol extracted from Rust source code with full metadata.
///
/// # Examples
///
/// ```
/// use iris_core::code::{Symbol, Visibility, ItemKind};
///
/// let sym = Symbol {
///     name: "IrisConfig".into(),
///     kind: ItemKind::Struct,
///     visibility: Visibility::Public,
///     signature: "pub struct IrisConfig".into(),
///     doc_comment: Some("Configuration for iris.".into()),
///     annotations: vec!["#[derive(Debug)]".into()],
///     file_path: "src/config.rs".into(),
///     byte_range: 100..250,
///     module_path: vec!["config".into()],
/// };
/// assert_eq!(sym.name, "IrisConfig");
/// assert!(sym.visibility.is_public());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// The symbol's name (or impl target type for `impl` blocks).
    pub name: String,
    /// The kind of symbol (function, struct, etc.).
    pub kind: ItemKind,
    /// Visibility modifier.
    pub visibility: Visibility,
    /// The declaration line(s) without the body — the signature.
    pub signature: String,
    /// The `///` or `//!` doc comment text, if present.
    pub doc_comment: Option<String>,
    /// Decorators, annotations, or attributes attached to this symbol.
    ///
    /// Examples: `@property`, `@Override`, `#[derive(Debug)]`, `@Component`.
    pub annotations: Vec<String>,
    /// Source file path (relative to corpus root).
    pub file_path: String,
    /// Byte range in the source file covering the entire item (including doc comments).
    pub byte_range: Range<usize>,
    /// Module path segments (e.g. `["config"]` for items in `config.rs`).
    pub module_path: Vec<String>,
}

/// Extract symbols from a parsed tree-sitter syntax tree.
///
/// Walks top-level items and extracts visibility, doc comments, and signatures.
///
/// # Errors
///
/// Returns [`ParseError::Failed`] if the source cannot be interpreted as UTF-8.
///
/// # Examples
///
/// ```
/// use iris_core::code::{AstParser, extract_symbols, ItemKind, Visibility};
///
/// let mut parser = AstParser::new();
/// let source = b"/// A greeting.\npub fn hello() {}";
/// let tree = parser.parse(source).unwrap();
/// let symbols = extract_symbols(&tree, source, "lib.rs", &["mymod"]);
/// assert_eq!(symbols.len(), 1);
/// assert_eq!(symbols[0].name, "hello");
/// assert_eq!(symbols[0].visibility, Visibility::Public);
/// assert_eq!(symbols[0].doc_comment.as_deref(), Some("A greeting."));
/// ```
#[must_use]
pub fn extract_symbols(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    module_path: &[&str],
) -> Vec<Symbol> {
    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        let Some(kind) = ItemKind::from_node_kind(child.kind()) else {
            continue;
        };

        let name = super::ast_parser::extract_item_name(&child, source, kind);
        let visibility = extract_visibility(&child, source);
        let doc_comment = extract_doc_comment(&child, source);
        let signature = extract_signature(&child, source);

        // Extend byte range to include preceding doc comments
        let byte_start = doc_comment_start_byte(&child, source).unwrap_or(child.start_byte());

        let sym_name = name.clone();
        symbols.push(Symbol {
            name,
            kind,
            visibility,
            signature,
            doc_comment,
            annotations: Vec::new(),
            file_path: file_path.to_string(),
            byte_range: byte_start..child.end_byte(),
            module_path: module_path.iter().map(|s| (*s).to_string()).collect(),
        });

        // Recurse into impl/trait bodies to extract methods
        if kind == ItemKind::Impl || kind == ItemKind::Trait {
            if let Some(body) = child.child_by_field_name("body") {
                let mut method_path: Vec<String> =
                    module_path.iter().map(|s| (*s).to_string()).collect();
                method_path.push(sym_name);

                let method_module: Vec<&str> = method_path.iter().map(String::as_str).collect();
                let mut body_cursor = body.walk();
                for body_child in body.children(&mut body_cursor) {
                    let Some(child_kind) = ItemKind::from_node_kind(body_child.kind()) else {
                        continue;
                    };
                    let method_name =
                        super::ast_parser::extract_item_name(&body_child, source, child_kind);
                    let method_vis = extract_visibility(&body_child, source);
                    let method_doc = extract_doc_comment(&body_child, source);
                    let method_sig = extract_signature(&body_child, source);
                    let method_start = doc_comment_start_byte(&body_child, source)
                        .unwrap_or(body_child.start_byte());

                    symbols.push(Symbol {
                        name: method_name,
                        kind: child_kind,
                        visibility: method_vis,
                        signature: method_sig,
                        doc_comment: method_doc,
                        annotations: Vec::new(),
                        file_path: file_path.to_string(),
                        byte_range: method_start..body_child.end_byte(),
                        module_path: method_module.iter().map(|s| (*s).to_string()).collect(),
                    });
                }
            }
        }
    }

    symbols
}

/// Extract the visibility modifier from an item node.
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    let Some(vis_node) = node.child_by_field_name("visibility") else {
        // Also check for a direct `visibility_modifier` child (some items use this)
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "visibility_modifier" {
                return parse_visibility_text(child.utf8_text(source).unwrap_or(""));
            }
        }
        return Visibility::Private;
    };

    parse_visibility_text(vis_node.utf8_text(source).unwrap_or(""))
}

/// Parse visibility text like `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`.
fn parse_visibility_text(text: &str) -> Visibility {
    let text = text.trim();
    if text == "pub" {
        Visibility::Public
    } else if text == "pub(crate)" {
        Visibility::PubCrate
    } else if text == "pub(super)" {
        Visibility::PubSuper
    } else if let Some(inner) = text
        .strip_prefix("pub(in ")
        .and_then(|s| s.strip_suffix(')'))
    {
        Visibility::PubIn(inner.trim().to_string())
    } else if text.starts_with("pub(") {
        // Handle `pub(crate)` etc. with varying whitespace
        Visibility::PubCrate
    } else {
        Visibility::Private
    }
}

/// Extract doc comments (`///` lines) that immediately precede an item node.
fn extract_doc_comment(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut doc_lines = Vec::new();

    // Walk backwards through siblings to find consecutive doc comment lines
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        let text = sibling.utf8_text(source).unwrap_or("");
        if sibling.kind() == "line_comment" && text.starts_with("///") {
            // Strip the `/// ` or `///` prefix and any trailing whitespace/newlines
            let content = text
                .trim_end()
                .strip_prefix("/// ")
                .or_else(|| text.trim_end().strip_prefix("///"))
                .unwrap_or("");
            doc_lines.push(content.to_string());
        } else if sibling.kind() == "block_comment" && text.starts_with("/**") {
            // Block doc comment
            let content = text
                .strip_prefix("/**")
                .and_then(|s| s.strip_suffix("*/"))
                .unwrap_or("")
                .trim();
            if !content.is_empty() {
                doc_lines.push(content.to_string());
            }
            break;
        } else if sibling.kind() == "attribute_item" || sibling.kind() == "inner_attribute_item" {
            // Skip attributes like #[derive(...)] — they sit between doc comments and the item
            prev = sibling.prev_sibling();
            continue;
        } else {
            break;
        }
        prev = sibling.prev_sibling();
    }

    if doc_lines.is_empty() {
        return None;
    }

    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

/// Find the byte offset where the doc comment block starts (for `byte_range` extension).
fn doc_comment_start_byte(node: &tree_sitter::Node, source: &[u8]) -> Option<usize> {
    let mut earliest_start = None;
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        let text = sibling.utf8_text(source).unwrap_or("");
        if sibling.kind() == "line_comment" && text.starts_with("///") {
            earliest_start = Some(sibling.start_byte());
        } else if sibling.kind() == "block_comment" && text.starts_with("/**") {
            earliest_start = Some(sibling.start_byte());
            break;
        } else if sibling.kind() == "attribute_item" || sibling.kind() == "inner_attribute_item" {
            earliest_start = Some(sibling.start_byte());
            prev = sibling.prev_sibling();
            continue;
        } else {
            break;
        }
        prev = sibling.prev_sibling();
    }

    earliest_start
}

/// Extract the signature (declaration without body) from an item node.
///
/// For items with a body block `{ ... }`, takes everything up to (not including)
/// the opening brace. For items without a body (e.g. `struct Foo;`), takes the
/// full text.
fn extract_signature(node: &tree_sitter::Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");

    // For items with a body, find the opening brace
    if let Some(body_node) = node.child_by_field_name("body") {
        let body_start = body_node.start_byte() - node.start_byte();
        let sig = &text[..body_start];
        return sig.trim_end().to_string();
    }

    // For impl blocks, the body is in the `body` or we look for `{`
    // For items without a body field, look for `{` in the text
    if let Some(brace_pos) = text.find('{') {
        return text[..brace_pos].trim_end().to_string();
    }

    // No body — use full text (e.g. `struct Foo;`, `type Alias = Vec<u8>;`)
    text.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::AstParser;

    #[test]
    fn extract_public_function_symbol() {
        let mut parser = AstParser::new();
        let source = b"pub fn hello() -> String { String::new() }";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &["mymod"]);

        assert_eq!(symbols.len(), 1);
        let sym = &symbols[0];
        assert_eq!(sym.name, "hello");
        assert_eq!(sym.kind, ItemKind::Function);
        assert_eq!(sym.visibility, Visibility::Public);
        assert_eq!(sym.signature, "pub fn hello() -> String");
        assert!(sym.doc_comment.is_none());
        assert_eq!(sym.file_path, "lib.rs");
        assert_eq!(sym.module_path, vec!["mymod"]);
    }

    #[test]
    fn extract_doc_comment() {
        let mut parser = AstParser::new();
        let source = b"/// Says hello.\n/// Returns a greeting.\npub fn hello() {}";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols.len(), 1);
        assert_eq!(
            symbols[0].doc_comment.as_deref(),
            Some("Says hello.\nReturns a greeting.")
        );
    }

    #[test]
    fn extract_private_visibility() {
        let mut parser = AstParser::new();
        let source = b"fn private_fn() {}";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols[0].visibility, Visibility::Private);
    }

    #[test]
    fn extract_pub_crate_visibility() {
        let mut parser = AstParser::new();
        let source = b"pub(crate) struct Internal { x: i32 }";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols[0].visibility, Visibility::PubCrate);
        assert_eq!(symbols[0].kind, ItemKind::Struct);
    }

    #[test]
    fn extract_struct_signature_without_body() {
        let mut parser = AstParser::new();
        let source = b"pub struct Unit;";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols[0].signature, "pub struct Unit;");
    }

    #[test]
    fn extract_struct_signature_with_fields() {
        let mut parser = AstParser::new();
        let source = b"pub struct Foo {\n    x: i32,\n    y: String,\n}";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols[0].signature, "pub struct Foo");
    }

    #[test]
    fn extract_trait_symbol() {
        let mut parser = AstParser::new();
        let source = b"/// A trait for parsing.\npub trait Parser {\n    fn parse(&self);\n}";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "parser.rs", &["parser"]);

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Parser");
        assert_eq!(symbols[0].kind, ItemKind::Trait);
        assert_eq!(symbols[0].visibility, Visibility::Public);
        assert_eq!(
            symbols[0].doc_comment.as_deref(),
            Some("A trait for parsing.")
        );
        assert_eq!(symbols[0].module_path, vec!["parser"]);
    }

    #[test]
    fn extract_impl_block() {
        let mut parser = AstParser::new();
        let source = b"impl Foo {\n    pub fn new() -> Self { Self {} }\n}";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols.len(), 2, "expected impl + 1 method");
        assert_eq!(symbols[0].name, "Foo");
        assert_eq!(symbols[0].kind, ItemKind::Impl);
        assert_eq!(symbols[1].name, "new");
        assert_eq!(symbols[1].kind, ItemKind::Function);
        assert_eq!(symbols[1].module_path, vec!["Foo"]);
    }

    #[test]
    fn extract_const_and_static() {
        let mut parser = AstParser::new();
        let source = b"pub const MAX: usize = 100;\nstatic COUNTER: i32 = 0;";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].kind, ItemKind::Const);
        assert_eq!(symbols[0].visibility, Visibility::Public);
        assert_eq!(symbols[1].kind, ItemKind::Static);
        assert_eq!(symbols[1].visibility, Visibility::Private);
    }

    #[test]
    fn extract_enum_with_doc() {
        let mut parser = AstParser::new();
        let source = b"/// Color options.\npub enum Color {\n    Red,\n    Blue,\n}";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols[0].name, "Color");
        assert_eq!(symbols[0].kind, ItemKind::Enum);
        assert_eq!(symbols[0].doc_comment.as_deref(), Some("Color options."));
    }

    #[test]
    fn doc_comment_with_attributes_skipped() {
        let mut parser = AstParser::new();
        let source =
            b"/// Documented struct.\n#[derive(Debug)]\npub struct WithAttr {\n    x: i32,\n}";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols[0].name, "WithAttr");
        assert_eq!(
            symbols[0].doc_comment.as_deref(),
            Some("Documented struct.")
        );
        // byte_range should extend back to include the doc comment
        assert_eq!(symbols[0].byte_range.start, 0);
    }

    #[test]
    fn extract_type_alias() {
        let mut parser = AstParser::new();
        let source = b"pub type Result<T> = std::result::Result<T, Error>;";
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols[0].name, "Result");
        assert_eq!(symbols[0].kind, ItemKind::Type);
        assert_eq!(symbols[0].visibility, Visibility::Public);
    }

    // Test against real iris-core source files

    #[test]
    fn extract_symbols_from_config_rs() {
        let source = std::fs::read("src/config.rs").expect("cannot read config.rs");
        let mut parser = AstParser::new();
        let tree = parser.parse(&source).unwrap();
        let symbols = extract_symbols(&tree, &source, "src/config.rs", &["config"]);

        // Should have IrisConfig struct
        let iris_config = symbols
            .iter()
            .find(|s| s.name == "IrisConfig" && s.kind == ItemKind::Struct);
        assert!(
            iris_config.is_some(),
            "expected IrisConfig struct, found: {:?}",
            symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        let ic = iris_config.unwrap();
        assert_eq!(ic.visibility, Visibility::Public);
        assert!(
            ic.doc_comment.is_some(),
            "IrisConfig should have a doc comment"
        );
        assert!(ic.signature.contains("IrisConfig"));

        // Should have PrefetchConfig struct
        let prefetch = symbols
            .iter()
            .find(|s| s.name == "PrefetchConfig" && s.kind == ItemKind::Struct);
        assert!(prefetch.is_some(), "expected PrefetchConfig struct");
    }

    #[test]
    fn extract_symbols_from_ingestion_rs() {
        let source = std::fs::read("src/ingestion.rs").expect("cannot read ingestion.rs");
        let mut parser = AstParser::new();
        let tree = parser.parse(&source).unwrap();
        let symbols = extract_symbols(&tree, &source, "src/ingestion.rs", &["ingestion"]);

        // Should have IngestionStats struct
        let stats = symbols
            .iter()
            .find(|s| s.name == "IngestionStats" && s.kind == ItemKind::Struct);
        assert!(stats.is_some(), "expected IngestionStats struct");
        let s = stats.unwrap();
        assert_eq!(s.visibility, Visibility::Public);
        assert!(s.doc_comment.is_some());

        // Should have SUMMARY_MAX_SENTENCES const
        let constant = symbols
            .iter()
            .find(|s| s.name == "SUMMARY_MAX_SENTENCES" && s.kind == ItemKind::Const);
        assert!(constant.is_some(), "expected SUMMARY_MAX_SENTENCES const");

        // All top-level symbols have module_path ["ingestion"],
        // methods inside impl blocks have ["ingestion", "TypeName"]
        for sym in &symbols {
            assert_eq!(
                sym.module_path[0], "ingestion",
                "first module path segment should be 'ingestion' for symbol {}",
                sym.name
            );
        }
    }
}
