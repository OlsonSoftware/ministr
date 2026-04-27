//! Cross-reference extraction from tree-sitter AST nodes.
//!
//! Extracts raw (unresolved) cross-reference candidates from source code across
//! multiple languages: Rust `use` imports, Python `import`/`from` statements,
//! JS/TS `import`/`require` statements, Go `import` declarations, plus
//! Rust-specific `impl Trait for Type` relationships, function/method calls,
//! and type usage in signatures.
//!
//! These are resolved against the stored symbol table during ingestion to
//! produce [`SymbolRefRecord`]s.

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
/// use ministr_core::code::refs::RawRef;
/// use ministr_core::types::RefKind;
///
/// let raw = RawRef {
///     target_name: "MinistrConfig".to_string(),
///     kind: RefKind::Imports,
///     line: 5,
///     from_context: None,
///     target_crate: Some("ministr_core".to_string()),
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
    /// The root crate name from a use path (e.g., `"ministr_core"` from `use ministr_core::Foo`).
    /// Used for cross-crate resolution in workspace contexts.
    pub target_crate: Option<String>,
}

/// Primitive and built-in type names to exclude from type-usage references.
const PRIMITIVE_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8", "u16",
    "u32", "u64", "u128", "usize", "Self",
];

// ---------------------------------------------------------------------------
// Shared query helpers
// ---------------------------------------------------------------------------

/// Extract the 1-based line number from a tree-sitter node.
///
/// Centralises the `row + 1` cast so callers don't each need
/// `#[allow(clippy::cast_possible_truncation)]`.
#[allow(clippy::cast_possible_truncation)]
fn node_line(node: &tree_sitter::Node<'_>) -> u32 {
    node.start_position().row as u32 + 1
}

/// Construct an import [`RawRef`] with the common defaults.
///
/// This is the most-repeated pattern across language extractors: an import
/// reference with no `from_context` and no `target_crate`.
fn import_ref(name: String, line: u32) -> RawRef {
    RawRef {
        target_name: name,
        kind: RefKind::Imports,
        line,
        from_context: None,
        target_crate: None,
    }
}

/// Language-parameterised import extractor.
///
/// Each language provides an implementation that walks root-level children
/// and extracts import references.  The shared [`extract_imports`] driver
/// creates the result vec and invokes the language callback, eliminating
/// the per-language boilerplate.
trait ImportExtractor {
    /// Walk the root-level children of a parsed file and push import refs.
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>);
}

/// Run a language-specific [`ImportExtractor`] on a tree-sitter parse tree.
fn extract_imports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    extractor: &dyn ImportExtractor,
) -> Vec<RawRef> {
    let mut refs = Vec::new();
    let root = tree.root_node();
    extractor.walk_imports(&root, source, &mut refs);
    refs
}

/// Extract raw cross-reference candidates from a tree-sitter AST.
///
/// Dispatches to language-specific extractors based on the `language` name.
/// Supported languages: `"rust"`, `"python"`, `"javascript"`, `"typescript"`,
/// `"tsx"`, `"go"`, `"c"`, `"cpp"`. For unrecognized languages, returns an
/// empty vec.
///
/// Returns unresolved references that must be matched against the symbol
/// table to produce `SymbolRefRecord` values.
#[must_use]
pub fn extract_refs(tree: &tree_sitter::Tree, source: &[u8], language: &str) -> Vec<RawRef> {
    match language {
        "rust" => extract_refs_rust(tree, source),
        "python" => extract_refs_python(tree, source),
        "javascript" | "typescript" | "tsx" => extract_refs_js_ts(tree, source),
        "go" => extract_refs_go(tree, source),
        "c" | "cpp" => extract_refs_c_cpp(tree, source),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Rust
// ---------------------------------------------------------------------------

/// Extract cross-references from Rust source code.
///
/// Walks the AST looking for:
/// - `use` declarations → `RefKind::Imports`
/// - `impl Trait for Type` blocks → `RefKind::Implements`
/// - `call_expression` nodes → `RefKind::Calls`
/// - Type identifiers in function signatures → `RefKind::Uses`
fn extract_refs_rust(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    let mut refs = Vec::new();
    let root = tree.root_node();
    extract_refs_rust_from_node(&root, source, &mut refs);
    refs
}

/// Recursively extract refs from a node and its children.
///
/// Handles top-level items and descends into `mod` blocks to capture
/// references from nested module definitions.
fn extract_refs_rust_from_node(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    refs: &mut Vec<RawRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "use_declaration" => extract_use_refs(&child, source, refs),
            "impl_item" => {
                extract_impl_refs(&child, source, refs);
                extract_from_impl_body(&child, source, refs);
            }
            "function_item" => {
                extract_from_function(&child, source, refs);
            }
            "struct_item" => {
                extract_from_struct(&child, source, refs);
            }
            "enum_item" => {
                extract_from_enum(&child, source, refs);
            }
            "trait_item" => {
                extract_from_trait(&child, source, refs);
            }
            "const_item" | "static_item" => {
                extract_from_const_static(&child, source, refs);
            }
            "type_item" => {
                extract_from_type_alias(&child, source, refs);
            }
            "mod_item" => {
                // Descend into inline module bodies
                if let Some(body) = child.child_by_field_name("body") {
                    extract_refs_rust_from_node(&body, source, refs);
                }
            }
            _ => {}
        }
    }
}

