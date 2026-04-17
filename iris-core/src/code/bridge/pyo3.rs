//! `PyO3` bridge extractor for `#[pyfunction]`, `#[pyclass]`, and `#[pymethods]` bindings.
//!
//! Detects cross-language bridges in `PyO3` projects:
//!
//! - **Rust exports** — functions annotated with `#[pyfunction]`, classes with
//!   `#[pyclass]`, and method blocks with `#[pymethods]`
//! - **Python imports** — `from module import name` and `import module` patterns
//!
//! Implements [`BridgeExtractor`] and can be registered with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

// ---------------------------------------------------------------------------
// PyO3Extractor
// ---------------------------------------------------------------------------

/// Extracts `PyO3` bindings from Rust and Python source files.
///
/// **Rust exports** — items annotated with `PyO3` attributes:
/// ```rust,ignore
/// #[pyfunction]
/// fn my_func() -> PyResult<()> { Ok(()) }
///
/// #[pyclass]
/// struct MyClass { value: i32 }
///
/// #[pymethods]
/// impl MyClass {
///     fn get_value(&self) -> i32 { self.value }
/// }
/// ```
///
/// **Python imports** — standard import statements:
/// ```python,ignore
/// from mymodule import my_func, MyClass
/// import mymodule
/// ```
///
/// The binding key is the symbol name. The linker handles case normalization.
pub struct PyO3Extractor;

