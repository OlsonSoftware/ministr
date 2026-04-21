//! Tauri bridge extractors for command and event bindings.
//!
//! Detects cross-language bridges in Tauri applications:
//!
//! - [`TauriCommandExtractor`] — `#[tauri::command]` exports in Rust ↔ `invoke("name")`
//!   imports in JS/TS
//! - [`TauriEventExtractor`] — `emit`/`listen` patterns across Rust and JS/TS
//!
//! Both implement [`BridgeExtractor`] and can be registered with a
//! [`BridgeLinker`](super::linker::BridgeLinker).
//!
//! # Command registration validation
//!
//! [`extract_registered_commands`] parses `tauri::generate_handler![]` macro
//! invocations, and [`boost_registered_commands`] promotes matching command
//! endpoints to [`ConfidenceLevel::RegistrationValidated`].

use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

// ---------------------------------------------------------------------------
// TauriCommandExtractor
// ---------------------------------------------------------------------------

/// Extracts Tauri command bindings from Rust and JS/TS source files.
///
/// **Rust exports** — functions annotated with `#[tauri::command]`:
/// ```rust,ignore
/// #[tauri::command]
/// fn greet(name: &str) -> String { format!("Hello, {name}!") }
/// ```
///
/// **JS/TS imports** — `invoke("command_name")` calls:
/// ```javascript,ignore
/// const result = await invoke("greet", { name: "World" });
/// ```
///
/// The binding key is the command name string, which must match exactly
/// between the Rust function name and the JS `invoke` argument.
pub struct TauriCommandExtractor;

