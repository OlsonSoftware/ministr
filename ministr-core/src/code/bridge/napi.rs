//! NAPI-RS bridge extractor for `#[napi]` bindings.
//!
//! Detects cross-language bridges in napi-rs projects:
//!
//! - **Rust exports** — functions, structs, and enums annotated with `#[napi]`
//! - **JS/TS imports** — `import { name } from './native'` or `require('./native')`
//!
//! Implements [`BridgeExtractor`] and can be registered with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

// ---------------------------------------------------------------------------
// NapiExtractor
// ---------------------------------------------------------------------------

/// Extracts napi-rs bindings from Rust and JS/TS source files.
///
/// **Rust exports** — items annotated with `#[napi]`:
/// ```rust,ignore
/// #[napi]
/// fn add(a: i32, b: i32) -> i32 { a + b }
///
/// #[napi(constructor)]
/// pub struct MyClass { pub name: String }
/// ```
///
/// **JS/TS imports** — named imports from native module:
/// ```javascript,ignore
/// import { add, MyClass } from './native';
/// const { add } = require('./index.node');
/// ```
///
/// The binding key is the symbol name. The linker handles case normalization
/// (`snake_case` ↔ `camelCase`).
pub struct NapiExtractor;

impl BridgeExtractor for NapiExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::Napi
    }

    fn applicable_languages(&self) -> &[&str] {
        &["rust", "javascript", "typescript", "tsx"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        match language {
            "rust" => extract_rust_napi_exports(tree, source, file_path),
            "javascript" | "typescript" | "tsx" => {
                extract_js_napi_imports(tree, source, file_path, language)
            }
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rust export extraction
// ---------------------------------------------------------------------------

/// Find `#[napi]` annotated items and produce Export endpoints.
fn extract_rust_napi_exports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_rust_napi_items(&mut cursor, source, file_path, &mut endpoints);
    endpoints
}

/// Recursively walk the tree looking for items with `#[napi]` attribute.
///
/// Detects functions, structs, enums, and methods inside `impl` blocks
/// that carry the `#[napi]` annotation.
fn walk_rust_napi_items(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();
        let kind = node.kind();

        match kind {
            "function_item" | "function_definition" | "struct_item" | "enum_item" => {
                if has_napi_attribute_before(&node, source)
                    && let Some(name) = rust_item_name(&node, source)
                {
                    #[allow(clippy::cast_possible_truncation)]
                    let line = node.start_position().row as u32 + 1;
                    endpoints.push(BridgeEndpoint {
                        binding_key: name.clone(),
                        kind: BridgeKind::Napi,
                        role: EndpointRole::Export,
                        language: "rust".into(),
                        file_path: file_path.into(),
                        line,
                        symbol_name: name,
                        confidence: ConfidenceLevel::CaseTransformed.score(),
                    });
                }
            }
            "impl_item" => {
                // Walk into impl blocks to find #[napi] methods
                if has_napi_attribute_before(&node, source) {
                    // The whole impl is #[napi] — extract methods from inside
                    walk_napi_impl_methods(cursor, source, file_path, endpoints);
                }
            }
            _ => {}
        }

        // Recurse into children
        if cursor.goto_first_child() {
            walk_rust_napi_items(cursor, source, file_path, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Walk methods inside an `#[napi] impl` block, extracting methods with `#[napi]`.
fn walk_napi_impl_methods(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    if !cursor.goto_first_child() {
        return;
    }
    loop {
        let node = cursor.node();
        if (node.kind() == "function_item" || node.kind() == "function_definition")
            && has_napi_attribute_before(&node, source)
            && let Some(name) = rust_item_name(&node, source)
        {
            #[allow(clippy::cast_possible_truncation)]
            let line = node.start_position().row as u32 + 1;
            endpoints.push(BridgeEndpoint {
                binding_key: name.clone(),
                kind: BridgeKind::Napi,
                role: EndpointRole::Export,
                language: "rust".into(),
                file_path: file_path.into(),
                line,
                symbol_name: name,
                confidence: ConfidenceLevel::CaseTransformed.score(),
            });
        }

        // Recurse into declaration_list (the body of impl)
        if node.kind() == "declaration_list" {
            walk_napi_impl_methods(cursor, source, file_path, endpoints);
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
    cursor.goto_parent();
}

/// Check whether preceding siblings contain a `#[napi]` attribute.
fn has_napi_attribute_before(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() == "attribute_item" {
            let text = node_text(&sibling, source);
            if text.contains("napi") {
                return true;
            }
        } else if sibling.kind() != "attribute_item"
            && sibling.kind() != "line_comment"
            && sibling.kind() != "block_comment"
        {
            break;
        }
        prev = sibling.prev_sibling();
    }
    false
}

// ---------------------------------------------------------------------------
// JS/TS import extraction
// ---------------------------------------------------------------------------

/// Find import statements that reference native/napi modules and produce Import endpoints.
fn extract_js_napi_imports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_js_napi_imports(&mut cursor, source, file_path, language, &mut endpoints);
    endpoints
}

/// Native module path indicators for napi-rs packages.
const NAPI_MODULE_INDICATORS: &[&str] = &[".node", "/native", "native", "/index.node", "@napi-rs/"];

/// Recursively walk looking for import/require statements referencing native modules.
fn walk_js_napi_imports(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();

        match node.kind() {
            // ES module imports: import { foo } from './native'
            "import_statement" => {
                if let Some(module_path) = import_module_path(&node, source)
                    && is_napi_module_path(&module_path)
                {
                    collect_import_names(&node, source, file_path, language, endpoints);
                }
            }
            // CommonJS: const { foo } = require('./native')
            "lexical_declaration" | "variable_declaration" => {
                if let Some((names, module_path)) = try_extract_require_destructure(&node, source)
                    && is_napi_module_path(&module_path)
                {
                    for name in names {
                        #[allow(clippy::cast_possible_truncation)]
                        let line = node.start_position().row as u32 + 1;
                        endpoints.push(BridgeEndpoint {
                            binding_key: name.clone(),
                            kind: BridgeKind::Napi,
                            role: EndpointRole::Import,
                            language: language.into(),
                            file_path: file_path.into(),
                            line,
                            symbol_name: name,
                            confidence: ConfidenceLevel::CaseTransformed.score(),
                        });
                    }
                }
            }
            _ => {}
        }

        if cursor.goto_first_child() {
            walk_js_napi_imports(cursor, source, file_path, language, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Check if a module path looks like a napi-rs native module.
fn is_napi_module_path(path: &str) -> bool {
    NAPI_MODULE_INDICATORS
        .iter()
        .any(|indicator| path.contains(indicator))
}

/// Extract the module specifier string from an import statement.
fn import_module_path(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "string" || child.kind() == "string_literal" {
            return Some(strip_quotes(&node_text(&child, source)));
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Collect named import specifiers from an import statement.
fn collect_import_names(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    let mut cursor = node.walk();
    collect_import_names_recursive(&mut cursor, source, file_path, language, endpoints);
}

/// Recursively find import specifier names.
fn collect_import_names_recursive(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();
        if node.kind() == "import_specifier" {
            // The imported name is the first identifier child
            if let Some(name) = import_specifier_name(&node, source) {
                #[allow(clippy::cast_possible_truncation)]
                let line = node.start_position().row as u32 + 1;
                endpoints.push(BridgeEndpoint {
                    binding_key: name.clone(),
                    kind: BridgeKind::Napi,
                    role: EndpointRole::Import,
                    language: language.into(),
                    file_path: file_path.into(),
                    line,
                    symbol_name: name,
                    confidence: ConfidenceLevel::CaseTransformed.score(),
                });
            }
        }

        if cursor.goto_first_child() {
            collect_import_names_recursive(cursor, source, file_path, language, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Extract the name from an import specifier node.
///
/// Handles `{ foo }` and `{ foo as bar }` — returns `foo` (the original name).
fn import_specifier_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "identifier" {
            return Some(node_text(&child, source));
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Try to extract destructured names and module path from a `require()` call.
///
/// Handles: `const { a, b } = require('./native')`
fn try_extract_require_destructure(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
) -> Option<(Vec<String>, String)> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }

    loop {
        let child = cursor.node();
        if child.kind() == "variable_declarator" {
            return try_extract_from_declarator(&child, source);
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Extract names and module path from a single variable declarator.
fn try_extract_from_declarator(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
) -> Option<(Vec<String>, String)> {
    let pattern = node.child_by_field_name("name")?;
    let value = node.child_by_field_name("value")?;

    // Pattern must be object_pattern: { a, b }
    if pattern.kind() != "object_pattern" {
        return None;
    }

    // Value must be call_expression: require('...')
    if value.kind() != "call_expression" {
        return None;
    }

    let func = value.child_by_field_name("function")?;
    if node_text(&func, source) != "require" {
        return None;
    }

    let args = value.child_by_field_name("arguments")?;
    let module_path = first_string_arg(&args, source)?;

    let mut names = Vec::new();
    let mut cursor = pattern.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "shorthand_property_identifier_pattern"
                || child.kind() == "shorthand_property_identifier"
            {
                names.push(node_text(&child, source));
            } else if child.kind() == "pair_pattern" {
                // { original: alias } — use the original name
                if let Some(key) = child.child_by_field_name("key") {
                    names.push(node_text(&key, source));
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    if names.is_empty() {
        None
    } else {
        Some((names, module_path))
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Extract UTF-8 text from a tree-sitter node.
fn node_text(node: &tree_sitter::Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

/// Extract the name identifier from a function, struct, or enum item.
fn rust_item_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "identifier"
            && (cursor.field_name() == Some("name") || cursor.field_name() == Some("type"))
        {
            return Some(node_text(&child, source));
        }
        // For type_identifier (struct/enum names)
        if child.kind() == "type_identifier" && cursor.field_name() == Some("name") {
            return Some(node_text(&child, source));
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Strip surrounding quotes from a string literal.
fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('\'') && s.ends_with('\''))
        || (s.starts_with('`') && s.ends_with('`'))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Extract the first string literal argument from an arguments node.
fn first_string_arg(args_node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = args_node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "string" || child.kind() == "string_literal" {
            return Some(strip_quotes(&node_text(&child, source)));
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    #[cfg(feature = "lang-javascript")]
    fn parse_js(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    #[cfg(feature = "lang-typescript")]
    fn parse_ts(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    // -- Rust exports --

    #[test]
    fn rust_napi_function_export() {
        let source = r"
#[napi]
fn add(a: i32, b: i32) -> i32 {
    a + b
}
";
        let tree = parse_rust(source);
        let extractor = NapiExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "add");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
        assert_eq!(endpoints[0].kind, BridgeKind::Napi);
        assert_eq!(endpoints[0].language, "rust");
    }

    #[test]
    fn rust_napi_struct_export() {
        let source = r"
#[napi(constructor)]
pub struct Animal {
    pub name: String,
    pub kind: u32,
}
";
        let tree = parse_rust(source);
        let extractor = NapiExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "Animal");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    #[test]
    fn rust_napi_multiple_exports() {
        let source = r#"
#[napi]
fn get_name() -> String {
    "hello".into()
}

fn internal_helper() {}

#[napi]
fn set_name(name: String) {
    // ...
}

#[napi]
pub enum Status {
    Active,
    Inactive,
}
"#;
        let tree = parse_rust(source);
        let extractor = NapiExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 3);
        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"get_name"));
        assert!(names.contains(&"set_name"));
        assert!(names.contains(&"Status"));
    }

    #[test]
    fn rust_napi_no_attribute() {
        let source = r#"
fn regular_function() -> String { "hello".into() }

#[derive(Debug)]
struct Foo;
"#;
        let tree = parse_rust(source);
        let extractor = NapiExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert!(endpoints.is_empty());
    }

    // -- JS/TS imports --

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_napi_es_import() {
        let source = r"
import { add, Animal } from './native';

const result = add(1, 2);
const animal = new Animal();
";
        let tree = parse_js(source);
        let extractor = NapiExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/index.js", "javascript");

        assert_eq!(endpoints.len(), 2);
        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"Animal"));
        for ep in &endpoints {
            assert_eq!(ep.role, EndpointRole::Import);
            assert_eq!(ep.kind, BridgeKind::Napi);
        }
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_napi_require_import() {
        let source = r"
const { getName, setName } = require('./index.node');

getName();
";
        let tree = parse_js(source);
        let extractor = NapiExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/index.js", "javascript");

        assert_eq!(endpoints.len(), 2);
        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"getName"));
        assert!(names.contains(&"setName"));
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_napi_non_native_import_ignored() {
        let source = r"
import { useState } from 'react';
import { foo } from './utils';
";
        let tree = parse_js(source);
        let extractor = NapiExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/App.js", "javascript");

        assert!(endpoints.is_empty());
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn ts_napi_import() {
        let source = r"
import { add, Animal } from '@napi-rs/my-package';

const result: number = add(1, 2);
";
        let tree = parse_ts(source);
        let extractor = NapiExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/index.ts", "typescript");

        assert_eq!(endpoints.len(), 2);
    }

    // -- Integration --

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn napi_full_link() {
        use super::super::linker::{BridgeLinker, SourceFile};

        let rust_source = r"
#[napi]
fn add(a: i32, b: i32) -> i32 { a + b }
";
        let js_source = r"
import { add } from './native';
const result = add(1, 2);
";
        let rust_tree = parse_rust(rust_source);
        let js_tree = parse_js(js_source);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(NapiExtractor));

        let files = [
            SourceFile {
                file_path: "src/lib.rs",
                language: "rust",
                tree: &rust_tree,
                source: rust_source.as_bytes(),
            },
            SourceFile {
                file_path: "src/index.js",
                language: "javascript",
                tree: &js_tree,
                source: js_source.as_bytes(),
            },
        ];

        let links = linker.extract_and_link(&files);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, BridgeKind::Napi);
        assert_eq!(links[0].export.binding_key, "add");
        assert_eq!(links[0].export.language, "rust");
        assert_eq!(links[0].import.language, "javascript");
    }
}
