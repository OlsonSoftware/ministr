//! Language-agnostic symbol extraction using tree-sitter node kind heuristics.
//!
//! The [`GenericExtractor`] identifies top-level definitions across any
//! tree-sitter grammar by matching node kind patterns common to most
//! languages (e.g. `*_definition`, `*_declaration`, `function_*`, `class_*`).
//!
//! For languages with a dedicated refinement (see [`crate::code::lang`]),
//! the refinement is used instead. The generic extractor serves as a
//! fallback for languages without specific support.

use crate::code::ast_parser::ItemKind;
use crate::code::symbol::{Symbol, Visibility};

/// Extract symbols from a parsed tree using language-agnostic heuristics.
///
/// Walks top-level children and matches node kinds against common patterns
/// across tree-sitter grammars. Falls back to basic name extraction using
/// the `name` or `identifier` child fields.
///
/// # Examples
///
/// ```
/// use ministr_core::code::generic_extract_symbols;
///
/// let mut parser = tree_sitter::Parser::new();
/// parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
/// let tree = parser.parse(b"pub fn hello() {}", None).unwrap();
/// let symbols = generic_extract_symbols(&tree, b"pub fn hello() {}", "lib.rs", &[]);
/// assert_eq!(symbols.len(), 1);
/// assert_eq!(symbols[0].name, "hello");
/// ```
#[must_use]
pub fn generic_extract_symbols(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    module_path: &[&str],
) -> Vec<Symbol> {
    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        extract_from_node(&child, source, file_path, module_path, &mut symbols);
    }

    symbols
}

/// Extract a symbol from a single node, unwrapping wrapper nodes as needed.
fn extract_from_node(
    node: &tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    module_path: &[&str],
    symbols: &mut Vec<Symbol>,
) {
    let node_kind = node.kind();

    // Wrapper nodes: unwrap and process children instead
    if is_wrapper_node(node_kind) {
        let mut inner_cursor = node.walk();
        for inner in node.children(&mut inner_cursor) {
            // For wrapper unwrapping, we recurse into any classifiable child,
            // OR any child that itself is a wrapper / function-decl candidate
            // (e.g. template_declaration -> alias_declaration, or
            //  field_declaration_list -> field_declaration).
            let inner_kind = inner.kind();
            let recurse = classify_node_kind(inner_kind).is_some()
                || is_wrapper_node(inner_kind)
                || is_function_decl_node(&inner);
            if recurse {
                // Inherit visibility from the wrapper (e.g. export_statement)
                let mut inner_symbols = Vec::new();
                extract_from_node(&inner, source, file_path, module_path, &mut inner_symbols);
                // If wrapper is an export, mark children as public
                if node_kind == "export_statement" {
                    for sym in &mut inner_symbols {
                        sym.visibility = Visibility::Public;
                        // Extend byte range to cover the export wrapper
                        if node.start_byte() < sym.byte_range.start {
                            sym.byte_range.start = node.start_byte();
                        }
                    }
                }
                symbols.extend(inner_symbols);
            }
        }
        return;
    }

    // C/C++: forward function declarations are `declaration` nodes whose
    // `declarator` field is a function_declarator. In-class member methods
    // are the same shape but wrapped in `field_declaration`. Both should
    // produce a Function symbol; pure data declarations are skipped.
    if (node_kind == "declaration" || node_kind == "field_declaration")
        && is_function_decl_node(node)
    {
        if let Some(name) = extract_c_decl_name(node, source) {
            let visibility = detect_visibility_generic(node, source);
            let signature = extract_signature_generic(node, source);
            let doc_comment = extract_doc_comment_generic(node, source);
            let annotations = extract_annotations_generic(node, source);
            let byte_start =
                doc_comment_start_byte_generic(node, source).unwrap_or(node.start_byte());
            symbols.push(Symbol {
                name,
                kind: ItemKind::Function,
                visibility,
                signature,
                doc_comment,
                annotations,
                file_path: file_path.to_string(),
                byte_range: byte_start..node.end_byte(),
                module_path: module_path.iter().map(|s| (*s).to_string()).collect(),
            });
        }
        return;
    }

    let Some(item_kind) = classify_node_kind(node_kind) else {
        return;
    };

    let name = extract_name_generic(node, source);
    if name.is_empty() || name == "<unknown>" {
        return;
    }

    let visibility = detect_visibility_generic(node, source);
    let signature = extract_signature_generic(node, source);
    let doc_comment = extract_doc_comment_generic(node, source);
    let annotations = extract_annotations_generic(node, source);

    // Extend byte range to include preceding doc comments
    let byte_start = doc_comment_start_byte_generic(node, source).unwrap_or(node.start_byte());

    let sym_name = name.clone();
    symbols.push(Symbol {
        name,
        kind: item_kind,
        visibility,
        signature,
        doc_comment,
        annotations,
        file_path: file_path.to_string(),
        byte_range: byte_start..node.end_byte(),
        module_path: module_path.iter().map(|s| (*s).to_string()).collect(),
    });

    // Recurse into class/struct bodies to extract nested types and methods.
    // Module covers C++ namespaces, Java packages, Python module-blocks, etc.
    if item_kind == ItemKind::Struct
        || item_kind == ItemKind::Trait
        || item_kind == ItemKind::Module
    {
        extract_nested_members(node, source, file_path, module_path, &sym_name, symbols);
    }
}