/// Extract import references from a `use_declaration` node.
fn extract_use_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let line = node_line(node);

    // The argument child contains the use path.
    // It can be a scoped_identifier, use_list, use_as_clause, or identifier.
    if let Some(arg) = node.child_by_field_name("argument") {
        // Extract the root crate name before collecting symbol names.
        let root_crate = extract_use_root_crate(&arg, source);

        let start = refs.len();
        collect_use_names(&arg, source, line, refs);

        // Set the target_crate on all newly added refs from this use declaration.
        if let Some(ref crate_name) = root_crate {
            for r in &mut refs[start..] {
                r.target_crate = Some(crate_name.clone());
            }
        }
    }
}

/// Extract the root crate name from a use path node.
///
/// Walks up the `path` chain of nested `scoped_identifier` nodes to find
/// the root identifier. For `use ministr_core::code::refs::RawRef`, returns
/// `Some("ministr_core")`. For bare identifiers or `self`/`super`/`crate`
/// prefixed paths, returns `None`.
fn extract_use_root_crate(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut current = *node;

    // Walk down the path chain: scoped_identifier → path → scoped_identifier → path → ...
    loop {
        match current.kind() {
            "scoped_identifier" | "scoped_use_list" | "use_as_clause" => {
                if let Some(path_node) = current.child_by_field_name("path") {
                    current = path_node;
                } else {
                    break;
                }
            }
            "identifier" | "type_identifier" => {
                let name = current.utf8_text(source).ok()?;
                // Skip Rust path prefixes — these are not external crate names
                if name == "self" || name == "super" || name == "crate" {
                    return None;
                }
                return Some(name.to_string());
            }
            _ => break,
        }
    }
    None
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
                    refs.push(import_ref(name.to_string(), line));
                    // Note: target_crate is set by extract_use_refs after collection
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

    // Strip scope prefix for qualified paths (e.g., "crate::foo::Display" → "Display")
    let trait_name = trait_name.rsplit("::").next().unwrap_or(trait_name);
    let type_name = type_name.rsplit("::").next().unwrap_or(type_name);

    refs.push(RawRef {
        target_name: trait_name.to_string(),
        kind: RefKind::Implements,
        line: node_line(node),
        from_context: Some(type_name.to_string()),
        target_crate: None,
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

/// Extract type references from struct field definitions.
///
/// For `struct Foo { bar: Session, baz: Vec<Config> }`, extracts
/// `Uses` refs for `Session` and `Config`.
fn extract_from_struct(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let struct_name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok());

    // Field declarations are inside the `field_declaration_list` body.
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "field_declaration"
                && let Some(type_node) = child.child_by_field_name("type")
            {
                collect_type_identifiers(&type_node, source, struct_name, refs);
            }
        }
    }
}

/// Extract type references from enum variant fields.
///
/// For `enum Foo { Bar(Session), Baz { x: Config } }`, extracts
/// `Uses` refs for `Session` and `Config`.
fn extract_from_enum(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let enum_name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok());

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "enum_variant" {
                // Recursively collect type identifiers from the entire variant.
                // This handles both tuple variants `A(Session)` and struct
                // variants `B { x: Config }` without special-casing node types.
                collect_type_identifiers(&child, source, enum_name, refs);
            }
        }
    }
}

/// Extract type references from trait method signatures.
///
/// For `trait Foo { fn bar(&self, x: Session) -> Config; }`, extracts
/// `Uses` refs for `Session` and `Config`.
fn extract_from_trait(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let Some(body) = node.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_signature_item" || child.kind() == "function_item" {
            extract_from_function(&child, source, refs);
        }
    }
}

/// Extract type references from `const` and `static` type annotations.
///
/// For `const FOO: Session = ...;`, extracts a `Uses` ref for `Session`.
fn extract_from_const_static(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok());

    if let Some(type_node) = node.child_by_field_name("type") {
        collect_type_identifiers(&type_node, source, name, refs);
    }
}

/// Extract type references from type alias definitions.
///
/// For `type Foo = Vec<Session>;`, extracts a `Uses` ref for `Session`.
fn extract_from_type_alias(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok());

    if let Some(type_node) = node.child_by_field_name("type") {
        collect_type_identifiers(&type_node, source, name, refs);
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

/// Extract one or more call references from a `call_expression` node.
///
/// Handles three callee patterns:
/// - `identifier` → direct function call (`bar()`) — emits `Calls(bar)`.
/// - `scoped_identifier` → qualified call (`MyType::new()`) — emits both
///   `Calls(new)` and `Uses(MyType)` so references on the parent type
///   (or module) surface the call site. Without the `Uses` ref a query
///   like `ministr_references(MyType)` would miss every `MyType::new()`
///   or `MyType::bind(...)` call site in the corpus.
/// - `field_expression` → method call (`x.baz()`) — emits `Calls(baz)`.
///   We can't recover the receiver's type here without full name
///   resolution, so cross-ref from method call → receiver type is
///   intentionally out of scope.
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
            // For `Parent::method(...)` emit a Uses ref on `Parent` *in
            // addition to* the regular Calls(method) below. `Parent` is
            // the immediate parent segment of the callee path — i.e. the
            // name field of the path for nested scopes like
            // `foo::Bar::baz` (→ Uses(Bar)), or the path itself when it's
            // a leaf identifier like `Listener::bind` (→ Uses(Listener)).
            if let Some(parent_name) = immediate_scope_parent(&func_node, source)
                && !is_primitive_type(parent_name)
            {
                refs.push(RawRef {
                    target_name: parent_name.to_string(),
                    kind: RefKind::Uses,
                    line: node_line(node),
                    from_context: fn_context.map(String::from),
                    target_crate: None,
                });
            }

            // Final `name` segment (e.g., `new` from `MyType::new`).
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
        refs.push(RawRef {
            target_name: name.to_string(),
            kind: RefKind::Calls,
            line: node_line(node),
            from_context: fn_context.map(String::from),
            target_crate: None,
        });
    }
}

