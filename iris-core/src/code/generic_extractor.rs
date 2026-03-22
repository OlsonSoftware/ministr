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
/// use iris_core::code::generic_extract_symbols;
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
            if classify_node_kind(inner.kind()).is_some() {
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

    // Extend byte range to include preceding doc comments
    let byte_start = doc_comment_start_byte_generic(node, source).unwrap_or(node.start_byte());

    symbols.push(Symbol {
        name,
        kind: item_kind,
        visibility,
        signature,
        doc_comment,
        file_path: file_path.to_string(),
        byte_range: byte_start..node.end_byte(),
        module_path: module_path.iter().map(|s| (*s).to_string()).collect(),
    });
}

/// Whether a node kind is a wrapper that should be unwrapped to find declarations.
fn is_wrapper_node(kind: &str) -> bool {
    matches!(
        kind,
        "export_statement" | "export_declaration" | "declaration_list"
    )
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

        "struct_item" | "struct_specifier" => Some(ItemKind::Struct),

        "enum_item" | "enum_specifier" | "enum_declaration" => Some(ItemKind::Enum),

        "trait_item" => Some(ItemKind::Trait),

        "interface_declaration" | "interface_definition" | "abstract_type_declaration" => {
            Some(ItemKind::Trait)
        }

        "impl_item" => Some(ItemKind::Impl),

        "mod_item" | "module_declaration" | "package_declaration" | "module" => {
            Some(ItemKind::Module)
        }

        "type_item" | "type_alias_declaration" | "type_declaration" | "type_spec" => {
            Some(ItemKind::Type)
        }

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
            if name_node.kind() == "function_declarator" || name_node.kind() == "pointer_declarator"
            {
                if let Some(inner) = name_node.child_by_field_name("declarator") {
                    if let Ok(text) = inner.utf8_text(source) {
                        let text = text.trim();
                        if !text.is_empty() {
                            return text.to_string();
                        }
                    }
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
    if node.kind() == "decorated_definition" {
        if let Some(definition) = node.child_by_field_name("definition") {
            return extract_name_generic(&definition, source);
        }
    }

    // For Go type_declaration: look inside the type_spec child
    if node.kind() == "type_declaration" {
        let mut inner = node.walk();
        for child in node.children(&mut inner) {
            if child.kind() == "type_spec" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(text) = name_node.utf8_text(source) {
                        return text.trim().to_string();
                    }
                }
            }
        }
    }

    // For Go const_declaration: look inside const_spec child
    if node.kind() == "const_declaration" {
        let mut inner = node.walk();
        for child in node.children(&mut inner) {
            if child.kind() == "const_spec" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(text) = name_node.utf8_text(source) {
                        return text.trim().to_string();
                    }
                }
            }
        }
    }

    // Fallback: look for first named child that is an identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "type_identifier" {
            if let Ok(text) = child.utf8_text(source) {
                return text.trim().to_string();
            }
        }
    }

    "<unknown>".to_string()
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

    // Check for Go exported names (capitalized first letter)
    // Check for access modifiers as children
    let mut cursor = node.walk();
    let text = node.utf8_text(source).unwrap_or("");
    for child in node.children(&mut cursor) {
        let child_kind = child.kind();
        if child_kind == "visibility_modifier"
            || child_kind == "access_modifier"
            || child_kind == "modifiers"
        {
            let mod_text = child.utf8_text(source).unwrap_or("");
            if mod_text.contains("public") || mod_text.contains("pub") {
                return Visibility::Public;
            } else if mod_text.contains("private") {
                return Visibility::Private;
            } else if mod_text.contains("protected") || mod_text.contains("internal") {
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

    // Python: check for decorated_definition wrapping
    if node.kind() == "decorated_definition" {
        if let Some(def) = node.child_by_field_name("definition") {
            return detect_visibility_generic(&def, source);
        }
    }

    Visibility::Private
}

/// Extract a signature (declaration without body) from a node.
fn extract_signature_generic(node: &tree_sitter::Node, source: &[u8]) -> String {
    let text = node.utf8_text(source).unwrap_or("");

    // For decorated definitions, use the inner definition
    if node.kind() == "decorated_definition" {
        if let Some(def) = node.child_by_field_name("definition") {
            return extract_signature_generic(&def, source);
        }
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
            if let Some(content) = strip_block_doc_comment(text) {
                if !content.is_empty() {
                    doc_lines.push(content);
                }
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
}