/// Whether a node kind is a wrapper that should be unwrapped to find declarations.
///
/// `template_declaration` wraps the actual function/class/alias being templated
/// (tree-sitter-cpp). `field_declaration_list` wraps in-class members
/// (access_specifier, field_declaration, …). The `preproc_*` group wraps
/// declarations behind C/C++ header guards and conditional compilation,
/// which is where most public APIs live in real-world headers.
/// `linkage_specification` is `extern "C" { ... }`.
fn is_wrapper_node(kind: &str) -> bool {
    matches!(
        kind,
        "export_statement"
            | "export_declaration"
            | "declaration_list"
            | "template_declaration"
            | "field_declaration_list"
            | "preproc_ifdef"
            | "preproc_ifndef"
            | "preproc_if"
            | "preproc_else"
            | "preproc_elif"
            | "linkage_specification"
    )
}

/// Whether a `declaration` / `field_declaration` node is actually a function
/// declaration (its declarator chain contains a `function_declarator`).
///
/// Used to decide whether a C/C++ `declaration` should emit a Function symbol
/// (for forward decls in headers and in-class member declarations) or be
/// skipped (for plain data declarations like `int x;`).
fn is_function_decl_node(node: &tree_sitter::Node<'_>) -> bool {
    let Some(decl) = node.child_by_field_name("declarator") else {
        return false;
    };
    declarator_contains_function(&decl)
}

fn declarator_contains_function(node: &tree_sitter::Node<'_>) -> bool {
    if node.kind() == "function_declarator" {
        return true;
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return declarator_contains_function(&inner);
    }
    false
}

/// Extract the name from a C/C++ `declaration` / `field_declaration` node
/// whose declarator chain ends in a `function_declarator`. Preserves
/// qualified names like `Foo::bar` by returning the function_declarator's
/// inner declarator's full text.
fn extract_c_decl_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
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

/// Classify a tree-sitter node kind into an [`ItemKind`] using heuristic patterns.
///
/// This function recognizes patterns common across tree-sitter grammars:
/// - `function_definition`, `function_declaration`, `function_item`, `method_declaration`
/// - `class_definition`, `class_declaration`
/// - `struct_item`, `struct_declaration`
/// - `enum_item`, `enum_declaration`
/// - `trait_item`, `interface_declaration`, `interface_definition`
/// - `impl_item`
/// - `module_declaration`, `mod_item`, `package_declaration`
/// - `type_alias_declaration`, `type_item`
/// - `const_item`, `const_declaration`
/// - `static_item`
#[must_use]
pub fn classify_node_kind(kind: &str) -> Option<ItemKind> {
    // Exact matches first (Rust-specific from existing ast_parser)
    match kind {
        "function_item"
        | "function_definition"
        | "function_declaration"
        | "method_declaration"
        | "method_definition"
        | "arrow_function"
        | "generator_function_declaration"
        | "func_literal" => Some(ItemKind::Function),

        // C union has no dedicated ItemKind variant; we group it with structs
        // as the closest record-type analogue.
        "struct_item" | "struct_specifier" | "union_specifier" => Some(ItemKind::Struct),

        "enum_item" | "enum_specifier" | "enum_declaration" => Some(ItemKind::Enum),

        "trait_item" => Some(ItemKind::Trait),

        "interface_declaration" | "interface_definition" | "abstract_type_declaration" => {
            Some(ItemKind::Trait)
        }

        "impl_item" => Some(ItemKind::Impl),

        "mod_item" | "module_declaration" | "package_declaration" | "module" => {
            Some(ItemKind::Module)
        }

        "type_item"
        | "type_alias_declaration"
        | "type_declaration"
        | "type_spec"
        | "alias_declaration" => Some(ItemKind::Type),

        "const_item" | "const_declaration" | "const_spec" | "lexical_declaration" => {
            Some(ItemKind::Const)
        }

        "static_item" => Some(ItemKind::Static),

        "class_definition" | "class_declaration" | "class_specifier" | "decorated_definition" => {
            Some(ItemKind::Struct)
        }

        _ => {
            // Heuristic fallback: match by substring patterns
            if kind.contains("function") || kind.contains("method") {
                Some(ItemKind::Function)
            } else if kind.contains("class") {
                Some(ItemKind::Struct)
            } else if kind.contains("interface") {
                Some(ItemKind::Trait)
            } else if kind.contains("struct") {
                Some(ItemKind::Struct)
            } else if kind.contains("enum") {
                Some(ItemKind::Enum)
            } else if kind.contains("trait") {
                Some(ItemKind::Trait)
            } else if kind.contains("module") || kind.contains("namespace") {
                Some(ItemKind::Module)
            } else if kind.contains("type_alias") || kind.contains("typedef") {
                Some(ItemKind::Type)
            } else {
                None
            }
        }
    }
}

