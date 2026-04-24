//! Wasm-bindgen bridge extractor for `#[wasm_bindgen]` bindings.
//!
//! Detects cross-language bridges in wasm-bindgen projects:
//!
//! - **Rust exports** — functions and structs annotated with `#[wasm_bindgen]`
//! - **JS imports** — `import { name } from './pkg/module'` patterns
//!
//! Implements [`BridgeExtractor`] and can be registered with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

// ---------------------------------------------------------------------------
// WasmBindgenExtractor
// ---------------------------------------------------------------------------

/// Extracts wasm-bindgen bindings from Rust and JS/TS source files.
///
/// **Rust exports** — items annotated with `#[wasm_bindgen]`:
/// ```rust,ignore
/// #[wasm_bindgen]
/// pub fn greet(name: &str) -> String { format!("Hello, {name}!") }
///
/// #[wasm_bindgen]
/// pub struct Counter { count: u32 }
/// ```
///
/// **JS imports** — named imports from wasm package paths:
/// ```javascript,ignore
/// import init, { greet, Counter } from './pkg/mymodule';
/// import { greet } from './pkg/mymodule_bg.wasm';
/// ```
///
/// The binding key is the symbol name. `js_name` overrides are not yet
/// tracked (a future enhancement).
pub struct WasmBindgenExtractor;

impl BridgeExtractor for WasmBindgenExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::WasmBindgen
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
            "rust" => extract_rust_wasm_exports(tree, source, file_path),
            "javascript" | "typescript" | "tsx" => {
                extract_js_wasm_imports(tree, source, file_path, language)
            }
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rust export extraction
// ---------------------------------------------------------------------------

/// Find `#[wasm_bindgen]` annotated items and produce Export endpoints.
fn extract_rust_wasm_exports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_rust_wasm_items(&mut cursor, source, file_path, &mut endpoints);
    endpoints
}

