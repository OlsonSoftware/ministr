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
///
/// Covers Rust scalar primitives plus the universal-prelude stdlib type
/// names. These shadow user types of the same name (e.g., a project's
/// `Command` enum), so leaving them in produces phantom cross-crate
/// bindings during reference resolution — the indexer picks the only
/// in-corpus `Command` and ties unrelated `std::process::Command::new(...)`
/// call sites to it. Skipping these at extraction time means stdlib
/// references stay unresolved (as they should — stdlib isn't in the
/// corpus), and same-named user types only get edges when they're
/// actually referenced.
const PRIMITIVE_TYPES: &[&str] = &[
    // Rust scalar primitives.
    "bool",
    "char",
    "f32",
    "f64",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "str",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "Self",
    // Rust prelude / extremely common stdlib types.
    // Collections and ownership.
    "String",
    "Vec",
    "Box",
    "Rc",
    "Arc",
    "Cell",
    "RefCell",
    "Mutex",
    "RwLock",
    "Weak",
    "HashMap",
    "HashSet",
    "BTreeMap",
    "BTreeSet",
    "VecDeque",
    "LinkedList",
    "BinaryHeap",
    // Core enums / wrappers.
    "Option",
    "Result",
    "Cow",
    "Ordering",
    // Iterators / closures.
    "Iterator",
    "IntoIterator",
    "FromIterator",
    "Fn",
    "FnMut",
    "FnOnce",
    // I/O and OS.
    "Command",
    "Child",
    "Stdio",
    "Output",
    "ExitStatus",
    "PathBuf",
    "Path",
    "OsString",
    "OsStr",
    "Read",
    "Write",
    "BufRead",
    "BufReader",
    "BufWriter",
    "File",
    "Error",
    "ErrorKind",
    // Networking.
    "IpAddr",
    "Ipv4Addr",
    "Ipv6Addr",
    "SocketAddr",
    "TcpListener",
    "TcpStream",
    "UdpSocket",
    // Time.
    "Duration",
    "Instant",
    "SystemTime",
    // Sync / future / channel primitives.
    "Sender",
    "Receiver",
    "Future",
    "Pin",
    "Poll",
    "Context",
    "Waker",
    // Common conversion / range / numeric.
    "From",
    "Into",
    "TryFrom",
    "TryInto",
    "AsRef",
    "AsMut",
    "Borrow",
    "BorrowMut",
    "Default",
    "Clone",
    "Copy",
    "Drop",
    "Debug",
    "Display",
    "Range",
    "RangeFrom",
    "RangeTo",
    "RangeInclusive",
    "NonZeroU8",
    "NonZeroU16",
    "NonZeroU32",
    "NonZeroU64",
    "NonZeroUsize",
    "NonZeroI8",
    "NonZeroI16",
    "NonZeroI32",
    "NonZeroI64",
    "NonZeroIsize",
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
/// `"tsx"`, `"go"`, `"c"`, `"cpp"`, `"php"`, `"kotlin"`, `"scala"`,
/// `"java"`, `"csharp"`, `"swift"`, `"ruby"`. For unrecognized
/// languages, returns an empty vec.
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
        "php" => extract_refs_php(tree, source),
        "kotlin" => extract_refs_kotlin(tree, source),
        "scala" => extract_refs_scala(tree, source),
        "java" => extract_refs_java(tree, source),
        "csharp" => extract_refs_csharp(tree, source),
        "swift" => extract_refs_swift(tree, source),
        "ruby" => extract_refs_ruby(tree, source),
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
    // Imports from the root walker; the call/heritage/type graph from a
    // full-tree walk — Python is import-only no more.
    let mut refs = extract_imports(tree, source, &PythonImports);
    walk_python_refs(&tree.root_node(), source, &mut refs);
    refs
}

/// Recursively emit `Calls`/`Implements`/`Uses` edges for Python. Complements
/// [`PythonImports`] (which handles `import` / `from … import`):
///
/// - `class_definition` superclasses → `Implements` for each positional base
///   (`class C(Base, Mixin)`); keyword args like `metaclass=…` are skipped.
///   Python has no interfaces, so a base class IS the conformance signal (ABCs
///   are the de-facto interfaces).
/// - `call` → `Calls` (callee `identifier`, or the final `attribute` of a
///   method call). Python has no `new`, so `Widget()` surfaces as
///   `Calls(Widget)`.
/// - `type` annotations (parameter / return / variable) → `Uses` for each
///   named type inside (recursing generics; `None` carries no identifier).
///
/// `from_context` is `None`; the line-based resolver attributes each edge to
/// its enclosing symbol.
fn walk_python_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "class_definition" => python_heritage(node, source, refs),
        "call" => python_call(node, source, refs),
        "type" => python_type_uses(node, source, refs),
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_python_refs(&child, source, refs);
    }
}

/// Emit an `Implements` edge for each positional base class in a
/// `class_definition`'s `superclasses` argument list (skipping `keyword_argument`s
/// such as `metaclass=…`).
fn python_heritage(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let Some(bases) = node.child_by_field_name("superclasses") else {
        return;
    };
    let mut cursor = bases.walk();
    for base in bases.children(&mut cursor) {
        let name = match base.kind() {
            "identifier" => base.utf8_text(source).ok(),
            // `module.Base` — take the final attribute segment.
            "attribute" => base
                .child_by_field_name("attribute")
                .and_then(|n| n.utf8_text(source).ok()),
            _ => None,
        };
        if let Some(name) = name {
            push_graph_ref(refs, name, RefKind::Implements, node_line(&base));
        }
    }
}

/// Emit a `Calls` edge from a `call`: callee `identifier` (`foo()`,
/// `Widget()`), or the final `attribute` of a method call (`a.b.foo()` →
/// `foo`).
fn python_call(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let Some(func) = node.child_by_field_name("function") else {
        return;
    };
    let name = match func.kind() {
        "identifier" => func.utf8_text(source).ok(),
        "attribute" => func
            .child_by_field_name("attribute")
            .and_then(|n| n.utf8_text(source).ok()),
        _ => None,
    };
    if let Some(name) = name {
        push_graph_ref(refs, name, RefKind::Calls, node_line(node));
    }
}

