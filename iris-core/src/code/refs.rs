//! Cross-reference extraction from tree-sitter AST nodes.
//!
//! Extracts raw (unresolved) cross-reference candidates from Rust source code:
//! `use` imports and `impl Trait for Type` relationships. These are resolved
//! against the stored symbol table during ingestion to produce [`SymbolRefRecord`]s.

use crate::types::RefKind;

/// An unresolved cross-reference candidate extracted from a tree-sitter AST.
///
/// Contains the target path segments and reference kind, but no resolved
/// symbol IDs. Resolution happens during ingestion when the full symbol
/// table is available.
///
/// # Examples
///
/// ```
/// use iris_core::code::refs::RawRef;
/// use iris_core::types::RefKind;
///
/// let raw = RawRef {
///     target_name: "IrisConfig".to_string(),
///     kind: RefKind::Imports,
///     line: 5,
///     from_context: None,
/// };
/// assert_eq!(raw.kind, RefKind::Imports);
/// ```
#[derive(Debug, Clone)]
pub struct RawRef {
    /// The name of the target symbol (last segment of a use path, or trait/type name).
    pub target_name: String,
    /// The kind of reference.
    pub kind: RefKind,
    /// Source line number where the reference appears.
    pub line: u32,
    /// For `impl Trait for Type`: the implementing type name (the "from" side).
    /// For imports: `None` (the whole file is the "from" context).
    pub from_context: Option<String>,
}

/// Extract raw cross-reference candidates from a tree-sitter AST.
///
/// Walks the AST looking for:
/// - `use` declarations → `RefKind::Imports`
/// - `impl Trait for Type` blocks → `RefKind::Implements`
///
/// Returns unresolved references that must be matched against the symbol
/// table to produce `SymbolRefRecord` values.
#[must_use]
pub fn extract_refs(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    let mut refs = Vec::new();
    let root = tree.root_node();

    let mut cursor = root.walk();
    for node in root.children(&mut cursor) {
        match node.kind() {
            "use_declaration" => extract_use_refs(&node, source, &mut refs),
            "impl_item" => extract_impl_refs(&node, source, &mut refs),
            _ => {}
        }
    }

    refs
}

/// Extract import references from a `use_declaration` node.
fn extract_use_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    #[allow(clippy::cast_possible_truncation)]
    let line = node.start_position().row as u32 + 1;

    // The argument child contains the use path.
    // It can be a scoped_identifier, use_list, use_as_clause, or identifier.
    if let Some(arg) = node.child_by_field_name("argument") {
        collect_use_names(&arg, source, line, refs);
    }
}

/// Recursively collect imported symbol names from a use path node.
fn collect_use_names(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    line: u32,
    refs: &mut Vec<RawRef>,
) {
    match node.kind() {
        "identifier" | "type_identifier" => {
            if let Ok(name) = node.utf8_text(source) {
                // Skip `self`, `super`, `crate` — these are path prefixes, not symbols
                if name != "self" && name != "super" && name != "crate" {
                    refs.push(RawRef {
                        target_name: name.to_string(),
                        kind: RefKind::Imports,
                        line,
                        from_context: None,
                    });
                }
            }
        }
        "scoped_identifier" | "scoped_use_list" => {
            // For scoped_identifier: the `name` field is the imported symbol.
            // For scoped_use_list: recurse into the `list` field.
            if let Some(name_node) = node.child_by_field_name("name") {
                collect_use_names(&name_node, source, line, refs);
            }
            if let Some(list_node) = node.child_by_field_name("list") {
                collect_use_names(&list_node, source, line, refs);
            }
        }
        "use_list" => {
            // Iterate over children (each is an identifier, scoped_identifier, etc.)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_use_names(&child, source, line, refs);
            }
        }
        "use_as_clause" => {
            // `use Foo as Bar` — the original name is the first child
            if let Some(path) = node.child_by_field_name("path") {
                collect_use_names(&path, source, line, refs);
            }
        }
        _ => {}
    }
}

/// Extract `impl Trait for Type` references from an `impl_item` node.
fn extract_impl_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    // Check if this is a trait impl (has a `trait` field).
    let Some(trait_node) = node.child_by_field_name("trait") else {
        return; // Inherent impl, not a trait impl
    };

    let Some(type_node) = node.child_by_field_name("type") else {
        return;
    };

    let Ok(trait_name) = trait_node.utf8_text(source) else {
        return;
    };

    let Ok(type_name) = type_node.utf8_text(source) else {
        return;
    };

    // Strip generic parameters for matching (e.g., "Display" not "Display<T>")
    let trait_name = trait_name.split('<').next().unwrap_or(trait_name).trim();
    let type_name = type_name.split('<').next().unwrap_or(type_name).trim();

    #[allow(clippy::cast_possible_truncation)]
    let line = node.start_position().row as u32 + 1;

    refs.push(RawRef {
        target_name: trait_name.to_string(),
        kind: RefKind::Implements,
        line,
        from_context: Some(type_name.to_string()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::AstParser;

    fn parse_and_extract(source: &str) -> Vec<RawRef> {
        let mut parser = AstParser::new();
        let tree = parser.parse(source.as_bytes()).unwrap();
        extract_refs(&tree, source.as_bytes())
    }

    #[test]
    fn extract_simple_use() {
        let refs = parse_and_extract("use std::collections::HashMap;");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_name, "HashMap");
        assert_eq!(refs[0].kind, RefKind::Imports);
    }

    #[test]
    fn extract_grouped_use() {
        let refs = parse_and_extract("use std::collections::{HashMap, BTreeMap};");
        assert_eq!(refs.len(), 2);
        let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"HashMap"));
        assert!(names.contains(&"BTreeMap"));
    }

    #[test]
    fn extract_use_as() {
        let refs = parse_and_extract("use std::collections::HashMap as Map;");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_name, "HashMap");
    }

    #[test]
    fn extract_crate_use() {
        let refs = parse_and_extract("use crate::config::IrisConfig;");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_name, "IrisConfig");
    }

    #[test]
    fn extract_impl_trait_for_type() {
        let source = r"
            pub struct Foo;
            pub trait Bar {}
            impl Bar for Foo {}
        ";
        let refs = parse_and_extract(source);
        let impl_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Implements)
            .collect();
        assert_eq!(impl_refs.len(), 1);
        assert_eq!(impl_refs[0].target_name, "Bar");
        assert_eq!(impl_refs[0].from_context.as_deref(), Some("Foo"));
    }

    #[test]
    fn skip_inherent_impl() {
        let source = r"
            pub struct Foo;
            impl Foo {
                fn new() -> Self { Foo }
            }
        ";
        let refs = parse_and_extract(source);
        let impl_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Implements)
            .collect();
        assert!(impl_refs.is_empty());
    }

    #[test]
    fn extract_use_wildcard_skipped() {
        let refs = parse_and_extract("use std::collections::*;");
        assert!(refs.is_empty());
    }

    #[test]
    fn skip_self_and_crate_identifiers() {
        let refs = parse_and_extract("use crate::config::IrisConfig;");
        // Should only have IrisConfig, not "crate" or "config"
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_name, "IrisConfig");
    }
}