/// Given a `scoped_identifier` node, return the name of the segment
/// immediately to the left of the final `::` — the type or module whose
/// item is being called.
///
/// For `Listener::bind` → `Some("Listener")`.
/// For `foo::bar::Baz::qux` → `Some("Baz")`.
/// For `crate::baz` → `Some("crate")` (filtered out downstream by the
/// primitive / keyword guard in the caller).
fn immediate_scope_parent<'a>(scoped: &tree_sitter::Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    let path = scoped.child_by_field_name("path")?;
    match path.kind() {
        "identifier" | "type_identifier" => path.utf8_text(source).ok(),
        "scoped_identifier" => path
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok()),
        _ => None,
    }
}

/// Rust primitive type names we don't want to emit ref rows for.
///
/// Used in call-site extraction so `i32::MAX`, `u64::from_ne_bytes(...)`,
/// etc. don't generate noisy refs against non-existent primitive symbols.
fn is_primitive_type(name: &str) -> bool {
    PRIMITIVE_TYPES.contains(&name)
        // Path keywords that show up as the left-most segment in scoped
        // calls but should never resolve to a user-defined symbol.
        || matches!(name, "self" | "Self" | "crate" | "super")
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
            if let Ok(name) = node.utf8_text(source)
                && !PRIMITIVE_TYPES.contains(&name)
            {
                refs.push(RawRef {
                    target_name: name.to_string(),
                    kind: RefKind::Uses,
                    line: node_line(node),
                    from_context: fn_context.map(String::from),
                    target_crate: None,
                });
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

// ---------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------

/// Python import extractor.
///
/// Handles:
/// - `import os` → [`RawRef`] for "os"
/// - `import os.path` → [`RawRef`] for "path" (last segment)
/// - `from os.path import join` → [`RawRef`] for "join"
/// - `from os.path import join, exists` → [`RawRef`] for each name
/// - `from os.path import join as j` → [`RawRef`] for "join" (original name)
/// - `from os.path import *` → skipped (wildcard)
struct PythonImports;

impl ImportExtractor for PythonImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            match node.kind() {
                "import_statement" => extract_python_import(&node, source, refs),
                "import_from_statement" => extract_python_from_import(&node, source, refs),
                _ => {}
            }
        }
    }
}

fn extract_refs_python(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    extract_imports(tree, source, &PythonImports)
}

/// Extract refs from `import x` or `import x.y.z` statements.
fn extract_python_import(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let line = node_line(node);

    let mut child_cursor = node.walk();
    for child in node.children(&mut child_cursor) {
        match child.kind() {
            "dotted_name" => {
                if let Some(name) = last_identifier_text(&child, source) {
                    refs.push(import_ref(name, line));
                }
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name")
                    && let Some(name) = last_identifier_text(&name_node, source)
                {
                    refs.push(import_ref(name, line));
                }
            }
            _ => {}
        }
    }
}

/// Extract refs from `from x import y` statements.
fn extract_python_from_import(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    collect_python_from_names(node, source, node_line(node), refs);
}

/// Collect imported names from a `from ... import ...` statement.
fn collect_python_from_names(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    line: u32,
    refs: &mut Vec<RawRef>,
) {
    // In tree-sitter-python, the from-import structure has children:
    // "from" keyword, module_name (dotted_name), "import" keyword, then imported names.
    // Imported names can be: identifier, aliased_import, or import_list (containing those).
    let mut past_import_keyword = false;
    let mut child_cursor = node.walk();
    for child in node.children(&mut child_cursor) {
        if child.kind() == "import" {
            past_import_keyword = true;
            continue;
        }
        if !past_import_keyword {
            continue;
        }
        match child.kind() {
            "dotted_name" | "identifier" => {
                if let Some(name) = last_identifier_text(&child, source)
                    && name != "*"
                {
                    refs.push(import_ref(name, line));
                }
            }
            "aliased_import" => {
                // `from x import foo as bar` — track original "foo"
                if let Some(name_node) = child.child_by_field_name("name")
                    && let Some(name) = last_identifier_text(&name_node, source)
                {
                    refs.push(import_ref(name, line));
                }
            }
            _ => {}
        }
    }
}