/// Emit `Uses` for every named type inside a `type` annotation subtree
/// (recursing through subscripts/generics). `None` is a `none` node, not an
/// `identifier`, so it is naturally skipped.
fn python_type_uses(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    if node.kind() == "identifier" {
        if let Ok(name) = node.utf8_text(source) {
            push_graph_ref(refs, name, RefKind::Uses, node_line(node));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        python_type_uses(&child, source, refs);
    }
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
    // Imports come from the dedicated root-level walker; the call/heritage/
    // type graph comes from a full-tree walk (call sites and type uses are
    // nested deep inside method bodies). Together they give the JS/TS/TSX
    // family a real Calls/Implements/Uses edge graph, not import-only.
    let mut refs = extract_imports(tree, source, &JsTsImports);
    walk_js_ts_refs(&tree.root_node(), source, &mut refs);
    refs
}

/// Recursively emit `Calls`/`Implements`/`Uses` edges for the JS/TS/TSX
/// family. Complements [`JsTsImports`] (which handles `import`/`require`):
///
/// - `class_declaration` heritage → `Implements` for each `extends` base and
///   each `implements` interface; `interface_declaration extends` likewise.
/// - `call_expression` → `Calls` (callee `identifier`, or the `.property` of
///   a `member_expression` method call).
/// - `new_expression` constructor + `type_annotation` type names → `Uses`.
///
/// `from_context` is left `None`; the line-based resolver attributes each
/// edge to its enclosing symbol. Unresolved targets (builtins like `Promise`,
/// host methods like `toString`) simply never bind to an in-corpus symbol.
fn walk_js_ts_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "class_declaration" | "class" => js_ts_class_heritage(node, source, refs),
        "interface_declaration" => js_ts_interface_heritage(node, source, refs),
        "call_expression" => js_ts_call_ref(node, source, refs),
        "new_expression" => js_ts_new_ref(node, source, refs),
        "type_annotation" => js_ts_type_uses(node, source, refs),
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_js_ts_refs(&child, source, refs);
    }
}

/// Push a `Calls`/`Implements`/`Uses` edge-graph ref with the shared defaults
/// (`from_context = None`, so the line-based resolver attributes the `from`
/// side). `Uses`/`Implements` targets are filtered through
/// [`is_primitive_type`]; `Calls` targets are not (a method named `new` is a
/// real call, not a primitive type). Shared by the per-language edge-graph
/// walkers (JS/TS, Java, …).
fn push_graph_ref(refs: &mut Vec<RawRef>, name: &str, kind: RefKind, line: u32) {
    if name.is_empty() {
        return;
    }
    if matches!(kind, RefKind::Uses | RefKind::Implements) && is_primitive_type(name) {
        return;
    }
    refs.push(RawRef {
        target_name: name.to_string(),
        kind,
        line,
        from_context: None,
        target_crate: None,
    });
}

/// Resolve the bound type name of a heritage/constructor node:
/// `identifier`/`type_identifier` directly, the `.property` of a
/// `member_expression`, the final segment of a `nested_type_identifier`, or
/// the `name` of a `generic_type` (`implements IFoo<T>` → `IFoo`).
fn js_ts_type_name<'a>(node: &tree_sitter::Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    match node.kind() {
        "identifier" | "type_identifier" | "property_identifier" => node.utf8_text(source).ok(),
        "member_expression" => node
            .child_by_field_name("property")
            .and_then(|p| p.utf8_text(source).ok()),
        "nested_type_identifier" => node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok()),
        "generic_type" => node
            .child_by_field_name("name")
            .and_then(|n| js_ts_type_name(&n, source)),
        _ => None,
    }
}

/// Emit `Implements` edges from a `class_declaration`'s `class_heritage`
/// (both `extends Base` and `implements I, J`).
fn js_ts_class_heritage(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "class_heritage" {
            continue;
        }
        let mut hc = child.walk();
        for clause in child.children(&mut hc) {
            match clause.kind() {
                "extends_clause" => {
                    if let Some(base) = clause.child_by_field_name("value")
                        && let Some(name) = js_ts_type_name(&base, source)
                    {
                        push_graph_ref(refs, name, RefKind::Implements, node_line(&clause));
                    }
                }
                "implements_clause" => {
                    let mut ic = clause.walk();
                    for t in clause.children(&mut ic) {
                        if let Some(name) = js_ts_type_name(&t, source) {
                            push_graph_ref(refs, name, RefKind::Implements, node_line(&clause));
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Emit `Implements` edges from an `interface_declaration`'s
/// `extends_type_clause` (`interface I extends A, B`).
fn js_ts_interface_heritage(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "extends_type_clause" {
            continue;
        }
        let mut ic = child.walk();
        for t in child.children(&mut ic) {
            if let Some(name) = js_ts_type_name(&t, source) {
                push_graph_ref(refs, name, RefKind::Implements, node_line(&child));
            }
        }
    }
}

/// Emit a `Calls` edge from a `call_expression` callee: a bare `identifier`
/// (`helper()`) or the `.property` of a `member_expression` (`x.compute()`).
fn js_ts_call_ref(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let Some(func) = node.child_by_field_name("function") else {
        return;
    };
    let callee = match func.kind() {
        "identifier" => func.utf8_text(source).ok(),
        "member_expression" => func
            .child_by_field_name("property")
            .and_then(|p| p.utf8_text(source).ok()),
        _ => None,
    };
    if let Some(name) = callee {
        push_graph_ref(refs, name, RefKind::Calls, node_line(node));
    }
}

/// Emit a `Uses` edge from a `new_expression` constructor (`new Widget()`).
fn js_ts_new_ref(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    if let Some(ctor) = node.child_by_field_name("constructor")
        && let Some(name) = js_ts_type_name(&ctor, source)
    {
        push_graph_ref(refs, name, RefKind::Uses, node_line(node));
    }
}

/// Emit `Uses` edges for every named type inside a `type_annotation`
/// (parameter / field / return-type positions), recursing through unions,
/// generics, and arrays. `predefined_type` nodes (`void`/`string`/...) carry
/// no `type_identifier`, so they are naturally skipped.
fn js_ts_type_uses(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "type_identifier" => {
            if let Ok(name) = node.utf8_text(source) {
                push_graph_ref(refs, name, RefKind::Uses, node_line(node));
            }
        }
        "nested_type_identifier" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
            {
                push_graph_ref(refs, name, RefKind::Uses, node_line(node));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        js_ts_type_uses(&child, source, refs);
    }
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

// ---------------------------------------------------------------------------
// PHP / Kotlin / Scala imports
// ---------------------------------------------------------------------------

/// Last `\`-separated segment of a PHP qualified name, stripping a trailing
/// ` as Alias`. `Foo\Bar` → `Bar`; `Foo\Baz as Q` → `Baz`.
fn php_use_name(clause: &str) -> Option<String> {
    let path = clause.split(" as ").next().unwrap_or(clause).trim();
    let seg = path.rsplit('\\').next().unwrap_or(path).trim();
    (!seg.is_empty()).then(|| seg.to_string())
}

struct PhpImports;
impl ImportExtractor for PhpImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() != "namespace_use_declaration" {
                continue;
            }
            let line = node_line(&node);
            let mut c2 = node.walk();
            for child in node.children(&mut c2) {
                if child.kind() == "namespace_use_clause"
                    && let Ok(text) = child.utf8_text(source)
                    && let Some(name) = php_use_name(text)
                {
                    refs.push(import_ref(name, line));
                }
            }
        }
    }
}

struct KotlinImports;
impl ImportExtractor for KotlinImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() != "import" {
                continue;
            }
            let line = node_line(&node);
            if let Some(qi) = node.child_by_field_name("name").or_else(|| {
                let mut c2 = node.walk();
                node.children(&mut c2)
                    .find(|c| c.kind() == "qualified_identifier")
            }) && let Ok(text) = qi.utf8_text(source)
            {
                // `com.x.Y` → `Y`; wildcard `com.x.*` → skipped.
                let last = text.rsplit('.').next().unwrap_or(text).trim();
                if !last.is_empty() && last != "*" {
                    refs.push(import_ref(last.to_string(), line));
                }
            }
        }
    }
}