impl BridgeExtractor for TauriCommandExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::TauriCommand
    }

    fn applicable_languages(&self) -> &[&str] {
        &["rust", "javascript", "typescript"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        match language {
            "rust" => extract_rust_command_exports(tree, source, file_path),
            "javascript" | "typescript" => {
                extract_js_command_imports(tree, source, file_path, language)
            }
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// TauriEventExtractor
// ---------------------------------------------------------------------------

/// Extracts Tauri event bindings from Rust and JS/TS source files.
///
/// **Rust side**:
/// - `app.emit("event", payload)` / `emit_to` / `emit_filter` → Export
/// - `app.listen("event", handler)` / `once` → Import
///
/// **JS/TS side**:
/// - `emit("event", payload)` → Export
/// - `listen("event", handler)` / `once("event", handler)` → Import
///
/// The binding key is the event name string literal.
pub struct TauriEventExtractor;

impl BridgeExtractor for TauriEventExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::TauriEvent
    }

    fn applicable_languages(&self) -> &[&str] {
        &["rust", "javascript", "typescript"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        match language {
            "rust" => extract_rust_events(tree, source, file_path),
            "javascript" | "typescript" => extract_js_events(tree, source, file_path, language),
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rust command extraction
// ---------------------------------------------------------------------------

/// Find `#[tauri::command]` annotated functions and produce Export endpoints.
fn extract_rust_command_exports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_rust_commands(&mut cursor, source, file_path, &mut endpoints);
    endpoints
}

/// Recursively walk the tree looking for function items with `#[tauri::command]`.
///
/// In tree-sitter-rust, `#[tauri::command]` is an `attribute_item` that appears
/// as a **preceding sibling** of the `function_item`, not as a child.
fn walk_rust_commands(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();

        if (node.kind() == "function_item" || node.kind() == "function_definition")
            && has_tauri_command_attribute_before(&node, source)
            && let Some(name) = rust_function_name(&node, source)
        {
            #[allow(clippy::cast_possible_truncation)]
            let line = node.start_position().row as u32 + 1;
            endpoints.push(BridgeEndpoint {
                binding_key: name.clone(),
                kind: BridgeKind::TauriCommand,
                role: EndpointRole::Export,
                language: "rust".into(),
                file_path: file_path.into(),
                line,
                symbol_name: name,
                confidence: ConfidenceLevel::CaseTransformed.score(),
            });
        }

        // Recurse into children
        if cursor.goto_first_child() {
            walk_rust_commands(cursor, source, file_path, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Check whether the preceding sibling(s) of a function node contain
/// a `#[tauri::command]` attribute.
fn has_tauri_command_attribute_before(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() == "attribute_item" {
            let text = node_text(&sibling, source);
            if text.contains("tauri::command") {
                return true;
            }
        } else if sibling.kind() != "attribute_item"
            && sibling.kind() != "line_comment"
            && sibling.kind() != "block_comment"
        {
            // Stop searching once we hit a non-attribute, non-comment node
            break;
        }
        prev = sibling.prev_sibling();
    }
    false
}

/// Extract the function name from a `function_item` node.
fn rust_function_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "identifier" && cursor.field_name() == Some("name") {
            return Some(node_text(&child, source));
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// JS/TS command extraction
// ---------------------------------------------------------------------------

/// Find `invoke("command_name")` calls in JS/TS and produce Import endpoints.
fn extract_js_command_imports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_js_invoke_calls(&mut cursor, source, file_path, language, &mut endpoints);
    endpoints
}

/// Recursively walk looking for `invoke("...")` call expressions.
fn walk_js_invoke_calls(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();

        if node.kind() == "call_expression"
            && let Some(endpoint) = try_extract_invoke_call(&node, source, file_path, language)
        {
            endpoints.push(endpoint);
        }

        if cursor.goto_first_child() {
            walk_js_invoke_calls(cursor, source, file_path, language, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Try to extract an `invoke("name")` call from a `call_expression` node.
fn try_extract_invoke_call(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Option<BridgeEndpoint> {
    // Get the function being called
    let function_node = node.child_by_field_name("function")?;
    let fn_text = node_text(&function_node, source);

    // Match `invoke(...)`, `...invoke(...)`, `TAURI_INVOKE(...)`, etc.
    // Case-insensitive to support tauri-specta and other generated wrappers.
    let fn_lower = fn_text.to_ascii_lowercase();
    if !fn_lower.contains("invoke") {
        return None;
    }

    // Get the arguments
    let args_node = node.child_by_field_name("arguments")?;
    let first_arg = first_string_arg(&args_node, source)?;

    #[allow(clippy::cast_possible_truncation)]
    let line = node.start_position().row as u32 + 1;

    Some(BridgeEndpoint {
        binding_key: first_arg.clone(),
        kind: BridgeKind::TauriCommand,
        role: EndpointRole::Import,
        language: language.into(),
        file_path: file_path.into(),
        line,
        symbol_name: first_arg,
        confidence: ConfidenceLevel::Exact.score(),
    })
}

// ---------------------------------------------------------------------------
// Rust event extraction
// ---------------------------------------------------------------------------

/// Event method names and their corresponding roles.
const RUST_EVENT_EXPORTS: &[&str] = &["emit", "emit_to", "emit_filter"];
const RUST_EVENT_IMPORTS: &[&str] = &["listen", "once"];

/// Find `.emit()`/`.listen()` method calls in Rust source.
fn extract_rust_events(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_rust_events(&mut cursor, source, file_path, &mut endpoints);
    endpoints
}

/// Recursively walk looking for event-related method calls.
fn walk_rust_events(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();

        if node.kind() == "call_expression"
            && let Some(ep) = try_extract_rust_event_call(&node, source, file_path)
        {
            endpoints.push(ep);
        }

        if cursor.goto_first_child() {
            walk_rust_events(cursor, source, file_path, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Try to extract an event endpoint from a Rust method call.
fn try_extract_rust_event_call(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    file_path: &str,
) -> Option<BridgeEndpoint> {
    let function_node = node.child_by_field_name("function")?;

    // Must be a field expression: `something.emit(...)`.
    if function_node.kind() != "field_expression" {
        return None;
    }

    let field_node = function_node.child_by_field_name("field")?;
    let method_name = node_text(&field_node, source);

    let role = if RUST_EVENT_EXPORTS.contains(&method_name.as_str()) {
        EndpointRole::Export
    } else if RUST_EVENT_IMPORTS.contains(&method_name.as_str()) {
        EndpointRole::Import
    } else {
        return None;
    };

    let args_node = node.child_by_field_name("arguments")?;
    let event_name = first_rust_string_arg(&args_node, source)?;

    #[allow(clippy::cast_possible_truncation)]
    let line = node.start_position().row as u32 + 1;

    Some(BridgeEndpoint {
        binding_key: event_name.clone(),
        kind: BridgeKind::TauriEvent,
        role,
        language: "rust".into(),
        file_path: file_path.into(),
        line,
        symbol_name: event_name,
        confidence: ConfidenceLevel::Exact.score(),
    })
}

// ---------------------------------------------------------------------------
// JS/TS event extraction
// ---------------------------------------------------------------------------

/// JS/TS event function names classified by role.
const JS_EVENT_EXPORTS: &[&str] = &["emit", "emitTo"];
const JS_EVENT_IMPORTS: &[&str] = &["listen", "once"];

/// Find event-related function/method calls in JS/TS source.
fn extract_js_events(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk_js_events(&mut cursor, source, file_path, language, &mut endpoints);
    endpoints
}

/// Recursively walk looking for event-related calls in JS/TS.
fn walk_js_events(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
    endpoints: &mut Vec<BridgeEndpoint>,
) {
    loop {
        let node = cursor.node();

        if node.kind() == "call_expression"
            && let Some(ep) = try_extract_js_event_call(&node, source, file_path, language)
        {
            endpoints.push(ep);
        }

        if cursor.goto_first_child() {
            walk_js_events(cursor, source, file_path, language, endpoints);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Try to extract an event endpoint from a JS/TS function/method call.
fn try_extract_js_event_call(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Option<BridgeEndpoint> {
    let function_node = node.child_by_field_name("function")?;
    let fn_name = callable_name(&function_node, source)?;

    // Distinguish from invoke() — only match event functions
    let role = if JS_EVENT_EXPORTS.contains(&fn_name.as_str()) {
        EndpointRole::Export
    } else if JS_EVENT_IMPORTS.contains(&fn_name.as_str()) {
        EndpointRole::Import
    } else {
        return None;
    };

    let args_node = node.child_by_field_name("arguments")?;
    let event_name = first_string_arg(&args_node, source)?;

    #[allow(clippy::cast_possible_truncation)]
    let line = node.start_position().row as u32 + 1;

    Some(BridgeEndpoint {
        binding_key: event_name.clone(),
        kind: BridgeKind::TauriEvent,
        role,
        language: language.into(),
        file_path: file_path.into(),
        line,
        symbol_name: event_name,
        confidence: ConfidenceLevel::Exact.score(),
    })
}

// ---------------------------------------------------------------------------
// Command registration validation (generate_handler!)
// ---------------------------------------------------------------------------

/// Extract command names from `tauri::generate_handler![...]` macro invocations.
///
/// Scans a Rust source file's tree-sitter AST for macro calls matching
/// `generate_handler!` (with or without the `tauri::` path prefix) and
/// collects all identifiers inside the token tree.
///
/// # Returns
///
/// A list of command names found in all `generate_handler!` invocations.
#[must_use]
pub fn extract_registered_commands(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<String> {
    let mut commands = Vec::new();
    let mut cursor = tree.walk();
    walk_generate_handler(&mut cursor, source, &mut commands);
    commands
}

/// Recursively walk looking for `generate_handler!` macro invocations.
fn walk_generate_handler(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    source: &[u8],
    commands: &mut Vec<String>,
) {
    loop {
        let node = cursor.node();

        if node.kind() == "macro_invocation"
            && let Some(macro_node) = node.child_by_field_name("macro")
        {
            let macro_text = node_text(&macro_node, source);
            if macro_text == "generate_handler" || macro_text.ends_with("::generate_handler") {
                // Collect identifiers from the token tree
                collect_token_tree_identifiers(&node, source, commands);
            }
        }

        if cursor.goto_first_child() {
            walk_generate_handler(cursor, source, commands);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Collect all identifiers from the token tree of a macro invocation.
fn collect_token_tree_identifiers(
    macro_node: &tree_sitter::Node<'_>,
    source: &[u8],
    commands: &mut Vec<String>,
) {
    let mut cursor = macro_node.walk();
    if !cursor.goto_first_child() {
        return;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "token_tree" {
            collect_idents_from_token_tree(&child, source, commands);
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Collect command identifiers from within a token tree.
///
/// In tree-sitter-rust, macro token trees are **flat** — `commands::greet`
/// appears as three tokens: `identifier("commands")`, `::`, `identifier("greet")`.
/// We handle both plain identifiers and `path::name` patterns by peeking
/// ahead for `::` separators.
fn collect_idents_from_token_tree(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    commands: &mut Vec<String>,
) {
    // Collect all children into a vec for lookahead
    let mut children = Vec::new();
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return;
    }
    loop {
        children.push(cursor.node());
        if !cursor.goto_next_sibling() {
            break;
        }
    }

    let mut i = 0;
    while i < children.len() {
        let child = children[i];

        if child.kind() == "token_tree" {
            collect_idents_from_token_tree(&child, source, commands);
            i += 1;
        } else if child.kind() == "scoped_identifier" {
            // Structured scoped_identifier (may appear in some tree-sitter versions)
            if let Some(name_node) = child.child_by_field_name("name") {
                commands.push(node_text(&name_node, source));
            }
            i += 1;
        } else if child.kind() == "identifier" {
            // Peek ahead: if followed by `::` + `identifier`, skip to the last segment
            let name = resolve_scoped_path(&children, &mut i, source);
            commands.push(name);
        } else {
            i += 1;
        }
    }
}

/// Resolve a potentially scoped path like `commands::greet` to its final segment.
///
/// Consumes `identifier (:: identifier)*` from `children[*idx..]` and returns
/// the last identifier. Advances `*idx` past all consumed tokens.
fn resolve_scoped_path(
    children: &[tree_sitter::Node<'_>],
    idx: &mut usize,
    source: &[u8],
) -> String {
    let mut name = node_text(&children[*idx], source);
    *idx += 1;

    // Consume `:: identifier` pairs
    while *idx + 1 < children.len() {
        let sep = node_text(&children[*idx], source);
        if sep == "::" && children[*idx + 1].kind() == "identifier" {
            name = node_text(&children[*idx + 1], source);
            *idx += 2;
        } else {
            break;
        }
    }

    name
}

/// Boost confidence of command export endpoints that appear in `generate_handler!`.
///
/// Mutates the endpoint list in place: any `TauriCommand` export whose
/// `binding_key` is in `registered` gets its confidence promoted to
/// [`ConfidenceLevel::RegistrationValidated`].
///
/// # Examples
///
/// ```
/// use ministr_core::code::bridge::{BridgeEndpoint, BridgeKind, EndpointRole, ConfidenceLevel};
/// use ministr_core::code::bridge::tauri::boost_registered_commands;
///
/// let mut endpoints = vec![BridgeEndpoint {
///     binding_key: "greet".into(),
///     kind: BridgeKind::TauriCommand,
///     role: EndpointRole::Export,
///     language: "rust".into(),
///     file_path: "src/main.rs".into(),
///     line: 5,
///     symbol_name: "greet".into(),
///     confidence: 0.9,
/// }];
///
/// boost_registered_commands(&mut endpoints, &["greet".to_string()]);
/// assert!((endpoints[0].confidence - 1.0).abs() < f32::EPSILON);
/// ```
pub fn boost_registered_commands(endpoints: &mut [BridgeEndpoint], registered: &[String]) {
    for ep in endpoints.iter_mut() {
        if ep.kind == BridgeKind::TauriCommand
            && ep.role == EndpointRole::Export
            && registered.contains(&ep.binding_key)
        {
            ep.confidence = ConfidenceLevel::RegistrationValidated.score();
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract UTF-8 text from a tree-sitter node.
fn node_text(node: &tree_sitter::Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

/// Get the callable name from a function node in a call expression.
///
/// Handles both plain identifiers (`emit`) and member expressions
/// (`appWindow.emit`, `event.listen`).
fn callable_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => Some(node_text(node, source)),
        "member_expression" => {
            // `obj.method` — return the method name
            let property = node.child_by_field_name("property")?;
            Some(node_text(&property, source))
        }
        _ => None,
    }
}

/// Extract the first string literal argument from an arguments node (JS/TS).
///
/// Handles `"string"` and `'string'` by stripping quotes.
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
        // In tree-sitter-typescript, string fragments may differ
        if child.kind() == "template_string" {
            let text = node_text(&child, source);
            // Only handle simple template strings without interpolation
            if !text.contains("${") {
                return Some(strip_quotes(&text));
            }
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Extract the first string literal argument from an arguments node (Rust).
///
/// Handles `"string"` by stripping quotes from `string_literal` nodes.
fn first_rust_string_arg(args_node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = args_node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        if child.kind() == "string_literal" {
            return Some(strip_quotes(&node_text(&child, source)));
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

    /// Parse Rust source into a tree-sitter tree.
    fn parse_rust(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    /// Parse JavaScript source into a tree-sitter tree.
    #[cfg(feature = "lang-javascript")]
    fn parse_js(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    /// Parse TypeScript source into a tree-sitter tree.
    #[cfg(feature = "lang-typescript")]
    fn parse_ts(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    // -- TauriCommandExtractor: Rust exports --

    #[test]
    fn rust_command_export_basic() {
        let source = r#"
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}
"#;
        let tree = parse_rust(source);
        let extractor = TauriCommandExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src-tauri/src/main.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "greet");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
        assert_eq!(endpoints[0].kind, BridgeKind::TauriCommand);
        assert_eq!(endpoints[0].language, "rust");
        assert!(
            (endpoints[0].confidence - ConfidenceLevel::CaseTransformed.score()).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn rust_command_export_multiple() {
        let source = r#"
#[tauri::command]
fn cmd_a() -> String { "a".into() }

fn not_a_command() {}

#[tauri::command]
async fn cmd_b(state: tauri::State<'_, AppState>) -> Result<String, String> {
    Ok("b".into())
}
"#;
        let tree = parse_rust(source);
        let extractor = TauriCommandExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].binding_key, "cmd_a");
        assert_eq!(endpoints[1].binding_key, "cmd_b");
    }

    #[test]
    fn rust_command_export_no_attribute() {
        let source = r#"
fn regular_function() -> String { "hello".into() }

#[derive(Debug)]
struct Foo;
"#;
        let tree = parse_rust(source);
        let extractor = TauriCommandExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        assert!(endpoints.is_empty());
    }

    // -- TauriCommandExtractor: JS/TS imports --

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_invoke_import_basic() {
        let source = r#"
import { invoke } from "@tauri-apps/api/core";

async function greetUser() {
    const result = await invoke("greet", { name: "World" });
    console.log(result);
}
"#;
        let tree = parse_js(source);
        let extractor = TauriCommandExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/App.jsx", "javascript");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "greet");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
        assert_eq!(endpoints[0].kind, BridgeKind::TauriCommand);
        assert!((endpoints[0].confidence - ConfidenceLevel::Exact.score()).abs() < f32::EPSILON);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn ts_invoke_import_multiple() {
        let source = r#"
import { invoke } from "@tauri-apps/api/core";

async function main() {
    await invoke("cmd_a");
    const data = await invoke("cmd_b", { value: 42 });
    invoke("cmd_c");
}
"#;
        let tree = parse_ts(source);
        let extractor = TauriCommandExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/main.ts", "typescript");

        assert_eq!(endpoints.len(), 3);
        let keys: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(keys.contains(&"cmd_a"));
        assert!(keys.contains(&"cmd_b"));
        assert!(keys.contains(&"cmd_c"));
    }

    // -- TauriEventExtractor: Rust events --

    #[test]
    fn rust_event_emit() {
        let source = r#"
fn send_progress(app: &AppHandle) {
    app.emit("progress", 42);
}
"#;
        let tree = parse_rust(source);
        let extractor = TauriEventExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src-tauri/src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "progress");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
        assert_eq!(endpoints[0].kind, BridgeKind::TauriEvent);
    }

    #[test]
    fn rust_event_listen() {
        let source = r#"
fn setup(app: &AppHandle) {
    app.listen("user-action", |event| {
        println!("got event: {:?}", event);
    });
}
"#;
        let tree = parse_rust(source);
        let extractor = TauriEventExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src-tauri/src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "user-action");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
    }

    #[test]
    fn rust_event_emit_to() {
        let source = r#"
fn notify(app: &AppHandle) {
    app.emit_to("main", "reload", ());
    app.emit_filter("update", payload, |_| true);
}
"#;
        let tree = parse_rust(source);
        let extractor = TauriEventExtractor;
        let endpoints = extractor.extract_endpoints(&tree, source.as_bytes(), "src/lib.rs", "rust");

        // emit_to first string arg is the target label, not event name
        // emit_filter first string arg is the event name
        // We extract string args positionally — for emit_to, the first arg is "main"
        // which is actually the target, and "reload" is the event. This is a known
        // limitation; for now we take the first string arg consistently.
        assert_eq!(endpoints.len(), 2);
    }

    // -- TauriEventExtractor: JS/TS events --

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_event_listen() {
        let source = r#"
import { listen } from "@tauri-apps/api/event";

async function setup() {
    await listen("progress", (event) => {
        console.log(event.payload);
    });
}
"#;
        let tree = parse_js(source);
        let extractor = TauriEventExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/App.jsx", "javascript");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "progress");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
        assert_eq!(endpoints[0].kind, BridgeKind::TauriEvent);
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_event_emit() {
        let source = r#"
import { emit } from "@tauri-apps/api/event";

function sendAction() {
    emit("user-action", { type: "click" });
}
"#;
        let tree = parse_js(source);
        let extractor = TauriEventExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/App.jsx", "javascript");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "user-action");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn ts_event_member_expression() {
        let source = r#"
import { appWindow } from "@tauri-apps/api/window";

async function setup() {
    await appWindow.listen("file-drop", (event) => {
        console.log(event.payload);
    });
    appWindow.once("loaded", () => {});
}
"#;
        let tree = parse_ts(source);
        let extractor = TauriEventExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/main.ts", "typescript");

        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].binding_key, "file-drop");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
        assert_eq!(endpoints[1].binding_key, "loaded");
        assert_eq!(endpoints[1].role, EndpointRole::Import);
    }

    // -- Command registration validation --

    #[test]
    fn extract_generate_handler_commands() {
        let source = r#"
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet, fetch_data, save_file])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
"#;
        let tree = parse_rust(source);
        let commands = extract_registered_commands(&tree, source.as_bytes());

        assert_eq!(commands.len(), 3);
        assert!(commands.contains(&"greet".to_string()));
        assert!(commands.contains(&"fetch_data".to_string()));
        assert!(commands.contains(&"save_file".to_string()));
    }

    #[test]
    fn extract_generate_handler_empty() {
        let source = r#"
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error");
}
"#;
        let tree = parse_rust(source);
        let commands = extract_registered_commands(&tree, source.as_bytes());
        assert!(commands.is_empty());
    }

    #[test]
    fn extract_generate_handler_with_module_paths() {
        let source = r#"
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::greet, commands::save])
        .run(tauri::generate_context!())
        .expect("error");
}
"#;
        let tree = parse_rust(source);
        let commands = extract_registered_commands(&tree, source.as_bytes());

        assert_eq!(commands.len(), 2);
        assert!(commands.contains(&"greet".to_string()));
        assert!(commands.contains(&"save".to_string()));
    }

    #[test]
    fn boost_registered_commands_works() {
        let mut endpoints = vec![
            BridgeEndpoint {
                binding_key: "greet".into(),
                kind: BridgeKind::TauriCommand,
                role: EndpointRole::Export,
                language: "rust".into(),
                file_path: "src/main.rs".into(),
                line: 5,
                symbol_name: "greet".into(),
                confidence: ConfidenceLevel::CaseTransformed.score(),
            },
            BridgeEndpoint {
                binding_key: "unregistered_cmd".into(),
                kind: BridgeKind::TauriCommand,
                role: EndpointRole::Export,
                language: "rust".into(),
                file_path: "src/main.rs".into(),
                line: 10,
                symbol_name: "unregistered_cmd".into(),
                confidence: ConfidenceLevel::CaseTransformed.score(),
            },
        ];

        let registered = vec!["greet".to_string()];
        boost_registered_commands(&mut endpoints, &registered);

        assert!(
            (endpoints[0].confidence - ConfidenceLevel::RegistrationValidated.score()).abs()
                < f32::EPSILON,
            "registered command should be boosted to 1.0"
        );
        assert!(
            (endpoints[1].confidence - ConfidenceLevel::CaseTransformed.score()).abs()
                < f32::EPSILON,
            "unregistered command should stay at 0.9"
        );
    }

    #[test]
    fn boost_does_not_affect_imports() {
        let mut endpoints = vec![BridgeEndpoint {
            binding_key: "greet".into(),
            kind: BridgeKind::TauriCommand,
            role: EndpointRole::Import,
            language: "typescript".into(),
            file_path: "src/app.ts".into(),
            line: 5,
            symbol_name: "greet".into(),
            confidence: ConfidenceLevel::Exact.score(),
        }];

        boost_registered_commands(&mut endpoints, &["greet".to_string()]);

        // Import endpoints should not be modified
        assert_eq!(endpoints[0].role, EndpointRole::Import);
        assert!((endpoints[0].confidence - ConfidenceLevel::Exact.score()).abs() < f32::EPSILON);
    }

    // -- Integration: end-to-end with BridgeLinker --

    #[test]
    fn command_extractor_with_linker() {
        use super::super::linker::BridgeLinker;

        let rust_source = r#"
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}
"#;
        let rust_tree = parse_rust(rust_source);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(TauriCommandExtractor));

        let rust_file = super::super::linker::SourceFile {
            file_path: "src-tauri/src/main.rs",
            language: "rust",
            tree: &rust_tree,
            source: rust_source.as_bytes(),
        };

        let endpoints = linker.extract_all(&[rust_file]);
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "greet");
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn command_full_link() {
        use super::super::linker::{BridgeLinker, SourceFile};

        let rust_source = r#"
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}
"#;
        let js_source = r#"
import { invoke } from "@tauri-apps/api/core";
const result = await invoke("greet", { name: "World" });
"#;
        let rust_tree = parse_rust(rust_source);
        let js_tree = parse_js(js_source);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(TauriCommandExtractor));

        let files = [
            SourceFile {
                file_path: "src-tauri/src/main.rs",
                language: "rust",
                tree: &rust_tree,
                source: rust_source.as_bytes(),
            },
            SourceFile {
                file_path: "src/App.jsx",
                language: "javascript",
                tree: &js_tree,
                source: js_source.as_bytes(),
            },
        ];

        let links = linker.extract_and_link(&files);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, BridgeKind::TauriCommand);
        assert_eq!(links[0].export.language, "rust");
        assert_eq!(links[0].import.language, "javascript");
        assert_eq!(links[0].export.binding_key, "greet");
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn event_full_link() {
        use super::super::linker::{BridgeLinker, SourceFile};

        let rust_source = r#"
fn send_progress(app: &AppHandle) {
    app.emit("download-progress", 42);
}
"#;
        let js_source = r#"
import { listen } from "@tauri-apps/api/event";
await listen("download-progress", (event) => {
    console.log(event.payload);
});
"#;
        let rust_tree = parse_rust(rust_source);
        let js_tree = parse_js(js_source);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(TauriEventExtractor));

        let files = [
            SourceFile {
                file_path: "src-tauri/src/lib.rs",
                language: "rust",
                tree: &rust_tree,
                source: rust_source.as_bytes(),
            },
            SourceFile {
                file_path: "src/App.jsx",
                language: "javascript",
                tree: &js_tree,
                source: js_source.as_bytes(),
            },
        ];

        let links = linker.extract_and_link(&files);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, BridgeKind::TauriEvent);
        assert_eq!(links[0].export.binding_key, "download-progress");
        assert_eq!(links[0].export.language, "rust");
        assert_eq!(links[0].import.language, "javascript");
    }

    #[test]
    fn strip_quotes_variants() {
        assert_eq!(strip_quotes("\"hello\""), "hello");
        assert_eq!(strip_quotes("'hello'"), "hello");
        assert_eq!(strip_quotes("`hello`"), "hello");
        assert_eq!(strip_quotes("noquotes"), "noquotes");
    }

    // -- Edge cases: rename_all, async, command modules, v1/v2 patterns --

    #[test]
    fn rust_command_export_with_rename_all_attribute() {
        // #[tauri::command(rename_all = "snake_case")] is still detected as a command
        let source = r#"
#[tauri::command(rename_all = "snake_case")]
fn get_user_data(user_id: i32) -> String {
    format!("user_{user_id}")
}
"#;
        let tree = parse_rust(source);
        let extractor = TauriCommandExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src-tauri/src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "get_user_data");
        assert_eq!(endpoints[0].role, EndpointRole::Export);
    }

    #[test]
    fn rust_command_export_with_async_attribute() {
        // #[tauri::command(async)] on a sync fn — should still be detected
        let source = r"
#[tauri::command(async)]
fn heavy_computation(input: String) -> String {
    input.to_uppercase()
}
";
        let tree = parse_rust(source);
        let extractor = TauriCommandExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src-tauri/src/lib.rs", "rust");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "heavy_computation");
    }

    #[test]
    fn rust_command_export_in_module() {
        // Commands defined inside a mod block
        let source = r"
mod commands {
    #[tauri::command]
    fn list_items() -> Vec<String> {
        vec![]
    }

    #[tauri::command]
    async fn delete_item(id: i32) -> Result<(), String> {
        Ok(())
    }
}
";
        let tree = parse_rust(source);
        let extractor = TauriCommandExtractor;
        let endpoints = extractor.extract_endpoints(
            &tree,
            source.as_bytes(),
            "src-tauri/src/commands.rs",
            "rust",
        );

        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].binding_key, "list_items");
        assert_eq!(endpoints[1].binding_key, "delete_item");
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_invoke_v1_import_path() {
        // Tauri v1 uses @tauri-apps/api/tauri, v2 uses @tauri-apps/api/core
        // Both should be detected
        let source = r#"
import { invoke } from "@tauri-apps/api/tauri";

async function loadData() {
    const data = await invoke("fetch_records");
    return data;
}
"#;
        let tree = parse_js(source);
        let extractor = TauriCommandExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/App.jsx", "javascript");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "fetch_records");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_invoke_tauri_global() {
        // window.__TAURI__.core.invoke() — Tauri v2 global access
        let source = r#"
async function callCommand() {
    const result = await window.__TAURI__.core.invoke("get_status");
    return result;
}
"#;
        let tree = parse_js(source);
        let extractor = TauriCommandExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/legacy.js", "javascript");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "get_status");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn js_invoke_tauri_v1_global() {
        // window.__TAURI__.tauri.invoke() — Tauri v1 global access
        let source = r#"
async function callCommand() {
    const result = await window.__TAURI__.tauri.invoke("get_config");
    return result;
}
"#;
        let tree = parse_js(source);
        let extractor = TauriCommandExtractor;
        let endpoints =
            extractor.extract_endpoints(&tree, source.as_bytes(), "src/legacy.js", "javascript");

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].binding_key, "get_config");
        assert_eq!(endpoints[0].role, EndpointRole::Import);
    }

    #[test]
    fn generate_handler_with_nested_module_paths() {
        // Deep module paths: a::b::c::command_name
        let source = r#"
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            api::v1::commands::list_users,
            api::v1::commands::get_user
        ])
        .run(tauri::generate_context!())
        .expect("error");
}
"#;
        let tree = parse_rust(source);
        let commands = extract_registered_commands(&tree, source.as_bytes());

        assert_eq!(commands.len(), 2);
        assert!(commands.contains(&"list_users".to_string()));
        assert!(commands.contains(&"get_user".to_string()));
    }

    /// Test that case-normalized matching works for Tauri commands
    /// where Rust uses `snake_case` and JS uses `camelCase`.
    #[cfg(feature = "lang-javascript")]
    #[test]
    fn case_normalized_command_linking() {
        use super::super::linker::{BridgeLinker, SourceFile};

        let rust_source = r#"
#[tauri::command]
fn get_user_data(id: i32) -> String {
    format!("user_{id}")
}
"#;
        // JS side uses camelCase (hypothetical user mistake or convention)
        let js_source = r#"
import { invoke } from "@tauri-apps/api/core";
const data = await invoke("getUserData", { id: 42 });
"#;
        let rust_tree = parse_rust(rust_source);
        let js_tree = parse_js(js_source);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(TauriCommandExtractor));

        let files = [
            SourceFile {
                file_path: "src-tauri/src/main.rs",
                language: "rust",
                tree: &rust_tree,
                source: rust_source.as_bytes(),
            },
            SourceFile {
                file_path: "src/App.jsx",
                language: "javascript",
                tree: &js_tree,
                source: js_source.as_bytes(),
            },
        ];

        let links = linker.extract_and_link(&files);
        assert_eq!(
            links.len(),
            1,
            "case-normalized match should produce a link"
        );
        assert_eq!(links[0].kind, BridgeKind::TauriCommand);
        // Confidence should be capped at CaseTransformed level
        assert!(
            links[0].confidence <= ConfidenceLevel::CaseTransformed.score(),
            "case-normalized link should have CaseTransformed confidence"
        );
    }

    // -----------------------------------------------------------------------
    // L3.4 — Comprehensive Tauri app test fixture
    // -----------------------------------------------------------------------
    //
    // Exercises: 5+ commands (basic, async, state, rename_all, module),
    // events, state management, plugin usage, and both v1/v2 JS patterns.

    /// Realistic Rust backend fixture for a minimal Tauri app.
    const FIXTURE_RUST_COMMANDS: &str = r#"
use tauri::State;

struct AppState {
    db: Database,
}

// Basic command
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

// Async command with state
#[tauri::command]
async fn fetch_records(state: State<'_, AppState>, limit: usize) -> Result<Vec<String>, String> {
    Ok(vec!["record1".into()])
}

// Command with rename_all attribute
#[tauri::command(rename_all = "snake_case")]
fn get_app_config() -> Config {
    Config::default()
}

// Sync command forced async via attribute
#[tauri::command(async)]
fn compute_hash(data: String) -> String {
    format!("hash_{data}")
}

// Command returning Result
#[tauri::command]
async fn save_document(title: String, content: String) -> Result<bool, String> {
    Ok(true)
}

// Command in a module
mod admin {
    use tauri::State;

    #[tauri::command]
    fn reset_database(state: State<'_, super::AppState>) -> Result<(), String> {
        Ok(())
    }
}

// Registration
fn main() {
    tauri::Builder::default()
        .manage(AppState { db: Database::new() })
        .plugin(tauri_plugin_store::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            greet,
            fetch_records,
            get_app_config,
            compute_hash,
            save_document,
            admin::reset_database
        ])
        .run(tauri::generate_context!())
        .expect("error");
}
"#;

    /// Rust event-related fixture.
    const FIXTURE_RUST_EVENTS: &str = r#"
use tauri::AppHandle;

fn emit_progress(app: &AppHandle, progress: f64) {
    app.emit("download-progress", progress);
}

fn emit_status(app: &AppHandle) {
    app.emit_to("main", "status-update", "ready");
}

fn setup_listeners(app: &AppHandle) {
    app.listen("user-action", |event| {
        println!("action: {:?}", event);
    });
    app.once("app-ready", |_| {
        println!("app is ready");
    });
}
"#;

    /// JS/TS frontend fixture exercising v2 import paths.
    const FIXTURE_JS_V2_FRONTEND: &str = r#"
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";

// Command invocations
async function loadApp() {
    const greeting = await invoke("greet", { name: "World" });
    const records = await invoke("fetch_records", { limit: 10 });
    const config = await invoke("get_app_config");
    const hash = await invoke("compute_hash", { data: "hello" });
    const saved = await invoke("save_document", { title: "Doc", content: "..." });
    const result = await invoke("reset_database");
}

// Event listeners
async function setupEvents() {
    await listen("download-progress", (event) => {
        updateProgressBar(event.payload);
    });
    await listen("status-update", (event) => {
        console.log(event.payload);
    });
    emit("user-action", { type: "click", target: "button" });
}
"#;

    /// JS frontend using Tauri v1 import paths.
    const FIXTURE_JS_V1_FRONTEND: &str = r#"
import { invoke } from "@tauri-apps/api/tauri";
import { listen, emit } from "@tauri-apps/api/event";

async function legacyLoad() {
    const greeting = await invoke("greet", { name: "Legacy" });
    await listen("download-progress", (event) => {
        console.log(event.payload);
    });
}
"#;

    #[test]
    fn fixture_rust_commands_extraction() {
        let tree = parse_rust(FIXTURE_RUST_COMMANDS);
        let extractor = TauriCommandExtractor;
        let endpoints = extractor.extract_endpoints(
            &tree,
            FIXTURE_RUST_COMMANDS.as_bytes(),
            "src-tauri/src/main.rs",
            "rust",
        );

        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"greet"), "should find greet command");
        assert!(
            names.contains(&"fetch_records"),
            "should find async command"
        );
        assert!(
            names.contains(&"get_app_config"),
            "should find rename_all command"
        );
        assert!(
            names.contains(&"compute_hash"),
            "should find async-attribute command"
        );
        assert!(
            names.contains(&"save_document"),
            "should find Result-returning command"
        );
        assert!(
            names.contains(&"reset_database"),
            "should find command in module"
        );
        assert!(endpoints.len() >= 6, "should find at least 6 commands");

        // All should be exports
        for ep in &endpoints {
            assert_eq!(ep.role, EndpointRole::Export);
            assert_eq!(ep.kind, BridgeKind::TauriCommand);
            assert_eq!(ep.language, "rust");
        }
    }

    #[test]
    fn fixture_rust_command_registration() {
        let tree = parse_rust(FIXTURE_RUST_COMMANDS);
        let commands = extract_registered_commands(&tree, FIXTURE_RUST_COMMANDS.as_bytes());

        assert_eq!(commands.len(), 6);
        assert!(commands.contains(&"greet".to_string()));
        assert!(commands.contains(&"fetch_records".to_string()));
        assert!(commands.contains(&"get_app_config".to_string()));
        assert!(commands.contains(&"compute_hash".to_string()));
        assert!(commands.contains(&"save_document".to_string()));
        assert!(commands.contains(&"reset_database".to_string()));
    }

    #[test]
    fn fixture_rust_events_extraction() {
        let tree = parse_rust(FIXTURE_RUST_EVENTS);
        let extractor = TauriEventExtractor;
        let endpoints = extractor.extract_endpoints(
            &tree,
            FIXTURE_RUST_EVENTS.as_bytes(),
            "src-tauri/src/events.rs",
            "rust",
        );

        assert!(
            endpoints.len() >= 4,
            "should find at least 4 event endpoints"
        );

        let exports: Vec<&BridgeEndpoint> = endpoints
            .iter()
            .filter(|e| e.role == EndpointRole::Export)
            .collect();
        let imports: Vec<&BridgeEndpoint> = endpoints
            .iter()
            .filter(|e| e.role == EndpointRole::Import)
            .collect();

        assert!(
            exports.len() >= 2,
            "should have at least 2 event exports (emit, emit_to)"
        );
        assert!(
            imports.len() >= 2,
            "should have at least 2 event imports (listen, once)"
        );
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn fixture_js_v2_command_extraction() {
        let tree = parse_js(FIXTURE_JS_V2_FRONTEND);
        let extractor = TauriCommandExtractor;
        let endpoints = extractor.extract_endpoints(
            &tree,
            FIXTURE_JS_V2_FRONTEND.as_bytes(),
            "src/App.jsx",
            "javascript",
        );

        let names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert_eq!(endpoints.len(), 6, "should find 6 invoke calls");
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"fetch_records"));
        assert!(names.contains(&"get_app_config"));
        assert!(names.contains(&"compute_hash"));
        assert!(names.contains(&"save_document"));
        assert!(names.contains(&"reset_database"));
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn fixture_js_v2_event_extraction() {
        let tree = parse_js(FIXTURE_JS_V2_FRONTEND);
        let extractor = TauriEventExtractor;
        let endpoints = extractor.extract_endpoints(
            &tree,
            FIXTURE_JS_V2_FRONTEND.as_bytes(),
            "src/App.jsx",
            "javascript",
        );

        assert!(
            endpoints.len() >= 3,
            "should find at least 3 event endpoints"
        );

        let event_names: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(event_names.contains(&"download-progress"));
        assert!(event_names.contains(&"status-update"));
        assert!(event_names.contains(&"user-action"));
    }

    #[cfg(feature = "lang-javascript")]
    #[test]
    fn fixture_js_v1_extraction() {
        let tree = parse_js(FIXTURE_JS_V1_FRONTEND);
        let extractor = TauriCommandExtractor;
        let cmd_endpoints = extractor.extract_endpoints(
            &tree,
            FIXTURE_JS_V1_FRONTEND.as_bytes(),
            "src/legacy.js",
            "javascript",
        );
        assert_eq!(cmd_endpoints.len(), 1);
        assert_eq!(cmd_endpoints[0].binding_key, "greet");

        let event_extractor = TauriEventExtractor;
        let event_endpoints = event_extractor.extract_endpoints(
            &tree,
            FIXTURE_JS_V1_FRONTEND.as_bytes(),
            "src/legacy.js",
            "javascript",
        );
        assert_eq!(event_endpoints.len(), 1);
        assert_eq!(event_endpoints[0].binding_key, "download-progress");
    }

    /// End-to-end integration: link the full Rust backend with JS v2 frontend.
    #[cfg(feature = "lang-javascript")]
    #[test]
    fn fixture_full_app_linking() {
        use super::super::linker::{BridgeLinker, SourceFile};

        let rust_cmd_tree = parse_rust(FIXTURE_RUST_COMMANDS);
        let rust_evt_tree = parse_rust(FIXTURE_RUST_EVENTS);
        let js_tree = parse_js(FIXTURE_JS_V2_FRONTEND);

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(TauriCommandExtractor));
        linker.register(Box::new(TauriEventExtractor));

        let files = [
            SourceFile {
                file_path: "src-tauri/src/main.rs",
                language: "rust",
                tree: &rust_cmd_tree,
                source: FIXTURE_RUST_COMMANDS.as_bytes(),
            },
            SourceFile {
                file_path: "src-tauri/src/events.rs",
                language: "rust",
                tree: &rust_evt_tree,
                source: FIXTURE_RUST_EVENTS.as_bytes(),
            },
            SourceFile {
                file_path: "src/App.jsx",
                language: "javascript",
                tree: &js_tree,
                source: FIXTURE_JS_V2_FRONTEND.as_bytes(),
            },
        ];

        let links = linker.extract_and_link(&files);

        // Separate command and event links
        let cmd_links: Vec<_> = links
            .iter()
            .filter(|l| l.kind == BridgeKind::TauriCommand)
            .collect();
        let evt_links: Vec<_> = links
            .iter()
            .filter(|l| l.kind == BridgeKind::TauriEvent)
            .collect();

        assert!(
            cmd_links.len() >= 5,
            "should link at least 5 commands, got {}",
            cmd_links.len()
        );
        assert!(
            evt_links.len() >= 2,
            "should link at least 2 events, got {}",
            evt_links.len()
        );

        // Verify specific command links
        let cmd_keys: Vec<&str> = cmd_links
            .iter()
            .map(|l| l.export.binding_key.as_str())
            .collect();
        assert!(cmd_keys.contains(&"greet"), "greet should be linked");
        assert!(
            cmd_keys.contains(&"fetch_records"),
            "fetch_records should be linked"
        );

        // Verify all links have cross-language pairs
        for link in &links {
            assert_ne!(
                link.export.language, link.import.language,
                "links should connect different languages"
            );
        }
    }

    /// Registration validation: boost commands that appear in `generate_handler!`
    #[test]
    fn fixture_registration_boost() {
        let tree = parse_rust(FIXTURE_RUST_COMMANDS);
        let extractor = TauriCommandExtractor;
        let mut endpoints = extractor.extract_endpoints(
            &tree,
            FIXTURE_RUST_COMMANDS.as_bytes(),
            "src-tauri/src/main.rs",
            "rust",
        );
        let registered = extract_registered_commands(&tree, FIXTURE_RUST_COMMANDS.as_bytes());

        boost_registered_commands(&mut endpoints, &registered);

        // All commands should be boosted to RegistrationValidated
        for ep in &endpoints {
            assert!(
                (ep.confidence - ConfidenceLevel::RegistrationValidated.score()).abs()
                    < f32::EPSILON,
                "command '{}' should be registration-validated",
                ep.binding_key
            );
        }
    }
}
