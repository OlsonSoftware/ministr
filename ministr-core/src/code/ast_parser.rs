//! Tree-sitter AST parser and top-level item walker for Rust source code.

use std::ops::Range;

use crate::error::ParseError;

/// The kind of a top-level Rust item identified by the AST walker.
///
/// # Examples
///
/// ```
/// use ministr_core::code::ItemKind;
///
/// let kind = ItemKind::Function;
/// assert_eq!(kind.as_str(), "function");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// `fn` item.
    Function,
    /// `struct` item.
    Struct,
    /// `enum` item.
    Enum,
    /// `trait` definition.
    Trait,
    /// `impl` block.
    Impl,
    /// `mod` item.
    Module,
    /// `type` alias.
    Type,
    /// `const` item.
    Const,
    /// `static` item.
    Static,
}

impl ItemKind {
    /// Returns the string representation of this item kind.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Module => "module",
            Self::Type => "type",
            Self::Const => "const",
            Self::Static => "static",
        }
    }

    /// Parse an item kind from a tree-sitter node kind string.
    ///
    /// Returns `None` for unrecognized node kinds.
    #[must_use]
    pub fn from_node_kind(kind: &str) -> Option<Self> {
        match kind {
            "function_item" => Some(Self::Function),
            "struct_item" => Some(Self::Struct),
            "enum_item" => Some(Self::Enum),
            "trait_item" => Some(Self::Trait),
            "impl_item" => Some(Self::Impl),
            "mod_item" => Some(Self::Module),
            "type_item" => Some(Self::Type),
            "const_item" => Some(Self::Const),
            "static_item" => Some(Self::Static),
            _ => None,
        }
    }
}

/// A top-level item discovered by the AST tree walker.
///
/// # Examples
///
/// ```
/// use ministr_core::code::{AstItem, ItemKind};
///
/// let item = AstItem {
///     kind: ItemKind::Function,
///     name: "main".into(),
///     byte_range: 0..15,
///     node_kind: "function_item".into(),
/// };
/// assert_eq!(item.kind, ItemKind::Function);
/// assert_eq!(item.name, "main");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstItem {
    /// The kind of item.
    pub kind: ItemKind,
    /// The item's name (or the impl target type for `impl` blocks).
    pub name: String,
    /// Byte range in the source file.
    pub byte_range: Range<usize>,
    /// The raw tree-sitter node kind string.
    pub node_kind: String,
}

/// A tree-sitter parser that can be initialized with any language grammar.
///
/// # Examples
///
/// ```
/// use ministr_core::code::AstParser;
///
/// let mut parser = AstParser::new();
/// let tree = parser.parse(b"fn main() {}").unwrap();
/// assert_eq!(tree.root_node().kind(), "source_file");
/// ```
pub struct AstParser {
    parser: tree_sitter::Parser,
}

impl AstParser {
    /// Create a new parser initialized with the Rust language grammar.
    ///
    /// Infallible by construction: loading the statically-linked
    /// `tree-sitter-rust` grammar only fails on an ABI-version mismatch,
    /// which is a build-time invariant of the pinned dependency. If it
    /// ever does fail we log and return a parser with no language set —
    /// [`parse`](Self::parse) then yields [`ParseError::Failed`] rather
    /// than panicking. Prefer [`try_new`](Self::try_new) when the caller
    /// can propagate the error.
    #[must_use]
    pub fn new() -> Self {
        Self::try_new().unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to load tree-sitter Rust grammar");
            Self {
                parser: tree_sitter::Parser::new(),
            }
        })
    }

    /// Create a new parser initialized with the Rust language grammar,
    /// returning an error instead of degrading if the grammar fails to
    /// load.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::Failed`] if the tree-sitter Rust grammar
    /// cannot be loaded (ABI-version mismatch).
    pub fn try_new() -> Result<Self, ParseError> {
        Self::with_language(&tree_sitter_rust::LANGUAGE.into())
    }

    /// Create a parser initialized with the specified language grammar.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::Failed`] if the language grammar cannot be loaded.
    #[must_use = "constructors return a new value"]
    pub fn with_language(language: &tree_sitter::Language) -> Result<Self, ParseError> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(language)
            .map_err(|e| ParseError::Failed {
                path: std::path::PathBuf::from("<language>"),
                reason: format!("failed to load tree-sitter grammar: {e}"),
            })?;
        Ok(Self { parser })
    }

    /// Parse source bytes into a tree-sitter syntax tree.
    ///
    /// A `PARSE_BUDGET` per-file wall-clock budget is enforced via a
    /// `ParseOptions` progress callback so pathological inputs (deeply
    /// nested templates, recursion bombs in C++ headers) can't hang the
    /// ingestion pipeline. On timeout, `parse_with_options` returns
    /// `None` and surfaces here as `ParseError::Failed`; the producer
    /// records the path in `failed_files` and moves on rather than
    /// blocking a worker indefinitely.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::Failed`] if tree-sitter fails to produce a tree
    /// (e.g. due to a timeout or cancellation).
    #[must_use = "returns the parsed syntax tree"]
    pub fn parse(&mut self, source: &[u8]) -> Result<tree_sitter::Tree, ParseError> {
        let len = source.len();
        let mut chunk_cb =
            |i: usize, _: tree_sitter::Point| -> &[u8] { if i < len { &source[i..] } else { &[] } };
        let deadline = std::time::Instant::now() + PARSE_BUDGET;
        let mut progress_cb = |_state: &tree_sitter::ParseState| {
            if std::time::Instant::now() >= deadline {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        };
        let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress_cb);
        self.parser
            .parse_with_options(&mut chunk_cb, None, Some(options))
            .ok_or_else(|| ParseError::Failed {
                path: std::path::PathBuf::from("<source>"),
                reason: "tree-sitter failed to produce a parse tree (timeout or cancellation)"
                    .into(),
            })
    }
}