struct ScalaImports;
impl ImportExtractor for ScalaImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() != "import_declaration" {
                continue;
            }
            let line = node_line(&node);
            let mut c2 = node.walk();
            let mut last_ident: Option<String> = None;
            for child in node.children(&mut c2) {
                match child.kind() {
                    "identifier" => {
                        if let Ok(t) = child.utf8_text(source) {
                            last_ident = Some(t.trim().to_string());
                        }
                    }
                    // `import a.b.{A, B}` — push each selected name.
                    "namespace_selectors" => {
                        last_ident = None;
                        let mut c3 = child.walk();
                        for sel in child.children(&mut c3) {
                            if sel.kind() == "identifier"
                                && let Ok(t) = sel.utf8_text(source)
                            {
                                let t = t.trim();
                                if !t.is_empty() && t != "_" {
                                    refs.push(import_ref(t.to_string(), line));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(name) = last_ident
                && !name.is_empty()
                && name != "_"
            {
                refs.push(import_ref(name, line));
            }
        }
    }
}

/// Java `import a.b.C;` → `C` (wildcard `import a.b.*;` skipped). Mirrors
/// the Kotlin extractor — JVM dotted imports, last segment is the symbol.
struct JavaImports;
impl ImportExtractor for JavaImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() != "import_declaration" {
                continue;
            }
            let line = node_line(&node);
            let mut c2 = node.walk();
            for child in node.children(&mut c2) {
                if matches!(child.kind(), "scoped_identifier" | "identifier")
                    && let Ok(text) = child.utf8_text(source)
                {
                    let last = text.rsplit('.').next().unwrap_or(text).trim();
                    if !last.is_empty() && last != "*" {
                        refs.push(import_ref(last.to_string(), line));
                    }
                }
            }
        }
    }
}

/// C# `using System.Text;` → `Text`; `using Foo = A.B;` → `B`.
struct CSharpImports;
impl ImportExtractor for CSharpImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() != "using_directive" {
                continue;
            }
            let line = node_line(&node);
            let mut c2 = node.walk();
            for child in node.children(&mut c2) {
                if matches!(
                    child.kind(),
                    "qualified_name" | "identifier" | "name_equals"
                ) && let Ok(text) = child.utf8_text(source)
                {
                    let last = text.rsplit(['.', '=']).next().unwrap_or(text).trim();
                    if !last.is_empty() {
                        refs.push(import_ref(last.to_string(), line));
                    }
                }
            }
        }
    }
}

/// Swift `import Foundation` → `Foundation`; `import struct A.B` → `B`.
struct SwiftImports;
impl ImportExtractor for SwiftImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.kind() != "import_declaration" {
                continue;
            }
            let line = node_line(&node);
            let mut c2 = node.walk();
            for child in node.children(&mut c2) {
                if matches!(child.kind(), "identifier" | "qualified_name")
                    && let Ok(text) = child.utf8_text(source)
                {
                    let last = text.rsplit('.').next().unwrap_or(text).trim();
                    if !last.is_empty() {
                        refs.push(import_ref(last.to_string(), line));
                    }
                }
            }
        }
    }
}

