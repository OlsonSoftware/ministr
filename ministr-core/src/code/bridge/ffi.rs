//! C-ABI Foreign Function Interface bridge extractor.
//!
//! Detects native interop spanning Rust ↔ C/C++ ↔ Python ↔ Java:
//!
//! - **Rust** — `pub extern "C" fn` exports gated by `#[no_mangle]` or
//!   `#[export_name = "…"]` (so the linker symbol matches the source-level
//!   identifier), plus `extern "C" { fn … }` foreign-block declarations
//!   (the import/consumer side of native symbols).
//! - **C / C++** — function definitions in `linkage_specification` blocks
//!   (`extern "C" { … }`), plus any function whose name matches the JNI
//!   `Java_<pkg>_<class>_<method>` mangling convention.
//! - **Python** — `ctypes.CDLL(…)` / `cffi.FFI().dlopen(…)` library handles
//!   plus subsequent attribute calls on those handles (`mylib.add(1, 2)`).
//! - **Java** — `native` method declarations (the JNI consumer side).
//!
//! ## Binding-key conventions
//!
//! - **C-ABI**: the bare symbol name as it appears in the binary
//!   (e.g. `add` for `pub extern "C" fn add(...)` and for `lib.add(...)`).
//! - **JNI**: the C-side `Java_pkg_Class_method` mangling is the binding key,
//!   and the Java side reconstructs it from `package.Class.method`. The
//!   linker pairs by exact key.
//!
//! ## Confidence
//!
//! All FFI endpoints emit at [`ConfidenceLevel::Exact`]. Cross-language
//! pairing is by exact symbol name (C-ABI has no name mangling), so the
//! linker doesn't need to fall through to its `CaseTransformed` pass.

use super::util::{has_rust_attribute_before, node_line, node_text};
use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

/// Attributes that suppress Rust's name-mangling and so allow a function's
/// source-level identifier to appear unchanged in the linker symbol table.
/// The FFI linker pairs by exact symbol name, so an `extern "C" fn` without
/// one of these would not actually be discoverable by a foreign consumer
/// looking up the bare name — emitting it as an Export would lead to
/// false-positive cross-language pairings.
const RUST_FFI_EXPORT_ATTRS: &[&str] = &["no_mangle", "export_name", "unsafe(no_mangle)"];

/// FFI bridge extractor — see module docs for supported patterns.
pub struct FfiExtractor;