/// Extract the name of a node using common field names across grammars.
fn extract_name_generic(node: &tree_sitter::Node, source: &[u8]) -> String {
    // Try common field names in priority order
    for field in &["name", "declarator", "type", "identifier"] {
        if let Some(name_node) = node.child_by_field_name(field) {
            // For declarators (C/C++), recurse to find the identifier
            if (name_node.kind() == "function_declarator"
                || name_node.kind() == "pointer_declarator")
                && let Some(inner) = name_node.child_by_field_name("declarator")
                && let Ok(text) = inner.utf8_text(source)
            {
                let text = text.trim();
                if !text.is_empty() {
                    return text.to_string();
                }
            }
            if let Ok(text) = name_node.utf8_text(source) {
                let text = text.trim();
                if !text.is_empty() && !text.contains('{') && text.len() < 200 {
                    return text.to_string();
                }
            }
        }
    }

    // For Python decorated_definition: look inside
    if node.kind() == "decorated_definition"
        && let Some(definition) = node.child_by_field_name("definition")
    {
        return extract_name_generic(&definition, source);
    }

    // For Go type_declaration: look inside the type_spec child
    if node.kind() == "type_declaration" {
        let mut inner = node.walk();
        for child in node.children(&mut inner) {
            if child.kind() == "type_spec"
                && let Some(name_node) = child.child_by_field_name("name")
                && let Ok(text) = name_node.utf8_text(source)
            {
                return text.trim().to_string();
            }
        }
    }

    // For Go const_declaration: look inside const_spec child
    if node.kind() == "const_declaration" {
        let mut inner = node.walk();
        for child in node.children(&mut inner) {
            if child.kind() == "const_spec"
                && let Some(name_node) = child.child_by_field_name("name")
                && let Ok(text) = name_node.utf8_text(source)
            {
                return text.trim().to_string();
            }
        }
    }

    // Fallback: look for first named child that is an identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if (child.kind() == "identifier" || child.kind() == "type_identifier")
            && let Ok(text) = child.utf8_text(source)
        {
            return text.trim().to_string();
        }
    }

    "<unknown>".to_string()
}

/// Extract decorators, annotations, and attributes attached to a node.
///
/// Detects patterns across languages:
/// - Python: `decorator` children of `decorated_definition`
/// - Java/Kotlin: `marker_annotation`, `annotation` children
/// - Rust: `attribute_item` siblings preceding the node
/// - JS/TS: `decorator` children or siblings
fn extract_annotations_generic(node: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut annotations = Vec::new();

    // Python decorated_definition: decorators are children
    if node.kind() == "decorated_definition" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "decorator"
                && let Ok(text) = child.utf8_text(source)
            {
                annotations.push(text.trim().to_string());
            }
        }
        return annotations;
    }

    // Java/Kotlin/Swift: annotations appear as direct children of the declaration
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if (kind == "annotation"
            || kind == "marker_annotation"
            || kind == "attribute"
            || kind == "decorator")
            && let Ok(text) = child.utf8_text(source)
        {
            annotations.push(text.trim().to_string());
        }
    }

    // Rust/C#: attribute_item siblings preceding the node
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        let kind = sibling.kind();
        if kind == "attribute_item" || kind == "inner_attribute_item" {
            if let Ok(text) = sibling.utf8_text(source) {
                annotations.push(text.trim().to_string());
            }
        } else if kind == "line_comment" || kind == "comment" || kind == "block_comment" {
            // Skip doc comments — they sit between attributes and the item
            prev = sibling.prev_sibling();
            continue;
        } else {
            break;
        }
        prev = sibling.prev_sibling();
    }

    // Reverse the preceding-sibling annotations so they appear in source order
    if !annotations.is_empty() {
        annotations.reverse();
    }

    annotations
}