/// Ruby has no `import` — dependencies come from `require`/
/// `require_relative`/`load`/`autoload` calls. The ref target is the
/// required file's stem: `require 'foo/bar'` → `bar`.
struct RubyImports;
impl RubyImports {
    fn collect(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        if node.kind() == "call"
            && let Some(method) = node.child_by_field_name("method")
            && let Ok(m) = method.utf8_text(source)
            && matches!(
                m.trim(),
                "require" | "require_relative" | "load" | "autoload"
            )
            && let Some(args) = node.child_by_field_name("arguments")
        {
            let line = node_line(node);
            let mut c = args.walk();
            for arg in args.children(&mut c) {
                if arg.kind() == "string"
                    && let Ok(raw) = arg.utf8_text(source)
                {
                    let path = raw.trim_matches(['"', '\'']).trim();
                    let stem = path
                        .rsplit('/')
                        .next()
                        .unwrap_or(path)
                        .trim_end_matches(".rb");
                    if !stem.is_empty() {
                        refs.push(import_ref(stem.to_string(), line));
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::collect(&child, source, refs);
        }
    }
}
impl ImportExtractor for RubyImports {
    fn walk_imports(&self, root: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
        Self::collect(root, source, refs);
    }
}

fn extract_refs_php(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    // Imports from the root walker; the call/heritage/type graph from a
    // full-tree walk (mirrors Java/C#) — PHP is import-only no more.
    let mut refs = extract_imports(tree, source, &PhpImports);
    walk_php_refs(&tree.root_node(), source, &mut refs);
    refs
}

/// Recursively emit `Calls`/`Implements`/`Uses` edges for PHP. Complements
/// [`PhpImports`] (which handles `use`):
///
/// - `class_declaration` / `interface_declaration` / `trait_declaration` /
///   `enum_declaration` → `Implements` for every type in a `base_clause`
///   (`extends`) and a `class_interface_clause` (`implements`).
/// - `function_call_expression` / `member_call_expression` /
///   `scoped_call_expression` → `Calls` (the callee / method `name`).
/// - `object_creation_expression` (`new T()`) + `named_type` annotations
///   (property / parameter / return) → `Uses`. PHP scalar hints are
///   `primitive_type` nodes, so they are naturally skipped.
///
/// `from_context` is `None`; the line-based resolver attributes each edge to
/// its enclosing symbol.
fn walk_php_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "class_declaration"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration" => php_heritage(node, source, refs),
        "function_call_expression" => {
            if let Some(f) = node.child_by_field_name("function")
                && let Some(name) = php_type_name(&f, source)
            {
                push_graph_ref(refs, name, RefKind::Calls, node_line(node));
            }
        }
        "member_call_expression" | "scoped_call_expression" | "nullsafe_member_call_expression" => {
            if let Some(n) = node.child_by_field_name("name")
                && let Some(name) = php_type_name(&n, source)
            {
                push_graph_ref(refs, name, RefKind::Calls, node_line(node));
            }
        }
        "object_creation_expression" => {
            // `new Foo()` — the constructed type is the first name child.
            let mut cursor = node.walk();
            if let Some(name) = node
                .children(&mut cursor)
                .find(|c| matches!(c.kind(), "name" | "qualified_name"))
                .and_then(|c| php_type_name(&c, source))
            {
                push_graph_ref(refs, name, RefKind::Uses, node_line(node));
            }
        }
        "named_type" => {
            if let Some(name) = php_type_name(node, source) {
                push_graph_ref(refs, name, RefKind::Uses, node_line(node));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php_refs(&child, source, refs);
    }
}

/// Emit `Implements` for every type in a declaration's `base_clause`
/// (`extends`) and `class_interface_clause` (`implements`).
fn php_heritage(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let mut cursor = node.walk();
    for clause in node.children(&mut cursor) {
        if !matches!(clause.kind(), "base_clause" | "class_interface_clause") {
            continue;
        }
        let mut cc = clause.walk();
        for t in clause.children(&mut cc) {
            if let Some(name) = php_type_name(&t, source) {
                push_graph_ref(refs, name, RefKind::Implements, node_line(&clause));
            }
        }
    }
}

/// The head type name of a PHP type node: a bare `name`, the final segment of
/// a `qualified_name` (`\App\Base` → `Base`), or the inner name of a
/// `named_type` wrapper.
fn php_type_name<'a>(node: &tree_sitter::Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    match node.kind() {
        "name" => node.utf8_text(source).ok(),
        "qualified_name" | "named_type" => {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .filter(|c| c.kind() == "name")
                .last()
                .and_then(|c| c.utf8_text(source).ok())
        }
        _ => None,
    }
}

fn extract_refs_kotlin(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    // Imports from the root walker; the call/heritage/type graph from a
    // full-tree walk (mirrors Java/C#) — Kotlin is import-only no more.
    let mut refs = extract_imports(tree, source, &KotlinImports);
    walk_kotlin_refs(&tree.root_node(), source, &mut refs);
    refs
}

/// Recursively emit `Calls`/`Implements`/`Uses` edges for Kotlin. Complements
/// [`KotlinImports`] (which handles `import`):
///
/// - `delegation_specifier` (the supertype list of a class/interface) →
///   `Implements` for the named supertype, whether a bare `user_type`
///   (interface / plain base) or a `constructor_invocation` (`Base()`).
/// - `call_expression` → `Calls` (callee `identifier`, or the final segment
///   of a `navigation_expression` receiver chain). Kotlin has no `new`, so a
///   constructor call `Widget()` surfaces here as `Calls(Widget)`.
/// - `user_type` in any type position (parameters, properties, return types,
///   generics) → `Uses`; heritage `user_type`s are skipped here since they
///   are already emitted as `Implements`.
///
/// `from_context` is `None`; the line-based resolver attributes each edge to
/// its enclosing symbol.
fn walk_kotlin_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "delegation_specifier" => {
            if let Some(ut) = kotlin_specifier_user_type(node)
                && let Some(name) = kotlin_user_type_head(&ut, source)
            {
                push_graph_ref(refs, name, RefKind::Implements, node_line(node));
            }
        }
        "call_expression" => kotlin_call(node, source, refs),
        "user_type" => {
            // Skip heritage user_types (already emitted as Implements via the
            // enclosing delegation_specifier / constructor_invocation).
            let in_heritage = node.parent().is_some_and(|p| {
                matches!(p.kind(), "delegation_specifier" | "constructor_invocation")
            });
            if !in_heritage && let Some(name) = kotlin_user_type_head(node, source) {
                push_graph_ref(refs, name, RefKind::Uses, node_line(node));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_kotlin_refs(&child, source, refs);
    }
}

/// The head type name of a `user_type` (its first `identifier` child), e.g.
/// `List<Foo>` → `List`. Nested generic arguments are separate `user_type`
/// nodes handled by the recursion.
fn kotlin_user_type_head<'a>(node: &tree_sitter::Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|c| c.kind() == "identifier")
        .and_then(|c| c.utf8_text(source).ok())
}

/// Find the supertype `user_type` of a `delegation_specifier`: either a direct
/// `user_type` child, or the one inside a `constructor_invocation` (`Base()`).
fn kotlin_specifier_user_type<'t>(node: &tree_sitter::Node<'t>) -> Option<tree_sitter::Node<'t>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "user_type" => return Some(child),
            "constructor_invocation" => {
                let mut cc = child.walk();
                if let Some(ut) = child.children(&mut cc).find(|g| g.kind() == "user_type") {
                    return Some(ut);
                }
            }
            _ => {}
        }
    }
    None
}