/// Get the text of the last identifier in a node (for dotted names like `os.path`).
fn last_identifier_text(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return node.utf8_text(source).ok().map(String::from);
    }
    if node.kind() == "dotted_name" {
        // Walk children to find the last identifier
        let mut last = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                last = child.utf8_text(source).ok().map(String::from);
            }
        }
        return last;
    }
    node.utf8_text(source).ok().map(String::from)
}

// ---------------------------------------------------------------------------
// JavaScript / TypeScript
// ---------------------------------------------------------------------------

/// JS/TS import extractor.
///
/// Handles:
/// - `import { foo, bar } from 'module'` → [`RawRef`] for "foo", "bar"
/// - `import foo from 'module'` → [`RawRef`] for "foo"
/// - `import * as ns from 'module'` → [`RawRef`] for "ns"
/// - `import 'module'` → skipped (side-effect import)
/// - `const x = require('module')` → [`RawRef`] for "x"
/// - `export { foo } from 'module'` → [`RawRef`] for "foo"
struct JsTsImports;

impl ImportExtractor for JsTsImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            match node.kind() {
                "import_statement" | "import_declaration" | "export_statement" => {
                    extract_js_import(&node, source, refs);
                }
                "lexical_declaration" | "variable_declaration" => {
                    extract_js_require(&node, source, refs);
                }
                _ => {}
            }
        }
    }
}

fn extract_refs_js_ts(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    extract_imports(tree, source, &JsTsImports)
}

/// Extract imported names from a JS/TS import statement.
fn extract_js_import(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let line = node_line(node);

    let mut child_cursor = node.walk();
    for child in node.children(&mut child_cursor) {
        match child.kind() {
            "import_specifier" => {
                extract_js_specifier_name(&child, source, line, refs);
            }
            "import_clause" | "named_imports" | "export_clause" => {
                collect_js_import_names(&child, source, line, refs);
            }
            "namespace_import" => {
                if let Some(name_node) = child.child_by_field_name("name")
                    && let Ok(name) = name_node.utf8_text(source)
                {
                    refs.push(import_ref(name.to_string(), line));
                }
            }
            "identifier" => {
                if let Ok(name) = child.utf8_text(source)
                    && name != "from"
                    && name != "import"
                    && name != "export"
                    && name != "type"
                {
                    refs.push(import_ref(name.to_string(), line));
                }
            }
            _ => {}
        }
    }
}

/// Recursively collect import names from an import clause or `named_imports` node.
fn collect_js_import_names(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    line: u32,
    refs: &mut Vec<RawRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if let Ok(name) = child.utf8_text(source)
                    && name != "from"
                    && name != "type"
                {
                    refs.push(import_ref(name.to_string(), line));
                }
            }
            "import_specifier" | "export_specifier" => {
                extract_js_specifier_name(&child, source, line, refs);
            }
            "named_imports" | "import_clause" | "namespace_import" => {
                collect_js_import_names(&child, source, line, refs);
            }
            _ => {}
        }
    }
}

/// Extract the imported name from an `import_specifier` node.
///
/// For `{ foo as bar }`, extracts "foo" (the original name).
/// For `{ foo }`, extracts "foo".
fn extract_js_specifier_name(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    line: u32,
    refs: &mut Vec<RawRef>,
) {
    // tree-sitter uses `name` field for the original name and `alias` for the local name
    let name_node = node.child_by_field_name("name").or_else(|| {
        // Fallback: first identifier child
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .find(|c| c.kind() == "identifier")
    });

    if let Some(name_node) = name_node
        && let Ok(name) = name_node.utf8_text(source)
        && name != "type"
    {
        refs.push(import_ref(name.to_string(), line));
    }
}

/// Extract refs from `const x = require('module')` patterns.
fn extract_js_require(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let line = node_line(node);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let has_require = child
                .child_by_field_name("value")
                .is_some_and(|val| is_require_call(&val, source));

            if has_require
                && let Some(name_node) = child.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(source)
            {
                refs.push(import_ref(name.to_string(), line));
            }
        }
    }
}

/// Check if a node is a `require(...)` call expression.
fn is_require_call(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    if node.kind() != "call_expression" {
        return false;
    }
    node.child_by_field_name("function")
        .and_then(|f| f.utf8_text(source).ok())
        == Some("require")
}

// ---------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------

/// Go import extractor.
///
/// Handles:
/// - `import "fmt"` → [`RawRef`] for "fmt"
/// - `import "path/filepath"` → [`RawRef`] for "filepath" (last path segment)
/// - `import f "fmt"` → [`RawRef`] for "fmt"
/// - `import ( "fmt"; "os" )` → [`RawRef`] for each package
/// - `import . "fmt"` → skipped (dot import, like wildcard)
/// - `import _ "net/http/pprof"` → skipped (blank import, side-effect only)
struct GoImports;

impl ImportExtractor for GoImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() == "import_declaration" {
                extract_go_imports(&node, source, refs);
            }
        }
    }
}

fn extract_refs_go(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    extract_imports(tree, source, &GoImports)
}