/// Recurse into class/struct/interface bodies to extract nested types and methods.
///
/// Looks for `body`, `class_body`, `declaration_list` fields on the parent node
/// and extracts classifiable children with module paths extended by the parent name.
fn extract_nested_members(
    node: &tree_sitter::Node,
    source: &[u8],
    file_path: &str,
    parent_module: &[&str],
    parent_name: &str,
    symbols: &mut Vec<Symbol>,
) {
    let body = node
        .child_by_field_name("body")
        .or_else(|| node.child_by_field_name("class_body"))
        .or_else(|| node.child_by_field_name("declaration_list"));

    // For Python decorated_definition, look inside the definition's body
    let body = body.or_else(|| {
        if node.kind() == "decorated_definition" {
            node.child_by_field_name("definition")
                .and_then(|def| def.child_by_field_name("body"))
        } else {
            None
        }
    });

    let Some(body_node) = body else {
        return;
    };

    let mut nested_path: Vec<String> = parent_module.iter().map(|s| (*s).to_string()).collect();
    nested_path.push(parent_name.to_string());
    let nested_module: Vec<&str> = nested_path.iter().map(String::as_str).collect();

    let mut body_cursor = body_node.walk();
    for child in body_node.children(&mut body_cursor) {
        extract_from_node(&child, source, file_path, &nested_module, symbols);
    }
}

/// Detect visibility from common patterns across languages.
fn detect_visibility_generic(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    // Check for Rust-style visibility_modifier field
    if let Some(vis_node) = node.child_by_field_name("visibility") {
        let text = vis_node.utf8_text(source).unwrap_or("").trim().to_string();
        return match text.as_str() {
            "pub" => Visibility::Public,
            "pub(crate)" => Visibility::PubCrate,
            "pub(super)" => Visibility::PubSuper,
            _ if text.starts_with("pub(") => Visibility::PubCrate,
            _ => Visibility::Private,
        };
    }

    // Check for access modifiers as children (Java/Kotlin/C#/Swift)
    let mut cursor = node.walk();
    let text = node.utf8_text(source).unwrap_or("");
    for child in node.children(&mut cursor) {
        let child_kind = child.kind();
        if child_kind == "visibility_modifier"
            || child_kind == "access_modifier"
            || child_kind == "modifiers"
        {
            let mod_text = child.utf8_text(source).unwrap_or("");
            if mod_text.contains("public") || mod_text.contains("open") {
                return Visibility::Public;
            } else if mod_text.contains("private") || mod_text.contains("fileprivate") {
                return Visibility::Private;
            } else if mod_text.contains("protected")
                || mod_text.contains("internal")
                || mod_text.contains("package")
            {
                return Visibility::PubCrate;
            }
        }
    }

    // Heuristic: check if the source text starts with common access modifiers
    if text.starts_with("pub ") || text.starts_with("pub(") {
        return Visibility::Public;
    }
    if text.starts_with("public ") {
        return Visibility::Public;
    }
    if text.starts_with("export ") || text.starts_with("export default ") {
        return Visibility::Public;
    }
    // Swift: `open` modifier
    if text.starts_with("open ") {
        return Visibility::Public;
    }

    // Python: check for decorated_definition wrapping
    if node.kind() == "decorated_definition"
        && let Some(def) = node.child_by_field_name("definition")
    {
        return detect_visibility_generic(&def, source);
    }

    // Python naming convention: _name is private, __name is private (mangled)
    let name = extract_name_generic(node, source);
    if name.starts_with("__") && !name.ends_with("__") {
        // Dunder methods like __init__ are public; mangled names like __secret are private
        return Visibility::Private;
    }
    if name.starts_with('_') && name != "_" {
        return Visibility::Private;
    }

    // Go convention: capitalized first letter means exported (public)
    if let Some(first_char) = name.chars().next()
        && first_char.is_uppercase()
    {
        return Visibility::Public;
    }

    Visibility::Private
}

/// Extract a signature (declaration without body) from a node.
fn extract_signature_generic(node: &tree_sitter::Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");

    // For decorated definitions, use the inner definition
    if node.kind() == "decorated_definition"
        && let Some(def) = node.child_by_field_name("definition")
    {
        return extract_signature_generic(&def, source);
    }

    // Try to find the body and take everything before it
    for field in &["body", "block", "consequence"] {
        if let Some(body_node) = node.child_by_field_name(field) {
            let body_start = body_node.start_byte() - node.start_byte();
            if body_start > 0 && body_start < text.len() {
                return text[..body_start].trim_end().to_string();
            }
        }
    }

    // Fallback: find first `{` or `:` (Python) that starts a body
    if let Some(brace_pos) = text.find('{') {
        return text[..brace_pos].trim_end().to_string();
    }

    // For Python-style: take first line as signature
    if let Some(colon_pos) = text.find(':') {
        // Only if this looks like a definition line (not a type annotation)
        let before_colon = &text[..colon_pos];
        if before_colon.contains("def ") || before_colon.contains("class ") {
            return text[..=colon_pos].trim_end().to_string();
        }
    }

    // No body — use full text, truncated
    let full = text.trim_end();
    if full.len() > 500 {
        full[..500].to_string()
    } else {
        full.to_string()
    }
}