/// Maximum wall time allowed for a single tree-sitter parse. 5 seconds
/// — long enough for a multi-MB file, short enough that a stuck parse
/// can't stall the pipeline.
const PARSE_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

impl Default for AstParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Walk the top-level children of a tree-sitter syntax tree and identify
/// Rust item nodes.
///
/// Returns an [`AstItem`] for each recognized top-level item. Nodes that
/// don't match any known item kind (e.g. `use_declaration`, `line_comment`)
/// are silently skipped.
///
/// # Examples
///
/// ```
/// use ministr_core::code::{AstParser, ItemKind, walk_top_level_items};
///
/// let mut parser = AstParser::new();
/// let source = b"struct Foo;\nfn bar() {}\n";
/// let tree = parser.parse(source).unwrap();
/// let items = walk_top_level_items(&tree, source);
/// assert_eq!(items.len(), 2);
/// assert_eq!(items[0].kind, ItemKind::Struct);
/// assert_eq!(items[1].kind, ItemKind::Function);
/// ```
#[must_use]
pub fn walk_top_level_items(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<AstItem> {
    let root = tree.root_node();
    let mut items = Vec::new();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        let Some(kind) = ItemKind::from_node_kind(child.kind()) else {
            continue;
        };

        let name = extract_item_name(&child, source, kind);

        items.push(AstItem {
            kind,
            name,
            byte_range: child.start_byte()..child.end_byte(),
            node_kind: child.kind().to_string(),
        });
    }

    items
}