/// Emit a `Calls` edge from a `call_expression`: callee `identifier` (`foo()`,
/// `Widget()`), or the final `identifier` of a `navigation_expression`
/// receiver chain (`a.b.foo()` → `foo`).
fn kotlin_call(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let Some(callee) = node.child(0) else {
        return;
    };
    let name = match callee.kind() {
        "identifier" => callee.utf8_text(source).ok(),
        "navigation_expression" => {
            let mut cursor = callee.walk();
            callee
                .children(&mut cursor)
                .filter(|c| c.kind() == "identifier")
                .last()
                .and_then(|c| c.utf8_text(source).ok())
        }
        _ => None,
    };
    if let Some(name) = name {
        push_graph_ref(refs, name, RefKind::Calls, node_line(node));
    }
}

fn extract_refs_scala(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    extract_imports(tree, source, &ScalaImports)
}
fn extract_refs_java(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    // Imports from the root-level walker; the call/heritage/type graph from a
    // full-tree walk (mirrors the JS/TS family) — Java is import-only no more.
    let mut refs = extract_imports(tree, source, &JavaImports);
    walk_java_refs(&tree.root_node(), source, &mut refs);
    refs
}

/// Recursively emit `Calls`/`Implements`/`Uses` edges for Java. Complements
/// [`JavaImports`] (which handles `import`):
///
/// - `class_declaration` → `Implements` for the `extends` superclass and each
///   `implements` interface; `interface_declaration extends` likewise.
/// - `method_invocation` → `Calls` (the `[name]` method, for both `foo()` and
///   `obj.foo()`).
/// - `object_creation_expression` (`new T()`) + every declared `[type]`
///   position (fields, parameters, locals, return types) → `Uses`.
///
/// `from_context` is `None`; the line-based resolver attributes each edge to
/// its enclosing symbol. Unresolved targets (JDK types, host methods) never
/// bind to an in-corpus symbol.
fn walk_java_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "class_declaration" => java_class_heritage(node, source, refs),
        "interface_declaration" => java_interface_heritage(node, source, refs),
        "method_invocation" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
            {
                push_graph_ref(refs, name, RefKind::Calls, node_line(node));
            }
        }
        "object_creation_expression"
        | "field_declaration"
        | "formal_parameter"
        | "local_variable_declaration"
        | "method_declaration" => {
            if let Some(ty) = node.child_by_field_name("type") {
                java_collect_type_uses(&ty, source, refs);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_java_refs(&child, source, refs);
    }
}

/// Emit `Implements` from a `class_declaration`'s `superclass` (`extends`) and
/// `super_interfaces` (`implements`) fields.
fn java_class_heritage(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    if let Some(sc) = node.child_by_field_name("superclass") {
        java_collect_implements(&sc, source, refs);
    }
    if let Some(ifaces) = node.child_by_field_name("interfaces") {
        java_collect_implements(&ifaces, source, refs);
    }
}

/// Emit `Implements` from an `interface_declaration`'s `extends_interfaces`.
fn java_interface_heritage(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "extends_interfaces" {
            java_collect_implements(&child, source, refs);
        }
    }
}

/// Recursively emit `Implements` for every `type_identifier` (and the final
/// segment of a `scoped_type_identifier`) within a heritage subtree.
fn java_collect_implements(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    java_collect_type_names(node, source, refs, RefKind::Implements);
}

/// Recursively emit `Uses` for every named type within a type subtree
/// (handles `generic_type` arguments and `scoped_type_identifier`).
fn java_collect_type_uses(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    java_collect_type_names(node, source, refs, RefKind::Uses);
}

/// Shared type-name collector: walks a type/heritage subtree and pushes a ref
/// of `kind` for each named type. `scoped_type_identifier` contributes only
/// its final `name` segment (not the package path).
fn java_collect_type_names(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    refs: &mut Vec<RawRef>,
    kind: RefKind,
) {
    match node.kind() {
        "type_identifier" => {
            if let Ok(name) = node.utf8_text(source) {
                push_graph_ref(refs, name, kind, node_line(node));
            }
            return;
        }
        "scoped_type_identifier" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
            {
                push_graph_ref(refs, name, kind, node_line(node));
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        java_collect_type_names(&child, source, refs, kind);
    }
}

fn extract_refs_csharp(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    // Imports from the root walker; the call/heritage/type graph from a
    // full-tree walk (mirrors Java) — C# is import-only no more.
    let mut refs = extract_imports(tree, source, &CSharpImports);
    walk_csharp_refs(&tree.root_node(), source, &mut refs);
    refs
}

/// Recursively emit `Calls`/`Implements`/`Uses` edges for C#. Complements
/// [`CSharpImports`] (which handles `using`):
///
/// - `class_declaration` / `interface_declaration` / `struct_declaration` /
///   `record_declaration` → `Implements` for every `base_list` entry (C#
///   merges the base class and interfaces into one list — all are emitted).
/// - `invocation_expression` → `Calls` (callee `identifier`, or the `[name]`
///   of a `member_access_expression`).
/// - `object_creation_expression` (`new T()`) + every declared `[type]` /
///   `[returns]` position (fields, locals, parameters, properties, return
///   types) → `Uses`.
///
/// `from_context` is `None`; the line-based resolver attributes each edge to
/// its enclosing symbol. C# type positions hold a bare `identifier`, so the
/// walker only reads the explicit type/heritage fields (never a free
/// identifier, which would also match locals and method names).
fn walk_csharp_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "class_declaration"
        | "interface_declaration"
        | "struct_declaration"
        | "record_declaration"
        | "record_struct_declaration" => {
            csharp_heritage(node, source, refs);
        }
        "invocation_expression" => csharp_call(node, source, refs),
        "object_creation_expression"
        | "variable_declaration"
        | "parameter"
        | "property_declaration" => {
            if let Some(ty) = node.child_by_field_name("type") {
                csharp_collect_type_uses(&ty, source, refs);
            }
        }
        "method_declaration" => {
            if let Some(ty) = node.child_by_field_name("returns") {
                csharp_collect_type_uses(&ty, source, refs);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_csharp_refs(&child, source, refs);
    }
}

/// Emit an `Implements` edge for each entry of a type's `base_list`.
fn csharp_heritage(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "base_list" {
            continue;
        }
        let mut bc = child.walk();
        for base in child.children(&mut bc) {
            if let Some(name) = csharp_type_head_name(&base, source) {
                push_graph_ref(refs, name, RefKind::Implements, node_line(&child));
            }
        }
    }
}