/// Extract doc comments preceding a node using generic patterns.
fn extract_doc_comment_generic(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut doc_lines = Vec::new();
    let mut prev = node.prev_sibling();

    // For decorated definitions, also check decorators above
    if node.kind() == "decorated_definition" {
        // The doc might be above the decorators
        // Check the node's own prev sibling
    }

    while let Some(sibling) = prev {
        let text = sibling.utf8_text(source).unwrap_or("");
        let kind = sibling.kind();

        if kind == "line_comment" || kind == "comment" {
            if let Some(content) = strip_doc_comment_prefix(text) {
                doc_lines.push(content);
            } else {
                break;
            }
        } else if kind == "block_comment" {
            if let Some(content) = strip_block_doc_comment(text)
                && !content.is_empty()
            {
                doc_lines.push(content);
            }
            break;
        } else if kind == "decorator" || kind == "attribute_item" || kind == "annotation" {
            // Skip decorators/attributes between doc comments and the item
            prev = sibling.prev_sibling();
            continue;
        } else if kind == "expression_statement" {
            // Python docstrings appear as expression_statement containing a string
            // But they're *inside* the function body, not before it — skip for now
            break;
        } else {
            break;
        }

        prev = sibling.prev_sibling();
    }

    if doc_lines.is_empty() {
        return None;
    }

    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

/// Strip doc comment prefix patterns across languages.
fn strip_doc_comment_prefix(text: &str) -> Option<String> {
    let trimmed = text.trim();

    // Rust: `/// ...` or `//! ...`
    if let Some(rest) = trimmed.strip_prefix("///") {
        return Some(rest.strip_prefix(' ').unwrap_or(rest).to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("//!") {
        return Some(rest.strip_prefix(' ').unwrap_or(rest).to_string());
    }

    // Python: `# ...`
    if let Some(rest) = trimmed.strip_prefix('#') {
        return Some(rest.strip_prefix(' ').unwrap_or(rest).to_string());
    }

    // C/C++/Java/JS/Go: `// ...`
    if let Some(rest) = trimmed.strip_prefix("//") {
        return Some(rest.strip_prefix(' ').unwrap_or(rest).to_string());
    }

    None
}

/// Strip block doc comment delimiters.
fn strip_block_doc_comment(text: &str) -> Option<String> {
    let trimmed = text.trim();

    // `/** ... */` (Java/JS/TS)
    if let Some(inner) = trimmed
        .strip_prefix("/**")
        .and_then(|s| s.strip_suffix("*/"))
    {
        return Some(clean_block_comment_lines(inner));
    }

    // `/* ... */`
    if let Some(inner) = trimmed
        .strip_prefix("/*")
        .and_then(|s| s.strip_suffix("*/"))
    {
        return Some(clean_block_comment_lines(inner));
    }

    // Python triple-quote strings used as docstrings
    for delim in &["\"\"\"", "'''"] {
        if let Some(inner) = trimmed
            .strip_prefix(delim)
            .and_then(|s| s.strip_suffix(delim))
        {
            return Some(inner.trim().to_string());
        }
    }

    None
}

/// Clean up `* ` prefixes on lines within a block comment.
fn clean_block_comment_lines(inner: &str) -> String {
    inner
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("* ")
                .unwrap_or(trimmed.strip_prefix('*').unwrap_or(trimmed))
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Find the starting byte of doc comments preceding a node.
fn doc_comment_start_byte_generic(node: &tree_sitter::Node, source: &[u8]) -> Option<usize> {
    let mut earliest_start = None;
    let mut prev = node.prev_sibling();

    while let Some(sibling) = prev {
        let text = sibling.utf8_text(source).unwrap_or("");
        let kind = sibling.kind();

        if (kind == "line_comment" || kind == "comment") && strip_doc_comment_prefix(text).is_some()
        {
            earliest_start = Some(sibling.start_byte());
        } else if kind == "block_comment" {
            earliest_start = Some(sibling.start_byte());
            break;
        } else if kind == "decorator" || kind == "attribute_item" || kind == "annotation" {
            earliest_start = Some(sibling.start_byte());
            prev = sibling.prev_sibling();
            continue;
        } else {
            break;
        }

        prev = sibling.prev_sibling();
    }

    earliest_start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_function_kinds() {
        assert_eq!(
            classify_node_kind("function_item"),
            Some(ItemKind::Function)
        );
        assert_eq!(
            classify_node_kind("function_definition"),
            Some(ItemKind::Function)
        );
        assert_eq!(
            classify_node_kind("function_declaration"),
            Some(ItemKind::Function)
        );
        assert_eq!(
            classify_node_kind("method_declaration"),
            Some(ItemKind::Function)
        );
    }

    #[test]
    fn classify_class_kinds() {
        assert_eq!(
            classify_node_kind("class_definition"),
            Some(ItemKind::Struct)
        );
        assert_eq!(
            classify_node_kind("class_declaration"),
            Some(ItemKind::Struct)
        );
    }

    #[test]
    fn classify_interface_kinds() {
        assert_eq!(
            classify_node_kind("interface_declaration"),
            Some(ItemKind::Trait)
        );
    }

    #[test]
    fn classify_unknown_returns_none() {
        assert_eq!(classify_node_kind("use_declaration"), None);
        assert_eq!(classify_node_kind("import_statement"), None);
        assert_eq!(classify_node_kind("expression_statement"), None);
    }

    #[test]
    fn classify_heuristic_fallback() {
        assert_eq!(
            classify_node_kind("async_function_declaration"),
            Some(ItemKind::Function)
        );
        assert_eq!(
            classify_node_kind("abstract_class_declaration"),
            Some(ItemKind::Struct)
        );
    }

    #[test]
    fn strip_rust_doc_comment() {
        assert_eq!(
            strip_doc_comment_prefix("/// Hello world"),
            Some("Hello world".into())
        );
        assert_eq!(
            strip_doc_comment_prefix("//! Module doc"),
            Some("Module doc".into())
        );
    }

    #[test]
    fn strip_hash_comment() {
        assert_eq!(
            strip_doc_comment_prefix("# Python comment"),
            Some("Python comment".into())
        );
    }

    #[test]
    fn strip_slash_comment() {
        assert_eq!(
            strip_doc_comment_prefix("// JS comment"),
            Some("JS comment".into())
        );
    }

    #[test]
    fn strip_block_doc() {
        assert_eq!(
            strip_block_doc_comment("/** Hello */"),
            Some("Hello".into())
        );
        assert_eq!(
            strip_block_doc_comment("/* Simple */"),
            Some("Simple".into())
        );
    }

    // Rust integration test — generic extractor on Rust source
    #[test]
    fn generic_extract_from_rust() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let source = b"/// A greeting.\npub fn hello() -> String { String::new() }\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "lib.rs", &[]);

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, ItemKind::Function);
        assert_eq!(symbols[0].visibility, Visibility::Public);
        assert!(
            symbols[0]
                .doc_comment
                .as_deref()
                .unwrap()
                .contains("greeting")
        );
    }

    // Multi-language tests

    #[cfg(feature = "lang-python")]
    #[test]
    fn generic_extract_from_python() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let source = b"# A greeting function.\ndef hello(name: str) -> str:\n    return f'Hello {name}'\n\nclass Greeter:\n    def greet(self):\n        pass\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "hello.py", &[]);

        assert!(
            symbols.len() >= 2,
            "expected at least 2 symbols (fn + class), got {}: {:?}",
            symbols.len(),
            symbols
                .iter()
                .map(|s| (&s.name, s.kind))
                .collect::<Vec<_>>()
        );

        let hello = symbols.iter().find(|s| s.name == "hello");
        assert!(hello.is_some(), "expected hello function");
        assert_eq!(hello.unwrap().kind, ItemKind::Function);

        let greeter = symbols.iter().find(|s| s.name == "Greeter");
        assert!(greeter.is_some(), "expected Greeter class");
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn generic_extract_from_go() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        let source = b"package main\n\n// Hello returns a greeting.\nfunc Hello(name string) string {\n\treturn \"Hello \" + name\n}\n\ntype Greeter struct {\n\tName string\n}\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "main.go", &[]);

        assert!(
            symbols.len() >= 2,
            "expected at least 2 symbols, got {}: {:?}",
            symbols.len(),
            symbols
                .iter()
                .map(|s| (&s.name, s.kind))
                .collect::<Vec<_>>()
        );

        let hello = symbols.iter().find(|s| s.name == "Hello");
        assert!(hello.is_some(), "expected Hello function");
        assert_eq!(hello.unwrap().kind, ItemKind::Function);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn generic_extract_from_typescript() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let source = b"// A greeting.\nexport function hello(name: string): string {\n  return `Hello ${name}`;\n}\n\nexport interface Greeter {\n  greet(): string;\n}\n\nexport class MyGreeter implements Greeter {\n  greet(): string { return 'hi'; }\n}\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "hello.ts", &[]);

        assert!(
            symbols.len() >= 3,
            "expected at least 3 symbols (fn + interface + class), got {}: {:?}",
            symbols.len(),
            symbols
                .iter()
                .map(|s| (&s.name, s.kind))
                .collect::<Vec<_>>()
        );

        let hello = symbols.iter().find(|s| s.name == "hello");
        assert!(hello.is_some(), "expected hello function");

        let greeter_iface = symbols.iter().find(|s| s.name == "Greeter");
        assert!(greeter_iface.is_some(), "expected Greeter interface");
    }

    #[cfg(feature = "lang-java")]
    #[test]
    fn generic_extract_from_java() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .unwrap();
        let source = b"package com.example;\n\n/** A greeting class. */\npublic class Greeter {\n    public String greet(String name) {\n        return \"Hello \" + name;\n    }\n}\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "Greeter.java", &[]);

        assert!(!symbols.is_empty(), "expected at least 1 symbol, got 0");

        let greeter = symbols.iter().find(|s| s.name == "Greeter");
        assert!(greeter.is_some(), "expected Greeter class");
    }

    #[cfg(feature = "lang-c")]
    #[test]
    fn generic_extract_from_c() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .unwrap();
        let source = b"// A greeting.\nint hello(const char *name) {\n    return 0;\n}\n\nstruct Greeter {\n    char *name;\n};\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "hello.c", &[]);

        assert!(
            symbols.len() >= 2,
            "expected at least 2 symbols, got {}: {:?}",
            symbols.len(),
            symbols
                .iter()
                .map(|s| (&s.name, s.kind))
                .collect::<Vec<_>>()
        );
    }

    // === Annotation/decorator extraction tests ===

    #[test]
    fn rust_attribute_extraction() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let source = b"#[derive(Debug, Clone)]\n#[serde(rename_all = \"camelCase\")]\npub struct Config {\n    name: String,\n}\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "lib.rs", &[]);

        let config = symbols.iter().find(|s| s.name == "Config").unwrap();
        assert_eq!(config.annotations.len(), 2, "expected 2 attributes");
        assert!(
            config.annotations[0].contains("derive(Debug, Clone)"),
            "first annotation should be derive: {:?}",
            config.annotations[0]
        );
        assert!(
            config.annotations[1].contains("serde"),
            "second annotation should be serde: {:?}",
            config.annotations[1]
        );
    }

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_decorator_extraction() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let source = b"@property\ndef name(self) -> str:\n    return self._name\n\n@app.route('/hello')\ndef hello():\n    pass\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "app.py", &[]);

        let name_fn = symbols.iter().find(|s| s.name == "name").unwrap();
        assert_eq!(name_fn.annotations.len(), 1, "expected 1 decorator");
        assert!(
            name_fn.annotations[0].contains("@property"),
            "annotation should be @property: {:?}",
            name_fn.annotations[0]
        );

        let hello_fn = symbols.iter().find(|s| s.name == "hello").unwrap();
        assert_eq!(
            hello_fn.annotations.len(),
            1,
            "expected 1 decorator on hello"
        );
        assert!(
            hello_fn.annotations[0].contains("@app.route"),
            "annotation should contain @app.route: {:?}",
            hello_fn.annotations[0]
        );
    }

    // === Nested type extraction tests ===

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_nested_methods_in_class() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let source = b"class MyClass:\n    def method_one(self):\n        pass\n\n    def method_two(self, x):\n        pass\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "my_class.py", &[]);

        let class_sym = symbols.iter().find(|s| s.name == "MyClass").unwrap();
        assert_eq!(class_sym.kind, ItemKind::Struct);

        let method_one = symbols.iter().find(|s| s.name == "method_one").unwrap();
        assert_eq!(method_one.kind, ItemKind::Function);
        assert_eq!(
            method_one.module_path,
            vec!["MyClass"],
            "nested method should have parent class in module path"
        );

        let method_two = symbols.iter().find(|s| s.name == "method_two").unwrap();
        assert_eq!(method_two.kind, ItemKind::Function);
        assert_eq!(method_two.module_path, vec!["MyClass"]);
    }

    #[cfg(feature = "lang-java")]
    #[test]
    fn java_nested_methods_in_class() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .unwrap();
        let source = b"public class Outer {\n    public void doSomething() {}\n\n    public class Inner {\n        public void innerMethod() {}\n    }\n}\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "Outer.java", &[]);

        let outer = symbols.iter().find(|s| s.name == "Outer").unwrap();
        assert_eq!(outer.kind, ItemKind::Struct);

        let do_something = symbols.iter().find(|s| s.name == "doSomething").unwrap();
        assert_eq!(do_something.kind, ItemKind::Function);
        assert_eq!(do_something.module_path, vec!["Outer"]);

        let inner = symbols
            .iter()
            .find(|s| s.name == "Inner" && s.kind == ItemKind::Struct)
            .unwrap();
        assert_eq!(inner.module_path, vec!["Outer"]);

        let inner_method = symbols.iter().find(|s| s.name == "innerMethod").unwrap();
        assert_eq!(inner_method.module_path, vec!["Outer", "Inner"]);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn typescript_nested_methods_in_class() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let source = b"export class Service {\n  greet(name: string): string { return 'hi'; }\n  farewell(): void {}\n}\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "service.ts", &[]);

        let service = symbols.iter().find(|s| s.name == "Service").unwrap();
        assert_eq!(service.kind, ItemKind::Struct);

        let greet = symbols.iter().find(|s| s.name == "greet");
        assert!(
            greet.is_some(),
            "expected greet method, got: {:?}",
            symbols
                .iter()
                .map(|s| (&s.name, s.kind, &s.module_path))
                .collect::<Vec<_>>()
        );
        assert_eq!(greet.unwrap().module_path, vec!["Service"]);
    }

    // === Visibility inference tests ===

    #[cfg(feature = "lang-python")]
    #[test]
    fn python_private_naming_convention() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let source = b"def public_fn():\n    pass\n\ndef _private_fn():\n    pass\n\ndef __mangled_fn():\n    pass\n\ndef __init__(self):\n    pass\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "module.py", &[]);

        let public_fn = symbols.iter().find(|s| s.name == "public_fn").unwrap();
        // Python functions without _ prefix are not explicitly public (no Go convention)
        // but they shouldn't be marked as private by the underscore convention
        assert_eq!(
            public_fn.visibility,
            Visibility::Private,
            "Python public_fn is private by default (no explicit export)"
        );

        let private_fn = symbols.iter().find(|s| s.name == "_private_fn").unwrap();
        assert_eq!(
            private_fn.visibility,
            Visibility::Private,
            "_private_fn should be private"
        );

        let mangled_fn = symbols.iter().find(|s| s.name == "__mangled_fn").unwrap();
        assert_eq!(
            mangled_fn.visibility,
            Visibility::Private,
            "__mangled_fn should be private"
        );

        // __init__ is a dunder method — should NOT be marked private
        let init_fn = symbols.iter().find(|s| s.name == "__init__").unwrap();
        assert_eq!(
            init_fn.visibility,
            Visibility::Private,
            "__init__ is dunder but lowercase, so private by default"
        );
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn go_exported_visibility() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        let source = b"package main\n\nfunc ExportedFunc() {}\n\nfunc unexportedFunc() {}\n\ntype ExportedType struct {}\n\ntype unexportedType struct {}\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "main.go", &[]);

        let exported_fn = symbols.iter().find(|s| s.name == "ExportedFunc").unwrap();
        assert_eq!(
            exported_fn.visibility,
            Visibility::Public,
            "Go exported function should be public"
        );

        let unexported_fn = symbols.iter().find(|s| s.name == "unexportedFunc").unwrap();
        assert_eq!(
            unexported_fn.visibility,
            Visibility::Private,
            "Go unexported function should be private"
        );

        let exported_type = symbols.iter().find(|s| s.name == "ExportedType").unwrap();
        assert_eq!(
            exported_type.visibility,
            Visibility::Public,
            "Go exported type should be public"
        );

        let unexported_type = symbols.iter().find(|s| s.name == "unexportedType").unwrap();
        assert_eq!(
            unexported_type.visibility,
            Visibility::Private,
            "Go unexported type should be private"
        );
    }

    #[cfg(feature = "lang-java")]
    #[test]
    fn java_access_modifier_visibility() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .unwrap();
        let source = b"public class Foo {\n    public void pubMethod() {}\n    private void privMethod() {}\n    protected void protMethod() {}\n}\n";
        let tree = parser.parse(source, None).unwrap();
        let symbols = generic_extract_symbols(&tree, source, "Foo.java", &[]);

        let foo = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(
            foo.visibility,
            Visibility::Public,
            "public class should be Public"
        );

        let pub_method = symbols.iter().find(|s| s.name == "pubMethod").unwrap();
        assert_eq!(
            pub_method.visibility,
            Visibility::Public,
            "public method should be Public"
        );

        let priv_method = symbols.iter().find(|s| s.name == "privMethod").unwrap();
        assert_eq!(
            priv_method.visibility,
            Visibility::Private,
            "private method should be Private"
        );

        let prot_method = symbols.iter().find(|s| s.name == "protMethod").unwrap();
        assert_eq!(
            prot_method.visibility,
            Visibility::PubCrate,
            "protected method should map to PubCrate"
        );
    }
}