/// Extract the name of an item from its AST node.
///
/// For most items this is the `name` child node's text. For `impl` blocks,
/// it's the `type` child (the type being implemented).
pub(crate) fn extract_item_name(node: &tree_sitter::Node, source: &[u8], kind: ItemKind) -> String {
    if kind == ItemKind::Impl {
        // impl blocks: look for the `type` field (the Self type)
        return node
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .unwrap_or("<unknown>")
            .to_string();
    }

    // Most items have a `name` field
    node.child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .unwrap_or("<unknown>")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_kind_from_node_kind_roundtrip() {
        let cases = [
            ("function_item", ItemKind::Function),
            ("struct_item", ItemKind::Struct),
            ("enum_item", ItemKind::Enum),
            ("trait_item", ItemKind::Trait),
            ("impl_item", ItemKind::Impl),
            ("mod_item", ItemKind::Module),
            ("type_item", ItemKind::Type),
            ("const_item", ItemKind::Const),
            ("static_item", ItemKind::Static),
        ];
        for (node_kind, expected) in cases {
            assert_eq!(ItemKind::from_node_kind(node_kind), Some(expected));
        }
    }

    #[test]
    fn item_kind_from_unknown_returns_none() {
        assert_eq!(ItemKind::from_node_kind("use_declaration"), None);
        assert_eq!(ItemKind::from_node_kind("line_comment"), None);
        assert_eq!(ItemKind::from_node_kind(""), None);
    }

    #[test]
    fn parse_simple_rust_source() {
        let mut parser = AstParser::new();
        let source = b"fn main() {}";
        let tree = parser.parse(source).unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
        assert!(tree.root_node().child_count() > 0);
    }

    #[test]
    fn walk_identifies_all_item_kinds() {
        let mut parser = AstParser::new();
        let source = b"
const MAX: usize = 42;
static GLOBAL: i32 = 0;
type Alias = Vec<u8>;
struct Foo { x: i32 }
enum Bar { A, B }
trait Baz { fn do_thing(&self); }
impl Foo { fn new() -> Self { Self { x: 0 } } }
mod inner {}
fn free_fn() {}
";
        let tree = parser.parse(source).unwrap();
        let items = walk_top_level_items(&tree, source);

        let kinds: Vec<ItemKind> = items.iter().map(|i| i.kind).collect();
        assert!(kinds.contains(&ItemKind::Const));
        assert!(kinds.contains(&ItemKind::Static));
        assert!(kinds.contains(&ItemKind::Type));
        assert!(kinds.contains(&ItemKind::Struct));
        assert!(kinds.contains(&ItemKind::Enum));
        assert!(kinds.contains(&ItemKind::Trait));
        assert!(kinds.contains(&ItemKind::Impl));
        assert!(kinds.contains(&ItemKind::Module));
        assert!(kinds.contains(&ItemKind::Function));
        assert_eq!(items.len(), 9);
    }

    #[test]
    fn walk_extracts_correct_names() {
        let mut parser = AstParser::new();
        let source = b"
struct MyStruct;
fn my_function() {}
impl MyStruct { fn method(&self) {} }
";
        let tree = parser.parse(source).unwrap();
        let items = walk_top_level_items(&tree, source);

        assert_eq!(items[0].name, "MyStruct");
        assert_eq!(items[0].kind, ItemKind::Struct);

        assert_eq!(items[1].name, "my_function");
        assert_eq!(items[1].kind, ItemKind::Function);

        assert_eq!(items[2].name, "MyStruct");
        assert_eq!(items[2].kind, ItemKind::Impl);
    }

    #[test]
    fn walk_skips_use_declarations_and_comments() {
        let mut parser = AstParser::new();
        let source = b"
// A comment
use std::path::Path;
use std::io;
fn only_fn() {}
";
        let tree = parser.parse(source).unwrap();
        let items = walk_top_level_items(&tree, source);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ItemKind::Function);
        assert_eq!(items[0].name, "only_fn");
    }

    #[test]
    fn walk_byte_ranges_are_valid() {
        let mut parser = AstParser::new();
        let source = b"fn hello() {}\nstruct World;";
        let tree = parser.parse(source).unwrap();
        let items = walk_top_level_items(&tree, source);

        assert_eq!(items.len(), 2);
        for item in &items {
            assert!(item.byte_range.start < item.byte_range.end);
            assert!(item.byte_range.end <= source.len());
            // Verify the slice contains the item name
            let slice = &source[item.byte_range.clone()];
            let text = std::str::from_utf8(slice).unwrap();
            assert!(text.contains(&item.name));
        }
    }

    // C1.3: Parse real ministr-core source files

    #[test]
    fn parse_config_rs_identifies_structs_and_impls() {
        let source = std::fs::read("src/config.rs").expect("cannot read config.rs");
        let mut parser = AstParser::new();
        let tree = parser.parse(&source).unwrap();
        let items = walk_top_level_items(&tree, &source);

        // config.rs should contain MinistrConfig struct and PrefetchConfig struct
        let struct_names: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == ItemKind::Struct)
            .map(|i| i.name.as_str())
            .collect();
        assert!(
            struct_names.contains(&"MinistrConfig"),
            "expected MinistrConfig struct, found: {struct_names:?}"
        );
        assert!(
            struct_names.contains(&"PrefetchConfig"),
            "expected PrefetchConfig struct, found: {struct_names:?}"
        );

        // Should have impl blocks for MinistrConfig
        let impl_names: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == ItemKind::Impl)
            .map(|i| i.name.as_str())
            .collect();
        assert!(
            impl_names.contains(&"MinistrConfig"),
            "expected MinistrConfig impl, found: {impl_names:?}"
        );

        // Should have at least some functions (free functions like default_data_dir)
        let fn_count = items
            .iter()
            .filter(|i| i.kind == ItemKind::Function)
            .count();
        assert!(fn_count > 0, "expected at least one free function");
    }

    #[test]
    fn parse_pipeline_rs_identifies_structs_and_consts() {
        let source = std::fs::read("src/ingestion/pipeline.rs").expect("cannot read pipeline.rs");
        let mut parser = AstParser::new();
        let tree = parser.parse(&source).unwrap();
        let items = walk_top_level_items(&tree, &source);

        // pipeline.rs should contain IngestionStats struct
        let struct_names: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == ItemKind::Struct)
            .map(|i| i.name.as_str())
            .collect();
        assert!(
            struct_names.contains(&"IngestionStats"),
            "expected IngestionStats struct, found: {struct_names:?}"
        );
        assert!(
            struct_names.contains(&"IngestionPipeline"),
            "expected IngestionPipeline struct, found: {struct_names:?}"
        );

        // Should have impl blocks
        let impl_count = items.iter().filter(|i| i.kind == ItemKind::Impl).count();
        assert!(impl_count > 0, "expected at least one impl block");
    }
}