/// Recursively walk the tree looking for items with `#[wasm_bindgen]` attribute.
fn walk_rust_wasm_items(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();
        let kind = node.kind();

        match kind {
            "function_item" | "function_definition" | "struct_item" => {
                if has_wasm_bindgen_attribute_before(&node, source)
                    && let Some(name) = rust_item_name(&node, source)
                {
                    #[allow(clippy::cast_possible_truncation)]
                    let line = node.start_position().row as u32 + 1;
                    endpoints.push(BridgeEndpoint {
                        binding_key: name.clone(),
                        kind: BridgeKind::WasmBindgen,
                        role: EndpointRole::Export,
                        language: "rust".into(),
                        file_path: file_path.into(),
                        line,
                        symbol_name: name,
                        confidence: ConfidenceLevel::CaseTransformed.score(),
                    });
                }
            }
            // Walk into #[wasm_bindgen] impl blocks and extract their methods.
            "impl_item" if has_wasm_bindgen_attribute_before(&node, source) => {
                walk_wasm_impl_methods(cursor, source, file_path, endpoints);
            }
            _ => {}
        }

        if cursor.goto_first_child() {
            walk_rust_wasm_items(cursor, source, file_path, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Walk methods inside a `#[wasm_bindgen] impl` block.
fn walk_wasm_impl_methods(
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
            && has_wasm_bindgen_attribute_before(&node, source)
            && let Some(name) = rust_item_name(&node, source)
        {
            #[allow(clippy::cast_possible_truncation)]
            let line = node.start_position().row as u32 + 1;
            endpoints.push(BridgeEndpoint {
                binding_key: name.clone(),
                kind: BridgeKind::WasmBindgen,
                role: EndpointRole::Export,
                language: "rust".into(),
                file_path: file_path.into(),
                line,
                symbol_name: name,
                confidence: ConfidenceLevel::CaseTransformed.score(),
            });
        }

        if node.kind() == "declaration_list" {
            walk_wasm_impl_methods(cursor, source, file_path, endpoints);
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
    cursor.goto_parent();
}

/// Check whether preceding siblings contain a `#[wasm_bindgen]` attribute.
fn has_wasm_bindgen_attribute_before(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() == "attribute_item" {
            let text = node_text(&sibling, source);
            if text.contains("wasm_bindgen") {
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

/// Find import statements referencing wasm/pkg modules and produce Import endpoints.
fn extract_js_wasm_imports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_js_wasm_imports(&mut cursor, source, file_path, language, &mut endpoints);
    endpoints
}

/// Path indicators for wasm-bindgen packages.
const WASM_MODULE_INDICATORS: &[&str] = &["/pkg/", "_bg.wasm", "_bg.js", "wasm-bindgen"];

/// Recursively walk looking for import statements referencing wasm modules.
fn walk_js_wasm_imports(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();

        if node.kind() == "import_statement"
            && let Some(module_path) = import_module_path(&node, source)
            && is_wasm_module_path(&module_path)
        {
            collect_import_names(&node, source, file_path, language, endpoints);
        }

        if cursor.goto_first_child() {
            walk_js_wasm_imports(cursor, source, file_path, language, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Check if a module path looks like a wasm-bindgen package.
fn is_wasm_module_path(path: &str) -> bool {
    WASM_MODULE_INDICATORS
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
        if node.kind() == "import_specifier"
            && let Some(name) = import_specifier_name(&node, source)
        {
            #[allow(clippy::cast_possible_truncation)]
            let line = node.start_position().row as u32 + 1;
            endpoints.push(BridgeEndpoint {
                binding_key: name.clone(),
                kind: BridgeKind::WasmBindgen,
                role: EndpointRole::Import,
                language: language.into(),
                file_path: file_path.into(),
                line,
                symbol_name: name,
                confidence: ConfidenceLevel::CaseTransformed.score(),
            });
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

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Extract UTF-8 text from a tree-sitter node.
fn node_text(node: &tree_sitter::Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

/// Extract the name identifier from a function or struct item.
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
    fn rust_wasm_function_export() {
        let source = r#"
#[wasm_bindgen]
pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}
"#;
        let tree = parse_rust(source);
        let extractor = WasmBindgenExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "greet");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
        assert_eq!(endpoints[0].kind, BridgeKind::WasmBindgen);
        assert_eq!(endpoints[0].language, "rust");
    }

    #[test]
    fn rust_wasm_struct_export() {
        let source = r"
#[wasm_bindgen]
pub struct Counter {
    count: u32,
}
";
        let tree = parse_rust(source);
        let extractor = WasmBindgenExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "Counter");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    #[test]
    fn rust_wasm_multiple_exports() {
        let source = r"
#[wasm_bindgen]
pub fn add(a: u32, b: u32) -> u32 { a + b }

fn internal() {}

#[wasm_bindgen]
pub fn multiply(a: u32, b: u32) -> u32 { a * b }

#[wasm_bindgen]
pub struct Point {
    x: f64,
    y: f64,
}
";
        let tree = parse_rust(source);
        let extractor = WasmBindgenExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 3);
        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"multiply"));
        assert!(names.contains(&"Point"));
    }

    #[test]
    fn rust_wasm_with_js_name() {
        // Even with js_name, we still detect the Rust symbol name
        let source = r#"
#[wasm_bindgen(js_name = "customGreet")]
pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}
"#;
        let tree = parse_rust(source);
        let extractor = WasmBindgenExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "greet");
    }

    #[test]
    fn rust_wasm_no_attribute() {
        let source = r"
pub fn regular_function() -> u32 { 42 }
";
        let tree = parse_rust(source);
        let extractor = WasmBindgenExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert!(endpoints.is_empty());
    }

    // -- JS imports --

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_wasm_pkg_import() {
        let source = r#"
import init, { greet, Counter } from './pkg/mymodule';

async function run() {
    await init();
    greet("World");
}
"#;
        let tree = parse_js(source);
        let extractor = WasmBindgenExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/index.js", "javascript");

        // Should capture greet and Counter (init is a default import, not a named specifier)
        assert_eq!(endpoints.len(), 2);
        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"Counter"));
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_wasm_bg_import() {
        let source = r"
import { add, multiply } from './pkg/mymodule_bg.wasm';
";
        let tree = parse_js(source);
        let extractor = WasmBindgenExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/index.js", "javascript");

        assert_eq!(endpoints.len(), 2);
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_wasm_non_pkg_ignored() {
        let source = r"
import { foo } from './utils';
import { bar } from 'lodash';
";
        let tree = parse_js(source);
        let extractor = WasmBindgenExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/index.js", "javascript");

        assert!(endpoints.is_empty());
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn ts_wasm_import() {
        let source = r#"
import { greet } from './pkg/mymodule';

const result: string = greet("World");
"#;
        let tree = parse_ts(source);
        let extractor = WasmBindgenExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/index.ts", "typescript");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "greet");
    }

    // -- Integration --

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn wasm_full_link() {
        use super::super::linker::{BridgeLinker, SourceFile};

        let rust_source = r#"
#[wasm_bindgen]
pub fn greet(name: &str) -> String { format!("Hello, {name}!") }
"#;
        let js_source = r#"
import { greet } from './pkg/mymodule';
greet("World");
"#;
        let rust_tree = parse_rust(rust_source);
        let js_tree = parse_js(js_source);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(WasmBindgenExtractor));

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
        assert_eq!(links[0].kind, BridgeKind::WasmBindgen);
        assert_eq!(links[0].export.binding_key, "greet");
        assert_eq!(links[0].export.language, "rust");
        assert_eq!(links[0].import.language, "javascript");
    }
}