/// Emit a `Calls` edge from an `invocation_expression`'s callee: a bare
/// `identifier` (`Foo()`) or the `[name]` of a `member_access_expression`
/// (`obj.Foo()`).
fn csharp_call(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let Some(func) = node.child_by_field_name("function") else {
        return;
    };
    let callee = match func.kind() {
        "identifier" => func.utf8_text(source).ok(),
        "member_access_expression" => func
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok()),
        _ => None,
    };
    if let Some(name) = callee {
        push_graph_ref(refs, name, RefKind::Calls, node_line(node));
    }
}

/// The principal (head) type name of a heritage/type node: an `identifier`
/// directly, the leading name of a `generic_name` (`IList<T>` → `IList`), or
/// the final segment of a `qualified_name`. Punctuation children return
/// `None`.
fn csharp_type_head_name<'a>(node: &tree_sitter::Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    match node.kind() {
        "identifier" => node.utf8_text(source).ok(),
        "generic_name" => {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|c| c.kind() == "identifier")
                .and_then(|c| c.utf8_text(source).ok())
        }
        "qualified_name" => node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok()),
        _ => None,
    }
}

/// Emit `Uses` for every named type within a type subtree (the head type plus
/// any generic arguments / array element / nullable inner type). `var`
/// (`implicit_type`) and predefined types carry no `identifier`, so they are
/// naturally skipped.
fn csharp_collect_type_uses(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "identifier" => {
            if let Ok(name) = node.utf8_text(source) {
                push_graph_ref(refs, name, RefKind::Uses, node_line(node));
            }
            return;
        }
        "qualified_name" => {
            // Emit only the final segment; the qualifier path isn't a symbol.
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
            {
                push_graph_ref(refs, name, RefKind::Uses, node_line(node));
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        csharp_collect_type_uses(&child, source, refs);
    }
}

fn extract_refs_swift(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    // Imports from the root walker; the call/heritage/type graph from a
    // full-tree walk (mirrors Kotlin) — Swift is import-only no more.
    let mut refs = extract_imports(tree, source, &SwiftImports);
    walk_swift_refs(&tree.root_node(), source, &mut refs);
    refs
}

/// Recursively emit `Calls`/`Implements`/`Uses` edges for Swift. Complements
/// [`SwiftImports`] (which handles `import`):
///
/// - `inheritance_specifier` (a class/protocol/struct supertype entry) →
///   `Implements` for the inherited `user_type` (Swift lists the base class
///   and conformed protocols together).
/// - `call_expression` → `Calls` (callee `simple_identifier`, or the final
///   `navigation_suffix` name of a `navigation_expression`). Swift has no
///   `new`, so a constructor call `Widget()` surfaces as `Calls(Widget)`.
/// - `user_type` in any type position (annotations, parameters, return types)
///   → `Uses`; heritage `user_type`s are skipped (already `Implements`).
///
/// `from_context` is `None`; the line-based resolver attributes each edge to
/// its enclosing symbol. Type references are wrapped in `user_type`, while a
/// declaration's own name is a bare `type_identifier`, so matching only
/// `user_type` avoids emitting the declared name as a use.
fn walk_swift_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "inheritance_specifier" => {
            if let Some(ut) = node.child_by_field_name("inherits_from")
                && let Some(name) = swift_user_type_head(&ut, source)
            {
                push_graph_ref(refs, name, RefKind::Implements, node_line(node));
            }
        }
        "call_expression" => swift_call(node, source, refs),
        "user_type" => {
            let in_heritage = node
                .parent()
                .is_some_and(|p| p.kind() == "inheritance_specifier");
            if !in_heritage && let Some(name) = swift_user_type_head(node, source) {
                push_graph_ref(refs, name, RefKind::Uses, node_line(node));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_swift_refs(&child, source, refs);
    }
}

/// The head type name of a `user_type` (its `type_identifier` child), e.g.
/// `Array<Foo>` → `Array`. Nested generic arguments are separate `user_type`
/// nodes handled by the recursion.
fn swift_user_type_head<'a>(node: &tree_sitter::Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|c| c.kind() == "type_identifier")
        .and_then(|c| c.utf8_text(source).ok())
}

/// Emit a `Calls` edge from a `call_expression`: callee `simple_identifier`
/// (`foo()`, `Widget()`), or the final `navigation_suffix` name of a
/// `navigation_expression` receiver chain (`a.b.foo()` → `foo`).
fn swift_call(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let Some(callee) = node.child(0) else {
        return;
    };
    let name = match callee.kind() {
        "simple_identifier" => callee.utf8_text(source).ok(),
        "navigation_expression" => callee
            .child_by_field_name("suffix")
            .and_then(|s| s.child_by_field_name("suffix"))
            .and_then(|n| n.utf8_text(source).ok()),
        _ => None,
    };
    if let Some(name) = name {
        push_graph_ref(refs, name, RefKind::Calls, node_line(node));
    }
}

fn extract_refs_ruby(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    // Imports (`require` stems) from the root walker; the call/heritage graph
    // from a full-tree walk — Ruby is import-only no more.
    let mut refs = extract_imports(tree, source, &RubyImports);
    walk_ruby_refs(&tree.root_node(), source, &mut refs);
    refs
}

/// Recursively emit `Calls`/`Implements`/`Uses` edges for Ruby. Complements
/// [`RubyImports`] (which handles `require`/`require_relative`/`load`/`autoload`):
///
/// - `class` superclass (`class C < Base`) → `Implements(Base)`.
/// - `include` / `prepend` / `extend` calls (Ruby's module-mixin mechanism —
///   the de-facto interface) → `Implements` for each module argument.
/// - other `call`s → `Calls` (the message `method` name). Ruby has no `new`
///   keyword; `Widget.new` is a `.new` call whose receiver constant is the
///   constructed class → `Uses(Widget)` (the `new` message itself is dropped).
/// - `require`-family calls are skipped here (already handled as imports).
///
/// Ruby has no static type annotations, so `Uses` comes only from `.new`.
/// `from_context` is `None`; the line-based resolver attributes each edge to
/// its enclosing symbol.
fn walk_ruby_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "class" => {
            if let Some(sc) = node.child_by_field_name("superclass") {
                let mut cursor = sc.walk();
                for child in sc.children(&mut cursor) {
                    if let Some(name) = ruby_const_name(&child, source) {
                        push_graph_ref(refs, name, RefKind::Implements, node_line(&sc));
                    }
                }
            }
        }
        "call" => ruby_call(node, source, refs),
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_ruby_refs(&child, source, refs);
    }
}