impl BridgeExtractor for PyO3Extractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::PyO3
    }

    fn applicable_languages(&self) -> &[&str] {
        &["rust", "python"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        match language {
            "rust" => extract_rust_pyo3_exports(tree, source, file_path),
            "python" => extract_python_imports(tree, source, file_path),
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rust export extraction
// ---------------------------------------------------------------------------

/// `PyO3` attribute markers that indicate an export.
const PYO3_FUNCTION_ATTRS: &[&str] = &["pyfunction"];
const PYO3_CLASS_ATTRS: &[&str] = &["pyclass"];
const PYO3_METHODS_ATTRS: &[&str] = &["pymethods"];

/// Find PyO3-annotated items and produce Export endpoints.
fn extract_rust_pyo3_exports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_rust_pyo3_items(&mut cursor, source, file_path, &mut endpoints);
    endpoints
}

/// Recursively walk the tree looking for items with `PyO3` attributes.
fn walk_rust_pyo3_items(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();
        let kind = node.kind();

        match kind {
            "function_item" | "function_definition" => {
                if has_attribute_before(&node, source, PYO3_FUNCTION_ATTRS)
                    && let Some(name) = rust_item_name(&node, source)
                {
                    #[allow(clippy::cast_possible_truncation)]
                    let line = node.start_position().row as u32 + 1;
                    endpoints.push(BridgeEndpoint {
                        binding_key: name.clone(),
                        kind: BridgeKind::PyO3,
                        role: EndpointRole::Export,
                        language: "rust".into(),
                        file_path: file_path.into(),
                        line,
                        symbol_name: name,
                        confidence: ConfidenceLevel::CaseTransformed.score(),
                    });
                }
            }
            "struct_item" | "enum_item" => {
                if has_attribute_before(&node, source, PYO3_CLASS_ATTRS)
                    && let Some(name) = rust_item_name(&node, source)
                {
                    #[allow(clippy::cast_possible_truncation)]
                    let line = node.start_position().row as u32 + 1;
                    endpoints.push(BridgeEndpoint {
                        binding_key: name.clone(),
                        kind: BridgeKind::PyO3,
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
                // #[pymethods] impl blocks — extract method names
                if has_attribute_before(&node, source, PYO3_METHODS_ATTRS) {
                    walk_pymethods_impl(cursor, source, file_path, endpoints);
                }
            }
            _ => {}
        }

        if cursor.goto_first_child() {
            walk_rust_pyo3_items(cursor, source, file_path, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Walk methods inside a `#[pymethods] impl` block.
///
/// All public methods inside a `#[pymethods]` block are exported to Python,
/// so we extract every function definition found inside.
fn walk_pymethods_impl(
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
            && let Some(name) = rust_item_name(&node, source)
        {
            #[allow(clippy::cast_possible_truncation)]
            let line = node.start_position().row as u32 + 1;
            endpoints.push(BridgeEndpoint {
                binding_key: name.clone(),
                kind: BridgeKind::PyO3,
                role: EndpointRole::Export,
                language: "rust".into(),
                file_path: file_path.into(),
                line,
                symbol_name: name,
                confidence: ConfidenceLevel::CaseTransformed.score(),
            });
        }

        if node.kind() == "declaration_list" {
            walk_pymethods_impl(cursor, source, file_path, endpoints);
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
    cursor.goto_parent();
}

/// Check whether preceding siblings contain any of the specified attributes.
fn has_attribute_before(node: &tree_sitter::Node<'_>, source: &[u8], attr_names: &[&str]) -> bool {
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() == "attribute_item" {
            let text = node_text(&sibling, source);
            for attr in attr_names {
                if text.contains(attr) {
                    return true;
                }
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
// Python import extraction
// ---------------------------------------------------------------------------

/// Find `from module import name` statements and produce Import endpoints.
fn extract_python_imports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_python_imports(&mut cursor, source, file_path, &mut endpoints);
    endpoints
}

/// Recursively walk looking for Python import statements from internal `PyO3` modules.
///
/// Captures `from _module import name1, name2` patterns where the module name
/// starts with `_` (`PyO3` convention for native extension modules, e.g.
/// `_pydantic_core`). Imports from external packages are skipped to avoid
/// false positive bridge matches.
fn walk_python_imports(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();

        if node.kind() == "import_from_statement" {
            // Only create endpoints for imports from internal `PyO3` modules.
            // Convention: native modules have a `_` prefix (e.g. `_pydantic_core`).
            if is_internal_pyo3_import(&node, source) {
                collect_python_import_names(&node, source, file_path, endpoints);
            }
        }

        if cursor.goto_first_child() {
            walk_python_imports(cursor, source, file_path, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Check if an `import_from_statement` imports from an internal `PyO3` module.
///
/// Returns `true` when the module name (the `from X` part) starts with `_`,
/// which is the `PyO3` convention for native extension modules (e.g.
/// `from _pydantic_core import SchemaValidator`). Also returns `true` for
/// relative imports (`from . import X`) since those are project-internal.
fn is_internal_pyo3_import(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    // tree-sitter-python uses "module_name" field for the source module.
    if let Some(module_node) = node.child_by_field_name("module_name") {
        let module_text = node_text(&module_node, source);
        // The final segment of a dotted path determines the native module.
        // e.g. `from pydantic_core._pydantic_core import X` → `_pydantic_core`
        let final_segment = module_text.rsplit('.').next().unwrap_or(&module_text);
        return final_segment.starts_with('_');
    }
    // Relative imports without a module name (`from . import X`) are project-internal.
    true
}

/// Collect imported names from a `from module import name1, name2` statement.
fn collect_python_import_names(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return;
    }

    loop {
        let child = cursor.node();
        // In tree-sitter-python, imported names appear as:
        // - `dotted_name` (the module path after `from`)
        // - identifiers within the import list
        if child.kind() == "dotted_name" && cursor.field_name() == Some("name") {
            // This is an imported name (not the module)
            let name = node_text(&child, source);
            #[allow(clippy::cast_possible_truncation)]
            let line = child.start_position().row as u32 + 1;
            endpoints.push(BridgeEndpoint {
                binding_key: name.clone(),
                kind: BridgeKind::PyO3,
                role: EndpointRole::Import,
                language: "python".into(),
                file_path: file_path.into(),
                line,
                symbol_name: name,
                confidence: ConfidenceLevel::Fuzzy.score(),
            });
        }

        if !cursor.goto_next_sibling() {
            break;
        }
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
        if child.kind() == "type_identifier" && cursor.field_name() == Some("name") {
            return Some(node_text(&child, source));
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

    #[cfg(feature = "lang-python")]
    fn parse_python(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    // -- Rust exports --

    #[test]
    fn rust_pyfunction_export() {
        let source = r"
#[pyfunction]
fn my_func() -> PyResult<()> {
    Ok(())
}
";
        let tree = parse_rust(source);
        let extractor = PyO3Extractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "my_func");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
        assert_eq!(endpoints[0].kind, BridgeKind::PyO3);
        assert_eq!(endpoints[0].language, "rust");
    }

    #[test]
    fn rust_pyclass_export() {
        let source = r"
#[pyclass]
struct MyClass {
    value: i32,
}
";
        let tree = parse_rust(source);
        let extractor = PyO3Extractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "MyClass");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    #[test]
    fn rust_pymethods_export() {
        let source = r"
#[pyclass]
struct Calculator {
    result: f64,
}

#[pymethods]
impl Calculator {
    fn add(&mut self, value: f64) {
        self.result += value;
    }

    fn get_result(&self) -> f64 {
        self.result
    }
}
";
        let tree = parse_rust(source);
        let extractor = PyO3Extractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        // Should find: Calculator (pyclass), add, get_result (pymethods)
        assert_eq!(endpoints.len(), 3);
        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"Calculator"));
        assert!(names.contains(&"add"));
        assert!(names.contains(&"get_result"));
    }

    #[test]
    fn rust_pyo3_multiple_exports() {
        let source = r#"
#[pyfunction]
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

fn internal_helper() {}

#[pyfunction]
fn add(a: i64, b: i64) -> i64 {
    a + b
}

#[pyclass]
struct Config {
    debug: bool,
}
"#;
        let tree = parse_rust(source);
        let extractor = PyO3Extractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 3);
        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"add"));
        assert!(names.contains(&"Config"));
    }

    #[test]
    fn rust_pyo3_no_attribute() {
        let source = r#"
fn regular_function() -> String { "hello".into() }

#[derive(Debug)]
struct Foo;
"#;
        let tree = parse_rust(source);
        let extractor = PyO3Extractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert!(endpoints.is_empty());
    }

    #[test]
    fn rust_pyfunction_with_pyo3_attr() {
        // Some users use #[pyo3(name = "custom_name")] alongside #[pyfunction]
        let source = r#"
#[pyfunction]
#[pyo3(name = "custom_name")]
fn original_name() -> i32 {
    42
}
"#;
        let tree = parse_rust(source);
        let extractor = PyO3Extractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "original_name");
    }

    // -- Python imports --

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_from_import() {
        let source = r"
from _mymodule import my_func, MyClass
";
        let tree = parse_python(source);
        let extractor = PyO3Extractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "main.py", "python");

        assert_eq!(endpoints.len(), 2);
        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"my_func"));
        assert!(names.contains(&"MyClass"));
        for ep in &endpoints {
            assert_eq!(ep.role, EndpointRole::Import);
            assert_eq!(ep.kind, BridgeKind::PyO3);
            assert_eq!(ep.language, "python");
        }
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_from_import_multiple() {
        let source = r"
from _calculator import add, subtract, Calculator
from _utils import greet
";
        let tree = parse_python(source);
        let extractor = PyO3Extractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "main.py", "python");

        assert_eq!(endpoints.len(), 4);
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_no_from_import() {
        // Plain `import module` doesn't give us specific symbol names
        let source = r"
import os
import sys
";
        let tree = parse_python(source);
        let extractor = PyO3Extractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "main.py", "python");

        assert!(endpoints.is_empty());
    }

    // -- Integration --

    #[cfg(feature = "lang-python")]
    #[test]
    fn pyo3_full_link() {
        use super::super::linker::{BridgeLinker, SourceFile};

        let rust_source = r#"
#[pyfunction]
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}
"#;
        let python_source = r#"
from _mymodule import greet

result = greet("World")
"#;
        let rust_tree = parse_rust(rust_source);
        let python_tree = parse_python(python_source);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(PyO3Extractor));

        let files = [
            SourceFile {
                file_path: "src/lib.rs",
                language: "rust",
                tree: &rust_tree,
                source: rust_source.as_bytes(),
            },
            SourceFile {
                file_path: "main.py",
                language: "python",
                tree: &python_tree,
                source: python_source.as_bytes(),
            },
        ];

        let links = linker.extract_and_link(&files);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, BridgeKind::PyO3);
        assert_eq!(links[0].export.binding_key, "greet");
        assert_eq!(links[0].export.language, "rust");
        assert_eq!(links[0].import.language, "python");
    }
}