/// Extract import names from a Go import declaration.
fn extract_go_imports(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let line = node_line(node);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                extract_go_import_spec(&child, source, line, refs);
            }
            "import_spec_list" => {
                let mut inner_cursor = child.walk();
                for spec in child.children(&mut inner_cursor) {
                    if spec.kind() == "import_spec" {
                        extract_go_import_spec(&spec, source, node_line(&spec), refs);
                    }
                }
            }
            "interpreted_string_literal" | "raw_string_literal" => {
                if let Some(name) = extract_go_package_name(&child, source) {
                    refs.push(import_ref(name, line));
                }
            }
            _ => {}
        }
    }
}

/// Extract a single import spec (possibly with alias).
fn extract_go_import_spec(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    line: u32,
    refs: &mut Vec<RawRef>,
) {
    // Check for alias: `import f "fmt"` or `import . "fmt"` or `import _ "fmt"`
    let alias = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok());

    // Skip blank imports (`_`) and dot imports (`.`)
    if let Some(alias) = alias
        && (alias == "_" || alias == ".")
    {
        return;
    }

    // Find the import path string
    let path_node = node.child_by_field_name("path");
    if let Some(path_node) = path_node
        && let Some(name) = extract_go_package_name(&path_node, source)
    {
        refs.push(import_ref(name, line));
    }
}

/// Extract the package name from a Go import path string.
///
/// For `"fmt"` → `"fmt"`, for `"path/filepath"` → `"filepath"`.
/// Strips surrounding quotes.
fn extract_go_package_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    // Strip quotes
    let path = text.trim_matches('"').trim_matches('`');
    if path.is_empty() {
        return None;
    }
    // Last segment of the path
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

// ---------------------------------------------------------------------------
// C / C++
// ---------------------------------------------------------------------------

/// Extract `#include` references from C / C++ source.
///
/// Walks the tree recursively (includes can sit inside `preproc_ifdef`
/// header guards or `extern "C"` linkage_specification blocks) looking for
/// `preproc_include` nodes. The reference target is the include path with
/// brackets/quotes stripped — e.g. `<stdio.h>` → `stdio.h`,
/// `"foo/bar.h"` → `foo/bar.h`.
fn extract_refs_c_cpp(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    let mut refs = Vec::new();
    walk_c_includes(&tree.root_node(), source, &mut refs);
    refs
}

fn walk_c_includes(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "preproc_include" {
            if let Some(path) = extract_c_include_path(&child, source) {
                refs.push(import_ref(path, node_line(&child)));
            }
        } else {
            // Recurse into preproc guards, linkage_specification, namespaces, etc.
            walk_c_includes(&child, source, refs);
        }
    }
}

