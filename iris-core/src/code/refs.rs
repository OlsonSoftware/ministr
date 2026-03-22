//! Cross-reference extraction from tree-sitter AST nodes.
//!
//! Extracts raw (unresolved) cross-reference candidates from Rust source code:
//! `use` imports, `impl Trait for Type` relationships, function/method calls,
//! and type usage in signatures. These are resolved against the stored symbol
//! table during ingestion to produce [`SymbolRefRecord`]s.

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

/// Primitive and built-in type names to exclude from type-usage references.
const PRIMITIVE_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8", "u16",
    "u32", "u64", "u128", "usize", "Self",
];

/// Extract raw cross-reference candidates from a tree-sitter AST.
///
/// Walks the AST looking for:
/// - `use` declarations → `RefKind::Imports`
/// - `impl Trait for Type` blocks → `RefKind::Implements`
/// - `call_expression` nodes → `RefKind::Calls`
/// - Type identifiers in function signatures → `RefKind::Uses`
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
            "impl_item" => {
                extract_impl_refs(&node, source, &mut refs);
                extract_from_impl_body(&node, source, &mut refs);
            }
            "function_item" => {
                extract_from_function(&node, source, &mut refs);
            }
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

/// Extract calls and type-usage refs from a top-level `function_item`.
fn extract_from_function(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let fn_name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok());

    // Extract type usage from parameters and return type.
    if let Some(params) = node.child_by_field_name("parameters") {
        collect_type_identifiers(&params, source, fn_name, refs);
    }
    if let Some(ret) = node.child_by_field_name("return_type") {
        collect_type_identifiers(&ret, source, fn_name, refs);
    }

    // Extract call expressions from the function body.
    if let Some(body) = node.child_by_field_name("body") {
        walk_for_calls(&body, source, fn_name, refs);
    }
}

/// Extract calls and type-usage refs from methods inside an `impl` block body.
fn extract_from_impl_body(
    impl_node: &tree_sitter::Node<'_>,
    source: &[u8],
    refs: &mut Vec<RawRef>,
) {
    // The impl block body is the `declaration_list` (or `body` field).
    let Some(body) = impl_node.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" {
            extract_from_function(&child, source, refs);
        }
    }
}

/// Recursively walk a subtree looking for `call_expression` nodes.
fn walk_for_calls(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    fn_context: Option<&str>,
    refs: &mut Vec<RawRef>,
) {
    if node.kind() == "call_expression" {
        extract_call_ref(node, source, fn_context, refs);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_calls(&child, source, fn_context, refs);
    }
}

/// Extract a single call reference from a `call_expression` node.
///
/// Handles three callee patterns:
/// - `identifier` → direct function call (`bar()`)
/// - `scoped_identifier` → qualified call (`MyType::new()`)
/// - `field_expression` → method call (`x.baz()`)
fn extract_call_ref(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    fn_context: Option<&str>,
    refs: &mut Vec<RawRef>,
) {
    let Some(func_node) = node.child_by_field_name("function") else {
        return;
    };

    let callee_name = match func_node.kind() {
        "identifier" => func_node.utf8_text(source).ok(),
        "scoped_identifier" => {
            // Extract the final `name` segment (e.g., `new` from `MyType::new`).
            func_node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
        }
        "field_expression" => {
            // Method call: extract the `field` (method name).
            func_node
                .child_by_field_name("field")
                .and_then(|n| n.utf8_text(source).ok())
        }
        _ => None,
    };

    if let Some(name) = callee_name {
        #[allow(clippy::cast_possible_truncation)]
        let line = node.start_position().row as u32 + 1;
        refs.push(RawRef {
            target_name: name.to_string(),
            kind: RefKind::Calls,
            line,
            from_context: fn_context.map(String::from),
        });
    }
}