impl BridgeExtractor for FfiExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::Ffi
    }

    fn applicable_languages(&self) -> &[&str] {
        &["rust", "c", "cpp", "python", "java"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        match language {
            "rust" => extract_rust_ffi(tree, source, file_path),
            "c" | "cpp" => extract_c_cpp_ffi(tree, source, file_path, language),
            "python" => extract_python_ffi(tree, source, file_path),
            "java" => extract_java_ffi(tree, source, file_path),
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rust
// ---------------------------------------------------------------------------

/// Walk a Rust tree for `extern "C" fn` exports and `extern "C" { ... }` imports.
fn extract_rust_ffi(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_rust_ffi(&mut cursor, source, file_path, &mut endpoints);
    endpoints
}

fn walk_rust_ffi(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();
        match node.kind() {
            "function_item" => {
                // An `extern "C" fn` is only an FFI export when the
                // compiler is told NOT to mangle its name — otherwise a
                // C/C++/Python consumer looking up the bare identifier
                // won't find it, and we'd emit a false cross-language
                // pair. Require `#[no_mangle]` (or `#[export_name = …]`).
                if rust_is_extern_c_fn(&node, source)
                    && has_rust_attribute_before(&node, source, RUST_FFI_EXPORT_ATTRS)
                    && let Some(name) = rust_fn_name(&node, source)
                {
                    endpoints.push(make_endpoint(
                        name,
                        EndpointRole::Export,
                        "rust",
                        file_path,
                        node_line(&node),
                    ));
                }
            }
            "foreign_mod_item" => {
                rust_collect_foreign_imports(&node, source, file_path, endpoints);
            }
            _ => {}
        }
        if cursor.goto_first_child() {
            walk_rust_ffi(cursor, source, file_path, endpoints);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Whether a Rust `function_item` carries `extern "C"` (or unspecified `extern`,
/// which defaults to "C") in its `function_modifiers`.
fn rust_is_extern_c_fn(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    let Some(modifiers) = node.child_by_field_name("modifiers") else {
        // tree-sitter-rust does not always expose `modifiers` as a field; fall
        // through to a textual check on the function's leading slice.
        let text = node_text(node, source);
        return text.contains("extern \"C\"") || text.contains("extern\"C\"");
    };
    let mod_text = node_text(&modifiers, source);
    mod_text.contains("extern \"C\"") || mod_text.contains("extern\"C\"") || {
        // Bare `extern fn` without an ABI string defaults to C ABI.
        mod_text.split_whitespace().any(|t| t == "extern")
    }
}

/// Extract the name from a Rust `function_item` (field `name` → `identifier`).
fn rust_fn_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let name = node.child_by_field_name("name")?;
    Some(node_text(&name, source))
}

/// Walk an `extern "C" { ... }` block and emit Import endpoints for each
/// `function_signature_item` (forward declaration of a foreign symbol).
fn rust_collect_foreign_imports(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            let mut inner = child.walk();
            for decl in child.children(&mut inner) {
                if decl.kind() == "function_signature_item"
                    && let Some(name) = rust_fn_name(&decl, source)
                {
                    endpoints.push(make_endpoint(
                        name,
                        EndpointRole::Import,
                        "rust",
                        file_path,
                        node_line(&decl),
                    ));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// C / C++
// ---------------------------------------------------------------------------

/// Extract C-ABI exports from a C or C++ tree.
///
/// For `.c`: every top-level `function_definition` is potentially exported.
/// For `.cpp`: only functions inside `linkage_specification` (`extern "C" {}`)
/// are C-ABI exports — C++ uses name-mangled symbols by default. JNI
/// `Java_*` exports are detected by name pattern in both.
fn extract_c_cpp_ffi(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_c_cpp_ffi(
        &mut cursor,
        source,
        file_path,
        language,
        false,
        &mut endpoints,
    );
    endpoints
}

fn walk_c_cpp_ffi(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
    inside_extern_c: bool,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();
        match node.kind() {
            "linkage_specification" => {
                if c_is_extern_c_linkage(&node, source) && cursor.goto_first_child() {
                    walk_c_cpp_ffi(cursor, source, file_path, language, true, endpoints);
                    cursor.goto_parent();
                }
            }
            "function_definition" => {
                if let Some(name) = c_fn_name(&node, source) {
                    let is_jni = is_jni_name(&name);
                    let visible = language == "c" || inside_extern_c || is_jni;
                    if visible {
                        endpoints.push(make_endpoint(
                            name,
                            EndpointRole::Export,
                            language,
                            file_path,
                            node_line(&node),
                        ));
                    }
                }
            }
            _ => {}
        }
        if cursor.goto_first_child() {
            walk_c_cpp_ffi(
                cursor,
                source,
                file_path,
                language,
                inside_extern_c,
                endpoints,
            );
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn c_is_extern_c_linkage(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    // tree-sitter-cpp: linkage_specification has children { extern, "C", body }.
    // Match by source text of the first ~40 chars to avoid grammar churn.
    let text = node_text(node, source);
    let prefix = text.chars().take(40).collect::<String>();
    prefix.contains("\"C\"")
}

/// Walk the declarator chain to extract a C/C++ function's bare name.
///
/// For `void Foo::bar() {}` returns `Foo::bar` (the qualified identifier
/// is what tree-sitter-cpp's utf8_text returns at that node — same
/// shape as the C/C++ symbol extractor produces).
fn c_fn_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut current = node.child_by_field_name("declarator")?;
    while current.kind() != "function_declarator" {
        current = current.child_by_field_name("declarator")?;
    }
    let inner = current.child_by_field_name("declarator")?;
    let text = inner.utf8_text(source).ok()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Whether a C function name matches the JNI mangling convention
/// `Java_<package>_<class>_<method>`. Case-sensitive: spec requires the
/// `Java_` prefix be exact.
fn is_jni_name(name: &str) -> bool {
    name.starts_with("Java_") && name.len() > "Java_".len()
}

// ---------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------

/// Extract Python ctypes/cffi import-side endpoints.
///
/// Two-pass: first walk module-level assignments to identify variable names
/// bound to `ctypes.CDLL(...)` or `cffi.FFI().dlopen(...)` results, then
/// scan call expressions of the form `<bound_name>.<method>(...)`.
fn extract_python_ffi(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let root = tree.root_node();
    let handles = python_collect_ffi_handles(&root, source);
    if handles.is_empty() {
        return Vec::new();
    }
    let mut endpoints = Vec::new();
    let mut cursor = root.walk();
    walk_python_calls(&mut cursor, source, file_path, &handles, &mut endpoints);
    endpoints
}

/// Find names bound to ctypes.CDLL(...) / cffi.FFI().dlopen(...) at module
/// scope. Returns the set of variable names that are FFI library handles.
fn python_collect_ffi_handles(root: &tree_sitter::Node<'_>, source: &[u8]) -> Vec<String> {
    let mut handles = Vec::new();
    let mut cursor = root.walk();
    walk_python_assignments(&mut cursor, source, &mut handles);
    handles
}

fn walk_python_assignments(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    handles: &mut Vec<String>,
) {
    loop {
        let node = cursor.node();
        if node.kind() == "assignment"
            && let Some(name) = python_ffi_handle_from_assignment(&node, source)
        {
            handles.push(name);
        }
        if cursor.goto_first_child() {
            walk_python_assignments(cursor, source, handles);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn python_ffi_handle_from_assignment(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
) -> Option<String> {
    let left = node.child_by_field_name("left")?;
    let right = node.child_by_field_name("right")?;
    if left.kind() != "identifier" {
        return None;
    }
    if !python_call_is_ffi_handle(&right, source) {
        return None;
    }
    Some(node_text(&left, source))
}

/// Whether a Python call expression returns an FFI library handle —
/// matches `ctypes.CDLL(...)`, `ctypes.WinDLL(...)`, `ctypes.cdll.LoadLibrary(...)`,
/// `cffi.FFI().dlopen(...)`.
///
/// Suffix-match on the call's function expression text. The
/// `case_sensitive_file_extension_comparisons` clippy lint fires here because
/// the suffixes look like file extensions; it's a false positive — these are
/// case-sensitive Python identifiers (`CDLL`, `WinDLL`, etc.) and a
/// case-insensitive match would mis-classify lowercase identifier collisions.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn python_call_is_ffi_handle(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    if node.kind() != "call" {
        return false;
    }
    let Some(func) = node.child_by_field_name("function") else {
        return false;
    };
    let text = node_text(&func, source);
    text.ends_with(".CDLL")
        || text.ends_with(".WinDLL")
        || text.ends_with(".OleDLL")
        || text.ends_with(".PyDLL")
        || text.ends_with(".LoadLibrary")
        || text.ends_with(".dlopen")
}

fn walk_python_calls(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    handles: &[String],
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();
        if node.kind() == "call"
            && let Some(method) = python_handle_method_call(&node, source, handles)
        {
            endpoints.push(make_endpoint(
                method,
                EndpointRole::Import,
                "python",
                file_path,
                node_line(&node),
            ));
        }
        if cursor.goto_first_child() {
            walk_python_calls(cursor, source, file_path, handles, endpoints);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// If a Python `call` is `<handle>.<method>(...)` where `<handle>` is a
/// known FFI library handle, return the `<method>` name.
fn python_handle_method_call(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    handles: &[String],
) -> Option<String> {
    let func = node.child_by_field_name("function")?;
    if func.kind() != "attribute" {
        return None;
    }
    let object = func.child_by_field_name("object")?;
    let attr = func.child_by_field_name("attribute")?;
    if object.kind() != "identifier" {
        return None;
    }
    let object_name = node_text(&object, source);
    if !handles.iter().any(|h| h == &object_name) {
        return None;
    }
    Some(node_text(&attr, source))
}

// ---------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------

/// Extract JNI `native` method declarations from a Java tree.
///
/// Reconstructs the C-side mangled name `Java_<pkg>_<class>_<method>`
/// from the file's `package` declaration and the enclosing class name.
fn extract_java_ffi(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let root = tree.root_node();
    let package = java_package_name(&root, source).unwrap_or_default();

    let mut endpoints = Vec::new();
    let mut cursor = root.walk();
    walk_java_classes(&mut cursor, source, file_path, &package, &mut endpoints);
    endpoints
}

fn java_package_name(root: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_declaration" {
            let mut inner = child.walk();
            for sub in child.children(&mut inner) {
                if sub.kind() == "scoped_identifier" || sub.kind() == "identifier" {
                    return Some(node_text(&sub, source));
                }
            }
        }
    }
    None
}

fn walk_java_classes(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    package: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();
        if node.kind() == "class_declaration"
            && let Some(class_name) = node
                .child_by_field_name("name")
                .map(|n| node_text(&n, source))
        {
            java_walk_class_body(&node, source, file_path, package, &class_name, endpoints);
        }
        if cursor.goto_first_child() {
            walk_java_classes(cursor, source, file_path, package, endpoints);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn java_walk_class_body(
    class_node: &tree_sitter::Node<'_>,
    source: &[u8],
    file_path: &str,
    package: &str,
    class_name: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    let Some(body) = class_node.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for member in body.children(&mut cursor) {
        if member.kind() == "method_declaration" && java_method_is_native(&member, source) {
            let Some(method_name) = member
                .child_by_field_name("name")
                .map(|n| node_text(&n, source))
            else {
                continue;
            };
            let key = jni_mangle(package, class_name, &method_name);
            endpoints.push(make_endpoint(
                key,
                EndpointRole::Import,
                "java",
                file_path,
                node_line(&member),
            ));
        }
    }
}

fn java_method_is_native(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(&child, source);
            return text.split_whitespace().any(|t| t == "native");
        }
    }
    false
}

/// Construct the JNI C-side mangled name for a Java native method.
///
/// Per the JNI spec: replace `.` in the package path with `_`, then
/// concatenate `Java_<package>_<class>_<method>`. Underscores in original
/// identifiers should be escaped to `_1`, but most projects don't use
/// underscores in JNI-exported names; we omit that escaping for simplicity
/// and document the limitation. If pairing fails on a `_`-containing name,
/// the linker's `CaseTransformed` pass will not rescue it — JNI names are
/// case- and underscore-sensitive.
fn jni_mangle(package: &str, class: &str, method: &str) -> String {
    let pkg = package.replace('.', "_");
    if pkg.is_empty() {
        format!("Java_{class}_{method}")
    } else {
        format!("Java_{pkg}_{class}_{method}")
    }
}

// ---------------------------------------------------------------------------
// Shared
// ---------------------------------------------------------------------------

fn make_endpoint(
    name: String,
    role: EndpointRole,
    language: &str,
    file_path: &str,
    line: u32,
) -> BridgeEndpoint {
    BridgeEndpoint {
        binding_key: name.clone(),
        kind: BridgeKind::Ffi,
        role,
        language: language.into(),
        file_path: file_path.into(),
        line,
        symbol_name: name,
        confidence: ConfidenceLevel::Exact.score(),
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

    #[cfg(feature = "lang-c")]
    fn parse_c(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    #[cfg(feature = "lang-cpp")]
    fn parse_cpp(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
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

    #[cfg(feature = "lang-java")]
    fn parse_java(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    // -- Rust --

    #[test]
    fn rust_extern_c_fn_without_no_mangle_is_not_exported() {
        // Bare `extern "C" fn` without `#[no_mangle]` is NOT an FFI export:
        // the compiler still mangles its symbol name, so a foreign consumer
        // looking up `add` would not find it in the linker symbol table.
        // Emitting it as an Export would produce false cross-language pairs.
        let source = r#"
pub extern "C" fn add(a: i32, b: i32) -> i32 { a + b }
"#;
        let tree = parse_rust(source);
        let endpoints = FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "lib.rs", "rust");
        assert!(endpoints.is_empty(), "got {endpoints:#?}");
    }

    #[test]
    fn rust_export_name_extern_c_export() {
        // `#[export_name = "..."]` also suppresses mangling and should
        // count as an FFI export.
        let source = r#"
#[export_name = "add"]
pub extern "C" fn rust_add(a: i32, b: i32) -> i32 { a + b }
"#;
        let tree = parse_rust(source);
        let endpoints = FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "lib.rs", "rust");
        assert_eq!(endpoints.len(), 1, "got {endpoints:#?}");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    #[test]
    fn rust_no_mangle_extern_c_export() {
        let source = r#"
#[no_mangle]
pub extern "C" fn add(a: i32, b: i32) -> i32 { a + b }
"#;
        let tree = parse_rust(source);
        let endpoints = FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "lib.rs", "rust");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "add");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    #[test]
    fn rust_foreign_block_imports() {
        let source = r#"
extern "C" {
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
    fn write(fd: i32, buf: *const u8, count: usize) -> isize;
}
"#;
        let tree = parse_rust(source);
        let endpoints = FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "lib.rs", "rust");
        assert_eq!(endpoints.len(), 2, "got {endpoints:#?}");
        let keys: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(keys.contains(&"read"));
        assert!(keys.contains(&"write"));
        for ep in &endpoints {
            assert_eq!(ep.role, EndpointRole::Import);
        }
    }

    // -- C / C++ --

    #[cfg(feature = "lang-c")]
    #[test]
    fn c_function_definition_export() {
        let source = "int add(int a, int b) { return a + b; }\n";
        let tree = parse_c(source);
        let endpoints = FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "lib.c", "c");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "add");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    #[cfg(feature = "lang-cpp")]
    #[test]
    fn cpp_extern_c_block_export() {
        let source = r#"
extern "C" {
    void greet(const char *name) {}
}
"#;
        let tree = parse_cpp(source);
        let endpoints = FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "lib.cpp", "cpp");
        assert_eq!(endpoints.len(), 1, "got {endpoints:#?}");
        assert_eq!(endpoints[0].binding_key, "greet");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    #[cfg(feature = "lang-cpp")]
    #[test]
    fn cpp_plain_function_not_exported() {
        // Without `extern "C"`, a C++ free function uses C++ name mangling
        // and should NOT be considered a C-ABI export.
        let source = "int add(int a, int b) { return a + b; }\n";
        let tree = parse_cpp(source);
        let endpoints = FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "lib.cpp", "cpp");
        assert!(
            endpoints.is_empty(),
            "plain C++ function should not be FFI-exported, got {endpoints:#?}"
        );
    }

    #[cfg(feature = "lang-c")]
    #[test]
    fn c_jni_function_export() {
        let source = "void Java_com_example_Foo_bar(void) {}\n";
        let tree = parse_c(source);
        let endpoints = FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "jni.c", "c");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "Java_com_example_Foo_bar");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    // -- Python --

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_ctypes_call_import() {
        let source = r#"
import ctypes
lib = ctypes.CDLL("./libfoo.so")
lib.add(1, 2)
"#;
        let tree = parse_python(source);
        let endpoints =
            FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "client.py", "python");
        assert_eq!(endpoints.len(), 1, "got {endpoints:#?}");
        assert_eq!(endpoints[0].binding_key, "add");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_cffi_call_import() {
        let source = r#"
from cffi import FFI
ffi = FFI()
lib = ffi.dlopen("./libfoo.so")
lib.add(1, 2)
"#;
        let tree = parse_python(source);
        let endpoints =
            FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "client.py", "python");
        assert_eq!(endpoints.len(), 1, "got {endpoints:#?}");
        assert_eq!(endpoints[0].binding_key, "add");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_no_ffi_means_no_endpoints() {
        let source = "x = 1\nprint(x)\n";
        let tree = parse_python(source);
        let endpoints =
            FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "client.py", "python");
        assert!(endpoints.is_empty());
    }

    // -- Java --

    #[cfg(feature = "lang-java")]
    #[test]
    fn java_native_method_import() {
        let source = r"
package com.example;

class Foo {
    public native int add(int a, int b);
    public int regular() { return 0; }
}
";
        let tree = parse_java(source);
        let endpoints =
            FfiExtractor.extract_endpoints(&tree, source.as_bytes(), "Foo.java", "java");
        assert_eq!(endpoints.len(), 1, "got {endpoints:#?}");
        assert_eq!(endpoints[0].binding_key, "Java_com_example_Foo_add");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
    }

    #[test]
    fn jni_mangle_basic() {
        assert_eq!(
            jni_mangle("com.example", "Foo", "bar"),
            "Java_com_example_Foo_bar"
        );
        assert_eq!(jni_mangle("", "Foo", "bar"), "Java_Foo_bar");
        assert_eq!(jni_mangle("a.b.c", "Foo", "x"), "Java_a_b_c_Foo_x");
    }

    #[test]
    fn is_jni_name_recognises_prefix() {
        assert!(is_jni_name("Java_pkg_Class_method"));
        assert!(!is_jni_name("Java_"));
        assert!(!is_jni_name("javaPkg"));
        assert!(!is_jni_name("foo"));
    }

    // -- Integration via BridgeLinker --

    #[cfg(feature = "lang-python")]
    #[test]
    fn rust_export_links_python_ctypes_import() {
        use crate::code::bridge::linker::{BridgeLinker, SourceFile};

        let rust_src = "#[no_mangle]\npub extern \"C\" fn add(a: i32, b: i32) -> i32 { a + b }\n";
        let py_src = "import ctypes\nlib = ctypes.CDLL(\"./libfoo.so\")\nlib.add(1, 2)\n";

        let rust_tree = parse_rust(rust_src);
        let py_tree = parse_python(py_src);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(FfiExtractor));

        let endpoints = linker.extract_all(&[
            SourceFile {
                file_path: "lib.rs",
                language: "rust",
                tree: &rust_tree,
                source: rust_src.as_bytes(),
            },
            SourceFile {
                file_path: "client.py",
                language: "python",
                tree: &py_tree,
                source: py_src.as_bytes(),
            },
        ]);

        let links = linker.link(&endpoints);
        assert_eq!(links.len(), 1, "got {links:#?}");
        assert_eq!(links[0].kind, BridgeKind::Ffi);
        assert_eq!(links[0].export.binding_key, "add");
        assert_eq!(links[0].import.binding_key, "add");
        assert_eq!(links[0].export.language, "rust");
        assert_eq!(links[0].import.language, "python");
        assert!((links[0].confidence - 1.0).abs() < f32::EPSILON);
    }

    #[cfg(all(feature = "lang-c", feature = "lang-java"))]
    #[test]
    fn c_jni_links_java_native() {
        use crate::code::bridge::linker::{BridgeLinker, SourceFile};

        let c_src = "void Java_com_example_Foo_add(int a, int b) {}\n";
        let java_src = "package com.example;\nclass Foo { public native int add(int a, int b); }\n";

        let c_tree = parse_c(c_src);
        let java_tree = parse_java(java_src);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(FfiExtractor));

        let endpoints = linker.extract_all(&[
            SourceFile {
                file_path: "jni.c",
                language: "c",
                tree: &c_tree,
                source: c_src.as_bytes(),
            },
            SourceFile {
                file_path: "Foo.java",
                language: "java",
                tree: &java_tree,
                source: java_src.as_bytes(),
            },
        ]);

        let links = linker.link(&endpoints);
        assert_eq!(links.len(), 1, "got {links:#?}");
        assert_eq!(links[0].kind, BridgeKind::Ffi);
        assert_eq!(links[0].export.binding_key, "Java_com_example_Foo_add");
        assert_eq!(links[0].import.binding_key, "Java_com_example_Foo_add");
    }
}