fn extract_c_include_path(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let path_node = node.child_by_field_name("path")?;
    let raw = path_node.utf8_text(source).ok()?.trim();
    let stripped = raw
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .or_else(|| raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')))?;
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::AstParser;

    fn parse_and_extract(source: &str) -> Vec<RawRef> {
        let mut parser = AstParser::new();
        let tree = parser.parse(source.as_bytes()).unwrap();
        extract_refs(&tree, source.as_bytes(), "rust")
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
        let refs = parse_and_extract("use crate::config::MinistrConfig;");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_name, "MinistrConfig");
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
        let refs = parse_and_extract("use crate::config::MinistrConfig;");
        // Should only have MinistrConfig, not "crate" or "config"
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_name, "MinistrConfig");
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

    // --- Python import extraction tests (C8.2) ---

    #[cfg(feature = "lang-python")]
    mod python_tests {
        use super::*;

        fn parse_python(source: &str) -> Vec<RawRef> {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .unwrap();
            let tree = parser.parse(source.as_bytes(), None).unwrap();
            extract_refs(&tree, source.as_bytes(), "python")
        }

        #[test]
        fn simple_import() {
            let refs = parse_python("import os");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "os");
            assert_eq!(refs[0].kind, RefKind::Imports);
        }

        #[test]
        fn dotted_import() {
            let refs = parse_python("import os.path");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "path");
        }

        #[test]
        fn from_import_single() {
            let refs = parse_python("from os.path import join");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "join");
            assert_eq!(refs[0].kind, RefKind::Imports);
        }

        #[test]
        fn from_import_multiple() {
            let refs = parse_python("from os.path import join, exists, isfile");
            assert_eq!(refs.len(), 3);
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"join"));
            assert!(names.contains(&"exists"));
            assert!(names.contains(&"isfile"));
        }

        #[test]
        fn from_import_alias() {
            let refs = parse_python("from collections import OrderedDict as OD");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "OrderedDict");
        }

        #[test]
        fn from_import_wildcard_skipped() {
            let refs = parse_python("from os.path import *");
            assert!(
                refs.is_empty(),
                "wildcard imports should be skipped, got: {refs:?}"
            );
        }

        #[test]
        fn import_alias() {
            let refs = parse_python("import numpy as np");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "numpy");
        }

        #[test]
        fn multiple_imports() {
            let refs = parse_python("import os\nimport sys\nfrom pathlib import Path");
            assert_eq!(refs.len(), 3);
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"os"));
            assert!(names.contains(&"sys"));
            assert!(names.contains(&"Path"));
        }
    }

    // --- JS/TS import extraction tests (C8.2) ---

    #[cfg(feature = "lang-typescript")]
    mod typescript_tests {
        use super::*;

        fn parse_ts(source: &str) -> Vec<RawRef> {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                .unwrap();
            let tree = parser.parse(source.as_bytes(), None).unwrap();
            extract_refs(&tree, source.as_bytes(), "typescript")
        }

        #[test]
        fn named_import() {
            let refs = parse_ts("import { foo, bar } from 'module';");
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"foo"), "missing foo, got: {names:?}");
            assert!(names.contains(&"bar"), "missing bar, got: {names:?}");
        }

        #[test]
        fn default_import() {
            let refs = parse_ts("import React from 'react';");
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"React"), "missing React, got: {names:?}");
        }

        #[test]
        fn namespace_import() {
            let refs = parse_ts("import * as path from 'path';");
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"path"), "missing path, got: {names:?}");
        }

        #[test]
        fn aliased_import() {
            let refs = parse_ts("import { foo as myFoo } from 'module';");
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(
                names.contains(&"foo"),
                "should track original name 'foo', got: {names:?}"
            );
        }

        #[test]
        fn side_effect_import_skipped() {
            let refs = parse_ts("import 'side-effect';");
            assert!(
                refs.is_empty(),
                "side-effect imports should produce no refs, got: {refs:?}"
            );
        }

        #[test]
        fn mixed_imports() {
            let source =
                "import React from 'react';\nimport { useState, useEffect } from 'react';\n";
            let refs = parse_ts(source);
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"React"));
            assert!(names.contains(&"useState"));
            assert!(names.contains(&"useEffect"));
        }
    }

    #[cfg(feature = "lang-javascript")]
    mod javascript_tests {
        use super::*;

        fn parse_js(source: &str) -> Vec<RawRef> {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_javascript::LANGUAGE.into())
                .unwrap();
            let tree = parser.parse(source.as_bytes(), None).unwrap();
            extract_refs(&tree, source.as_bytes(), "javascript")
        }

        #[test]
        fn require_call() {
            let refs = parse_js("const fs = require('fs');");
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"fs"), "missing fs, got: {names:?}");
        }

        #[test]
        fn named_import_js() {
            let refs = parse_js("import { readFile } from 'fs';");
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(
                names.contains(&"readFile"),
                "missing readFile, got: {names:?}"
            );
        }
    }

    // --- Go import extraction tests (C8.2) ---

    #[cfg(feature = "lang-go")]
    mod go_tests {
        use super::*;

        fn parse_go(source: &str) -> Vec<RawRef> {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_go::LANGUAGE.into())
                .unwrap();
            let tree = parser.parse(source.as_bytes(), None).unwrap();
            extract_refs(&tree, source.as_bytes(), "go")
        }

        #[test]
        fn single_import() {
            let refs = parse_go("package main\n\nimport \"fmt\"\n");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "fmt");
            assert_eq!(refs[0].kind, RefKind::Imports);
        }

        #[test]
        fn grouped_imports() {
            let source = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n";
            let refs = parse_go(source);
            assert_eq!(refs.len(), 2);
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"fmt"));
            assert!(names.contains(&"os"));
        }

        #[test]
        fn path_import_extracts_last_segment() {
            let refs = parse_go("package main\n\nimport \"path/filepath\"\n");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "filepath");
        }

        #[test]
        fn aliased_import() {
            let refs = parse_go("package main\n\nimport f \"fmt\"\n");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "fmt");
        }

        #[test]
        fn blank_import_skipped() {
            let refs = parse_go("package main\n\nimport _ \"net/http/pprof\"\n");
            assert!(
                refs.is_empty(),
                "blank imports should be skipped, got: {refs:?}"
            );
        }

        #[test]
        fn dot_import_skipped() {
            let refs = parse_go("package main\n\nimport . \"fmt\"\n");
            assert!(
                refs.is_empty(),
                "dot imports should be skipped, got: {refs:?}"
            );
        }

        #[test]
        fn mixed_go_imports() {
            let source = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n\t\"path/filepath\"\n\t_ \"net/http/pprof\"\n)\n";
            let refs = parse_go(source);
            assert_eq!(refs.len(), 3);
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"fmt"));
            assert!(names.contains(&"os"));
            assert!(names.contains(&"filepath"));
        }
    }

    // --- C / C++ #include extraction ---

    #[cfg(feature = "lang-c")]
    mod c_tests {
        use super::*;

        fn parse_c(source: &str) -> Vec<RawRef> {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_c::LANGUAGE.into())
                .unwrap();
            let tree = parser.parse(source.as_bytes(), None).unwrap();
            extract_refs(&tree, source.as_bytes(), "c")
        }

        #[test]
        fn system_include() {
            let refs = parse_c("#include <stdio.h>\n");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "stdio.h");
            assert_eq!(refs[0].kind, RefKind::Imports);
        }

        #[test]
        fn quoted_include() {
            let refs = parse_c("#include \"local.h\"\n");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "local.h");
        }

        #[test]
        fn nested_path_include() {
            let refs = parse_c("#include \"sub/dir/foo.h\"\n");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "sub/dir/foo.h");
        }

        #[test]
        fn includes_inside_header_guard() {
            let refs = parse_c(
                "#ifndef HELLO_H\n#define HELLO_H\n#include <stdio.h>\n#include \"foo.h\"\n#endif\n",
            );
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"stdio.h"), "got: {names:?}");
            assert!(names.contains(&"foo.h"), "got: {names:?}");
        }
    }

    #[cfg(feature = "lang-cpp")]
    mod cpp_tests {
        use super::*;

        fn parse_cpp(source: &str) -> Vec<RawRef> {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_cpp::LANGUAGE.into())
                .unwrap();
            let tree = parser.parse(source.as_bytes(), None).unwrap();
            extract_refs(&tree, source.as_bytes(), "cpp")
        }

        #[test]
        fn extern_c_block_includes() {
            let refs = parse_cpp("extern \"C\" {\n#include <string.h>\n}\n");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].target_name, "string.h");
        }
    }

    // --- Unsupported language returns empty ---

    #[test]
    fn unsupported_language_returns_empty() {
        let mut parser = AstParser::new();
        let tree = parser.parse(b"fn main() {}").unwrap();
        let refs = extract_refs(&tree, b"fn main() {}", "unknown");
        assert!(refs.is_empty());
    }

    // --- target_crate extraction tests ---

    #[test]
    fn target_crate_from_simple_use() {
        let refs = parse_and_extract("use std::collections::HashMap;");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_crate.as_deref(), Some("std"));
    }

    #[test]
    fn target_crate_from_grouped_use() {
        let refs = parse_and_extract("use ministr_core::code::{RawRef, RefKind};");
        assert_eq!(refs.len(), 2);
        for r in &refs {
            assert_eq!(r.target_crate.as_deref(), Some("ministr_core"));
        }
    }

    #[test]
    fn target_crate_none_for_crate_prefix() {
        let refs = parse_and_extract("use crate::config::MinistrConfig;");
        assert_eq!(refs.len(), 1);
        // `crate::` paths are local — target_crate should be None
        assert_eq!(refs[0].target_crate, None);
    }

    #[test]
    fn target_crate_none_for_self_prefix() {
        let refs = parse_and_extract("use self::submodule::Thing;");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_crate, None);
    }

    #[test]
    fn target_crate_none_for_super_prefix() {
        let refs = parse_and_extract("use super::config::Config;");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_crate, None);
    }

    #[test]
    fn target_crate_from_use_as() {
        let refs = parse_and_extract("use serde::Serialize as Ser;");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target_crate.as_deref(), Some("serde"));
    }

    #[test]
    fn target_crate_none_for_bare_identifier() {
        // `use HashMap;` — bare identifier, no crate path
        let refs = parse_and_extract("use HashMap;");
        assert_eq!(refs.len(), 1);
        // A bare identifier with no scoping — the name itself would be the "root"
        // but since it's not a path (no ::), we treat it as the crate name
        // This is fine — it maps to nothing useful in a workspace graph
        assert_eq!(refs[0].target_crate.as_deref(), Some("HashMap"));
    }

    #[test]
    fn target_crate_not_set_on_impl_refs() {
        let source = r"
            pub struct Foo;
            pub trait Display {}
            impl Display for Foo {}
        ";
        let refs = parse_and_extract(source);
        let impl_ref = refs.iter().find(|r| r.kind == RefKind::Implements).unwrap();
        assert_eq!(impl_ref.target_crate, None);
    }

    #[test]
    fn target_crate_not_set_on_call_refs() {
        let source = r"fn main() { foo(); }";
        let refs = parse_and_extract(source);
        let call_ref = refs.iter().find(|r| r.kind == RefKind::Calls).unwrap();
        assert_eq!(call_ref.target_crate, None);
    }

    // --- Scoped trait/type name extraction ---

    #[test]
    fn extract_impl_scoped_trait_strips_path() {
        let source = r"
            pub struct Foo;
            impl crate::traits::Display for Foo {}
        ";
        let refs = parse_and_extract(source);
        let impl_ref = refs.iter().find(|r| r.kind == RefKind::Implements).unwrap();
        assert_eq!(impl_ref.target_name, "Display");
        assert_eq!(impl_ref.from_context.as_deref(), Some("Foo"));
    }

    #[test]
    fn extract_impl_scoped_type_strips_path() {
        let source = r"
            pub trait Bar {}
            impl Bar for super::other::Baz {}
        ";
        let refs = parse_and_extract(source);
        let impl_ref = refs.iter().find(|r| r.kind == RefKind::Implements).unwrap();
        assert_eq!(impl_ref.from_context.as_deref(), Some("Baz"));
    }

    // --- Struct field type extraction ---

    #[test]
    fn extract_struct_field_types() {
        let source = r"
            pub struct Foo {
                pub bar: Session,
                pub baz: Vec<Config>,
            }
        ";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        let names: Vec<&str> = uses.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Session"), "missing Session ref: {names:?}");
        assert!(names.contains(&"Config"), "missing Config ref: {names:?}");
    }

    // --- Enum variant type extraction ---

    #[test]
    fn extract_enum_variant_types() {
        let source = r"
            pub enum MyEnum {
                A(Session),
                B { x: Config },
            }
        ";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        let names: Vec<&str> = uses.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Session"), "missing Session ref: {names:?}");
        assert!(names.contains(&"Config"), "missing Config ref: {names:?}");
    }

    // --- Trait method signature extraction ---

    #[test]
    fn extract_trait_method_types() {
        let source = r"
            pub trait MyTrait {
                fn process(&self, input: Session) -> Config;
            }
        ";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        let names: Vec<&str> = uses.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Session"), "missing Session ref: {names:?}");
        assert!(names.contains(&"Config"), "missing Config ref: {names:?}");
    }

    // --- Const/static type extraction ---

    #[test]
    fn extract_const_type() {
        let source = r"const MY_CONST: Config = Config::default();";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        assert!(
            uses.iter().any(|r| r.target_name == "Config"),
            "missing Config type ref: {uses:?}"
        );
    }

    // --- Mod block descent ---

    #[test]
    fn extract_refs_inside_mod_block() {
        let source = r"
            mod inner {
                use super::Config;
                pub struct Wrapper {
                    pub cfg: Config,
                }
            }
        ";
        let refs = parse_and_extract(source);
        let imports: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Imports).collect();
        assert!(
            imports.iter().any(|r| r.target_name == "Config"),
            "should find import inside mod block: {imports:?}"
        );
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        assert!(
            uses.iter().any(|r| r.target_name == "Config"),
            "should find struct field type ref inside mod block: {uses:?}"
        );
    }

    // --- Method-level from_context ---

    #[test]
    fn impl_method_call_has_function_from_context() {
        let source = r"
            struct Player;
            impl Player {
                fn check(&self) {
                    self.validate();
                }
            }
        ";
        let refs = parse_and_extract(source);
        let calls: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Calls).collect();
        assert!(
            calls
                .iter()
                .any(|r| r.target_name == "validate" && r.from_context == Some("check".into())),
            "method call should have enclosing method as from_context: {calls:?}"
        );
    }

    #[test]
    fn free_function_call_has_function_from_context() {
        let source = r"
            fn main() {
                helper();
            }
        ";
        let refs = parse_and_extract(source);
        let calls: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Calls).collect();
        assert!(
            calls
                .iter()
                .any(|r| r.target_name == "helper" && r.from_context == Some("main".into())),
            "call should have enclosing function as from_context: {calls:?}"
        );
    }

    // --- Scoped-call type-of-parent refs (the "Listener::bind" fix) ---
    //
    // Prior to this change, `Listener::bind(...)` recorded only
    // `Calls(bind)`, so `ministr_references(Listener)` missed every call
    // site. These tests lock in that `Type::method(...)` emits a
    // `Uses(Type)` ref alongside `Calls(method)` so the parent type's
    // reference list picks up the call.

    #[test]
    fn scoped_call_emits_uses_ref_for_parent_type() {
        let source = r"
            fn build() {
                let cfg = Config::new();
            }
        ";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Uses && r.target_name == "Config")
            .collect();
        assert_eq!(
            uses.len(),
            1,
            "expected exactly one Uses(Config) from Config::new(): {refs:?}"
        );
        assert_eq!(uses[0].from_context.as_deref(), Some("build"));
    }

    #[test]
    fn scoped_call_uses_immediate_parent_for_nested_paths() {
        // For `foo::Bar::baz()` the Uses ref should target `Bar`, not
        // `foo` — it's the immediate type/module whose method is called.
        let source = r"
            fn work() {
                foo::Bar::baz();
            }
        ";
        let refs = parse_and_extract(source);
        let uses_targets: Vec<&str> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Uses)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            uses_targets.contains(&"Bar"),
            "expected Uses(Bar) from foo::Bar::baz(): {uses_targets:?}"
        );
        assert!(
            !uses_targets.contains(&"foo"),
            "should not emit Uses(foo) for the outer module: {uses_targets:?}"
        );
    }

    #[test]
    fn scoped_call_skips_primitive_and_keyword_parents() {
        // Parents like `i32`, `Self`, `crate`, `super` should never
        // produce a Uses ref — they can't resolve to a user-defined
        // symbol and would just be noise for the resolver.
        let source = r"
            fn nope() {
                let _ = i32::MAX;
                let _ = Self::helper();
                crate::util::reset();
                super::parent::tick();
            }
        ";
        let refs = parse_and_extract(source);
        let uses_targets: Vec<&str> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Uses)
            .map(|r| r.target_name.as_str())
            .collect();
        for bad in ["i32", "Self", "crate", "super"] {
            assert!(
                !uses_targets.contains(&bad),
                "should not emit Uses({bad}) from primitive/keyword scope: {uses_targets:?}"
            );
        }
    }

    #[test]
    fn scoped_call_still_emits_method_calls_ref() {
        // Regression guard: adding the Uses ref must NOT replace the
        // Calls ref — both need to land so direct-method-name queries
        // keep working.
        let source = r"
            fn go() {
                Listener::bind(&addr);
            }
        ";
        let refs = parse_and_extract(source);
        let has_calls_bind = refs
            .iter()
            .any(|r| r.kind == RefKind::Calls && r.target_name == "bind");
        let has_uses_listener = refs
            .iter()
            .any(|r| r.kind == RefKind::Uses && r.target_name == "Listener");
        assert!(has_calls_bind, "missing Calls(bind): {refs:?}");
        assert!(has_uses_listener, "missing Uses(Listener): {refs:?}");
    }
}