/// Recursively collect `type_identifier` names from a type annotation subtree.
///
/// Walks through `generic_type`, `reference_type`, `scoped_type_identifier`,
/// `tuple_type`, `array_type`, etc. to find all named types. Filters out
/// primitive types to reduce noise.
fn collect_type_identifiers(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    fn_context: Option<&str>,
    refs: &mut Vec<RawRef>,
) {
    match node.kind() {
        "type_identifier" => {
            if let Ok(name) = node.utf8_text(source) {
                if !PRIMITIVE_TYPES.contains(&name) {
                    #[allow(clippy::cast_possible_truncation)]
                    let line = node.start_position().row as u32 + 1;
                    refs.push(RawRef {
                        target_name: name.to_string(),
                        kind: RefKind::Uses,
                        line,
                        from_context: fn_context.map(String::from),
                    });
                }
            }
        }
        "scoped_type_identifier" => {
            // For `path::Type`, extract the final type name.
            if let Some(name_node) = node.child_by_field_name("name") {
                collect_type_identifiers(&name_node, source, fn_context, refs);
            }
        }
        _ => {
            // Recurse into children for generic_type, reference_type,
            // tuple_type, array_type, parameters, etc.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_type_identifiers(&child, source, fn_context, refs);
            }
        }
    }
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

    // --- Call graph extraction tests (C8.0) ---

    #[test]
    fn extract_direct_function_call() {
        let source = r"
fn main() {
    let x = foo();
}
";
        let refs = parse_and_extract(source);
        let calls: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Calls).collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].target_name, "foo");
        assert_eq!(calls[0].from_context.as_deref(), Some("main"));
    }

    #[test]
    fn extract_method_call() {
        let source = r"
fn process() {
    let x = vec.push(42);
}
";
        let refs = parse_and_extract(source);
        let calls: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Calls).collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].target_name, "push");
        assert_eq!(calls[0].from_context.as_deref(), Some("process"));
    }

    #[test]
    fn extract_scoped_call() {
        let source = r"
fn build() {
    let cfg = Config::new();
}
";
        let refs = parse_and_extract(source);
        let calls: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Calls).collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].target_name, "new");
        assert_eq!(calls[0].from_context.as_deref(), Some("build"));
    }

    #[test]
    fn extract_multiple_calls() {
        let source = r"
fn work() {
    let a = foo();
    let b = x.bar();
    let c = Baz::create();
}
";
        let refs = parse_and_extract(source);
        let calls: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Calls).collect();
        assert_eq!(calls.len(), 3);
        let names: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"create"));
    }

    #[test]
    fn extract_calls_from_impl_methods() {
        let source = r"
struct MyStruct;
impl MyStruct {
    fn do_work(&self) {
        helper();
    }
}
";
        let refs = parse_and_extract(source);
        let calls: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Calls).collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].target_name, "helper");
        assert_eq!(calls[0].from_context.as_deref(), Some("do_work"));
    }

    // --- Type usage extraction tests (C8.1) ---

    #[test]
    fn extract_parameter_types() {
        let source = r"
fn process(config: Config, items: Vec<Item>) {}
";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        let names: Vec<&str> = uses.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Config"), "missing Config, got: {names:?}");
        assert!(names.contains(&"Vec"), "missing Vec, got: {names:?}");
        assert!(names.contains(&"Item"), "missing Item, got: {names:?}");
    }

    #[test]
    fn extract_return_type() {
        let source = r"
fn create() -> Result<Config, Error> { todo!() }
";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        let names: Vec<&str> = uses.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Result"), "missing Result, got: {names:?}");
        assert!(names.contains(&"Config"), "missing Config, got: {names:?}");
        assert!(names.contains(&"Error"), "missing Error, got: {names:?}");
    }

    #[test]
    fn extract_reference_type() {
        let source = r"
fn borrow(x: &MyStruct) {}
";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].target_name, "MyStruct");
    }

    #[test]
    fn skip_primitive_types() {
        let source = r"
fn primitives(a: u32, b: bool, c: &str) -> i64 { 0 }
";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        assert!(uses.is_empty(), "should skip primitives, got: {uses:?}");
    }

    #[test]
    fn skip_self_type() {
        let source = r"
struct Foo;
impl Foo {
    fn identity(self) -> Self { self }
}
";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        assert!(
            !uses.iter().any(|r| r.target_name == "Self"),
            "should skip Self type"
        );
    }

    #[test]
    fn extract_type_from_context() {
        let source = r"
fn process(config: Config) {}
";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].from_context.as_deref(), Some("process"));
    }
}