/// Handle a Ruby `call`: mixin (`include`/`prepend`/`extend`) → `Implements`;
/// `Constant.new` → `Uses(Constant)`; `require`-family → skipped (imports);
/// everything else → `Calls(method)`.
fn ruby_call(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let Some(method) = node
        .child_by_field_name("method")
        .filter(|m| m.kind() == "identifier")
        .and_then(|m| m.utf8_text(source).ok())
    else {
        return;
    };
    match method {
        "include" | "prepend" | "extend" => {
            if let Some(args) = node.child_by_field_name("arguments") {
                let mut cursor = args.walk();
                for arg in args.children(&mut cursor) {
                    if let Some(name) = ruby_const_name(&arg, source) {
                        push_graph_ref(refs, name, RefKind::Implements, node_line(node));
                    }
                }
            }
        }
        "require" | "require_relative" | "load" | "autoload" => {
            // Imports — already emitted by `RubyImports`.
        }
        "new" => {
            if let Some(recv) = node.child_by_field_name("receiver")
                && let Some(name) = ruby_const_name(&recv, source)
            {
                push_graph_ref(refs, name, RefKind::Uses, node_line(node));
            }
        }
        other => push_graph_ref(refs, other, RefKind::Calls, node_line(node)),
    }
}

/// Resolve a Ruby constant reference: a bare `constant`, or the final `name`
/// segment of a `scope_resolution` (`A::B` → `B`).
fn ruby_const_name<'a>(node: &tree_sitter::Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    match node.kind() {
        "constant" => node.utf8_text(source).ok(),
        "scope_resolution" => node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok()),
        _ => None,
    }
}

#[cfg(test)]
mod new_lang_import_tests {
    use super::*;
    use crate::code::GrammarRegistry;

    fn parse(lang: &str, src: &str) -> tree_sitter::Tree {
        let l = GrammarRegistry::global()
            .language_by_name(lang)
            .expect("registered");
        let mut p = tree_sitter::Parser::new();
        p.set_language(l).unwrap();
        p.parse(src, None).unwrap()
    }

    #[test]
    fn php_use_imports() {
        let src = "<?php\nnamespace App;\nuse Foo\\Bar;\nuse Foo\\Baz as Q;\n";
        let t = parse("php", src);
        let names: Vec<_> = extract_refs_php(&t, src.as_bytes())
            .into_iter()
            .map(|r| r.target_name)
            .collect();
        assert!(names.contains(&"Bar".to_string()), "got {names:?}");
        assert!(names.contains(&"Baz".to_string()), "got {names:?}");
    }

    #[test]
    fn kotlin_imports() {
        let src = "package a.b\nimport com.x.Y\nimport com.x.Z as W\n";
        let t = parse("kotlin", src);
        let names: Vec<_> = extract_refs_kotlin(&t, src.as_bytes())
            .into_iter()
            .map(|r| r.target_name)
            .collect();
        assert!(names.contains(&"Y".to_string()), "got {names:?}");
        assert!(names.contains(&"Z".to_string()), "got {names:?}");
    }

    #[test]
    fn scala_imports() {
        let src = "package a.b\nimport com.x.Y\nimport com.x.{A, B}\n";
        let t = parse("scala", src);
        let names: Vec<_> = extract_refs_scala(&t, src.as_bytes())
            .into_iter()
            .map(|r| r.target_name)
            .collect();
        assert!(names.contains(&"Y".to_string()), "got {names:?}");
        assert!(names.contains(&"A".to_string()), "got {names:?}");
        assert!(names.contains(&"B".to_string()), "got {names:?}");
    }

    #[test]
    fn java_imports() {
        let src = "package a.b;\nimport com.x.Y;\nimport com.x.*;\n";
        let t = parse("java", src);
        let names: Vec<_> = extract_refs_java(&t, src.as_bytes())
            .into_iter()
            .map(|r| r.target_name)
            .collect();
        assert!(names.contains(&"Y".to_string()), "got {names:?}");
        assert!(!names.contains(&"*".to_string()), "got {names:?}");
    }

    #[test]
    fn csharp_imports() {
        let src = "using System.Text;\nusing Foo = A.B;\n";
        let t = parse("csharp", src);
        let names: Vec<_> = extract_refs_csharp(&t, src.as_bytes())
            .into_iter()
            .map(|r| r.target_name)
            .collect();
        assert!(names.contains(&"Text".to_string()), "got {names:?}");
        assert!(names.contains(&"B".to_string()), "got {names:?}");
    }

    #[test]
    fn swift_imports() {
        let src = "import Foundation\nimport struct A.B\n";
        let t = parse("swift", src);
        let names: Vec<_> = extract_refs_swift(&t, src.as_bytes())
            .into_iter()
            .map(|r| r.target_name)
            .collect();
        assert!(names.contains(&"Foundation".to_string()), "got {names:?}");
    }

    #[test]
    fn ruby_requires() {
        let src = "require 'foo/bar'\nrequire_relative 'baz'\n";
        let t = parse("ruby", src);
        let names: Vec<_> = extract_refs_ruby(&t, src.as_bytes())
            .into_iter()
            .map(|r| r.target_name)
            .collect();
        assert!(names.contains(&"bar".to_string()), "got {names:?}");
        assert!(names.contains(&"baz".to_string()), "got {names:?}");
    }
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

/// Extract cross-references from C / C++ source.
///
/// Walks the tree recursively (refs can sit inside `preproc_ifdef` header
/// guards, `extern "C"` linkage_specification blocks, namespaces, classes,
/// and function bodies) collecting three kinds of reference:
///
/// - `preproc_include` → `Imports` whose target is the include path with
///   brackets/quotes stripped (`<stdio.h>` → `stdio.h`, `"foo/bar.h"` →
///   `foo/bar.h`).
/// - `call_expression` → `Calls` on the callee name (`add(...)` →
///   `Calls(add)`, `obj.method(...)`/`obj->method(...)` → `Calls(method)`,
///   `Ns::fn(...)` → `Calls(fn)`).
/// - `type_identifier` → `Uses` on the named type (`Point p;` → `Uses(Point)`).
///   Primitive types are `primitive_type` nodes, so they are naturally
///   excluded.
///
/// `#include` alone never resolves to a symbol (its target is a file path),
/// so the call/use refs are what give C/C++ real cross-file symbol edges,
/// resolved by name against the unified c/cpp ref family.
fn extract_refs_c_cpp(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawRef> {
    let mut refs = Vec::new();
    walk_c_refs(&tree.root_node(), source, &mut refs);
    refs
}

fn walk_c_refs(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    match node.kind() {
        "preproc_include" => {
            if let Some(path) = extract_c_include_path(node, source) {
                refs.push(import_ref(path, node_line(node)));
            }
        }
        "call_expression" => extract_c_call_ref(node, source, refs),
        "type_identifier" => {
            if let Ok(name) = node.utf8_text(source) {
                let name = name.trim();
                if !name.is_empty() {
                    refs.push(c_use_ref(name.to_string(), node_line(node)));
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_c_refs(&child, source, refs);
    }
}

/// A `Uses` ref with no `from_context` — attributed to the enclosing symbol
/// by line during resolution.
fn c_use_ref(name: String, line: u32) -> RawRef {
    RawRef {
        target_name: name,
        kind: RefKind::Uses,
        line,
        from_context: None,
        target_crate: None,
    }
}

/// Extract a `Calls` ref from a C/C++ `call_expression`'s callee.
fn extract_c_call_ref(node: &tree_sitter::Node<'_>, source: &[u8], refs: &mut Vec<RawRef>) {
    let Some(func) = node.child_by_field_name("function") else {
        return;
    };
    let line = node_line(node);
    let name = match func.kind() {
        "identifier" => func.utf8_text(source).ok(),
        // `obj.method()` / `obj->method()` — the `field` is the method name.
        "field_expression" => func
            .child_by_field_name("field")
            .and_then(|f| f.utf8_text(source).ok()),
        // `Ns::fn()` / `Type::method()` — the `name` field is the callee.
        "qualified_identifier" => func
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok()),
        _ => None,
    };
    if let Some(name) = name {
        let name = name.trim();
        if !name.is_empty() {
            refs.push(RawRef {
                target_name: name.to_string(),
                kind: RefKind::Calls,
                line,
                from_context: None,
                target_crate: None,
            });
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
        // Stdlib types (`Vec`) are now in the PRIMITIVE_TYPES denylist —
        // emitting `Uses(Vec)` causes phantom cross-crate bindings since
        // stdlib isn't indexed. The extraction mechanic for *user* types
        // is what this test verifies.
        let source = r"
fn process(config: MyConfig, items: MyVec<MyItem>) {}
";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        let names: Vec<&str> = uses.iter().map(|r| r.target_name.as_str()).collect();
        assert!(
            names.contains(&"MyConfig"),
            "missing MyConfig, got: {names:?}"
        );
        assert!(names.contains(&"MyVec"), "missing MyVec, got: {names:?}");
        assert!(names.contains(&"MyItem"), "missing MyItem, got: {names:?}");
    }

    #[test]
    fn extract_return_type() {
        // `Result` is intentionally filtered — same reasoning as above.
        let source = r"
fn create() -> MyResult<MyConfig, MyError> { todo!() }
";
        let refs = parse_and_extract(source);
        let uses: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Uses).collect();
        let names: Vec<&str> = uses.iter().map(|r| r.target_name.as_str()).collect();
        assert!(
            names.contains(&"MyResult"),
            "missing MyResult, got: {names:?}"
        );
        assert!(
            names.contains(&"MyConfig"),
            "missing MyConfig, got: {names:?}"
        );
        assert!(
            names.contains(&"MyError"),
            "missing MyError, got: {names:?}"
        );
    }

    #[test]
    fn stdlib_names_are_filtered() {
        // Regression: `Result`, `Vec`, `Option`, `Command`, `Box`, etc.
        // are denylisted to prevent phantom cross-crate bindings when a
        // user crate happens to define a same-named type.
        let source = r"
use std::process::Command;
fn run() -> Result<Vec<Option<Command>>, Box<dyn std::error::Error>> { todo!() }
";
        let refs = parse_and_extract(source);
        let names: Vec<&str> = refs
            .iter()
            .filter(|r| r.kind == RefKind::Uses)
            .map(|r| r.target_name.as_str())
            .collect();
        for stdlib_name in ["Result", "Vec", "Option", "Command", "Box"] {
            assert!(
                !names.contains(&stdlib_name),
                "stdlib name {stdlib_name} should be filtered, got: {names:?}"
            );
        }
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
                .set_language(&tree_sitter_unreal_cpp::LANGUAGE.into())
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

        /// Smoke check that UE reflection macros parse cleanly under
        /// the unreal-cpp grammar. Vanilla `tree-sitter-cpp` would
        /// blow this up into a sea of ERROR nodes; the unreal-cpp
        /// grammar treats `UCLASS()`, `GENERATED_BODY()`,
        /// `UFUNCTION()`, `UPROPERTY()` as first-class syntax.
        #[test]
        fn unreal_macros_parse_without_error() {
            let src = r#"
#include "CoreMinimal.h"
#include "GameFramework/Actor.h"

UCLASS(Blueprintable)
class MYGAME_API AMyActor : public AActor {
    GENERATED_BODY()
public:
    UFUNCTION(BlueprintCallable, Category="Test")
    void DoThing();

    UPROPERTY(EditAnywhere, BlueprintReadWrite)
    int32 Counter;
};
"#;
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_unreal_cpp::LANGUAGE.into())
                .unwrap();
            let tree = parser.parse(src.as_bytes(), None).unwrap();
            assert!(
                !tree.root_node().has_error(),
                "tree-sitter-unreal-cpp should parse UE reflection macros cleanly"
            );

            // The two #includes must still resolve as refs — confirms
            // the existing C/C++ extractor is compatible with the new
            // grammar's parse tree.
            let refs = extract_refs(&tree, src.as_bytes(), "cpp");
            let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
            assert!(names.contains(&"CoreMinimal.h"), "got: {names:?}");
            assert!(names.contains(&"GameFramework/Actor.h"), "got: {names:?}");
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
