//! Shared helpers for bridge extractors.
//!
//! Every detector in this directory needs the same handful of primitives:
//! read text out of a tree-sitter node, find the name identifier on a Rust
//! item, scan preceding attributes, peel quotes off a string literal, walk a
//! JS/TS import statement. Centralising them here keeps each per-bridge file
//! focused on its mechanism (PyO3 attrs, NAPI glue, Tauri invoke calls, …)
//! rather than re-implementing the same plumbing five times.
//!
//! All helpers are `pub(super)` — they are crate-private vocabulary for the
//! `bridge/` tree, not part of ministr-core's public API.

use tree_sitter::Node;

/// Extract text from a tree-sitter node.
///
/// Tree-sitter normally hands back valid UTF-8 (it tracks byte offsets in
/// the original source), but if a byte range straddles invalid UTF-8 we
/// fall back to a lossy decode of the underlying byte slice rather than
/// dropping the text entirely. The previous `unwrap_or("")` would silently
/// erase the node — bad on its own, and surprising given the doc-string.
pub(super) fn node_text(node: &Node<'_>, source: &[u8]) -> String {
    if let Ok(s) = node.utf8_text(source) {
        return s.to_string();
    }
    let bytes = source.get(node.byte_range()).unwrap_or(&[]);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Strip surrounding `"`, `'`, or `` ` `` quotes from a string literal.
///
/// Tree-sitter typically returns string-literal nodes with the quote
/// characters intact; downstream code wants the inner value.
pub(super) fn strip_quotes(s: &str) -> String {
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

/// Extract the name identifier from a Rust item node.
///
/// Handles `function_item` / `function_definition` (field `name` →
/// `identifier`), and `struct_item` / `enum_item` / `trait_item` (field
/// `name` → `type_identifier`). Returns `None` if no recognisable name is
/// found.
pub(super) fn rust_item_name(node: &Node<'_>, source: &[u8]) -> Option<String> {
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

/// Check whether any preceding sibling of `node` is an `attribute_item`
/// whose source text contains any of the given substrings.
///
/// Substring match (rather than parsed-attribute match) because the
/// existing detectors all matched by substring (`text.contains("napi")`,
/// `text.contains("pyfunction")`), and the false-positive rate is
/// acceptable in practice — attributes are short and distinctive.
///
/// Walks past comments transparently so doc comments between the attribute
/// and the item don't confuse the search.
pub(super) fn has_rust_attribute_before(
    node: &Node<'_>,
    source: &[u8],
    attr_substrings: &[&str],
) -> bool {
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        let kind = sibling.kind();
        if kind == "attribute_item" {
            let text = node_text(&sibling, source);
            if attr_substrings.iter().any(|s| text.contains(s)) {
                return true;
            }
        } else if kind != "line_comment" && kind != "block_comment" {
            break;
        }
        prev = sibling.prev_sibling();
    }
    false
}

/// Extract the module specifier string from a JS/TS `import_statement`.
///
/// e.g. for `import { foo } from './bar'` returns `Some("./bar")` (quotes stripped).
pub(super) fn import_module_path(node: &Node<'_>, source: &[u8]) -> Option<String> {
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

/// Extract the imported name from a JS/TS `import_specifier` node.
///
/// Handles `{ foo }` and `{ foo as bar }` — returns `foo` (the original
/// name, before any rename), per the convention that the binding key
/// matches the exported symbol.
pub(super) fn import_specifier_name(node: &Node<'_>, source: &[u8]) -> Option<String> {
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

/// Extract the first string-literal argument from an `arguments` node.
///
/// Used by JS/TS `require('./x')` and `invoke("name")` patterns, by
/// Python `ctypes.CDLL("path.so")` / `cffi.FFI().dlopen("path")` calls,
/// and by Rust call sites with `string_literal` nodes.
///
/// Recognises `string` (JS/TS), `string_literal` (Rust/Python), and
/// non-interpolated `template_string` (JS/TS backtick literals — but only
/// when there's no `${…}` substitution, since interpolated keys can't be
/// matched at static-analysis time).
///
/// Returns the inner value with surrounding quotes stripped.
pub(super) fn first_string_arg(args_node: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = args_node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        match child.kind() {
            "string" | "string_literal" => {
                return Some(strip_quotes(&node_text(&child, source)));
            }
            "template_string" => {
                let text = node_text(&child, source);
                if !text.contains("${") {
                    return Some(strip_quotes(&text));
                }
            }
            _ => {}
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// Return the 1-based start line of a node.
///
/// Centralises the `row + 1` cast so callers don't each need an
/// `#[allow(clippy::cast_possible_truncation)]` annotation.
#[allow(clippy::cast_possible_truncation)]
pub(super) fn node_line(node: &Node<'_>) -> u32 {
    node.start_position().row as u32 + 1
}

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

    #[test]
    fn strip_quotes_handles_all_three_kinds() {
        assert_eq!(strip_quotes("\"foo\""), "foo");
        assert_eq!(strip_quotes("'bar'"), "bar");
        assert_eq!(strip_quotes("`baz`"), "baz");
        assert_eq!(strip_quotes("unquoted"), "unquoted");
        assert_eq!(strip_quotes("  \"trim\"  "), "trim");
    }

    #[test]
    fn rust_item_name_extracts_function() {
        let src = "fn my_function() {}";
        let tree = parse_rust(src);
        let func = tree.root_node().child(0).unwrap();
        assert_eq!(func.kind(), "function_item");
        assert_eq!(
            rust_item_name(&func, src.as_bytes()).as_deref(),
            Some("my_function")
        );
    }

    #[test]
    fn rust_item_name_extracts_struct() {
        let src = "struct Foo;";
        let tree = parse_rust(src);
        let s = tree.root_node().child(0).unwrap();
        assert_eq!(s.kind(), "struct_item");
        assert_eq!(rust_item_name(&s, src.as_bytes()).as_deref(), Some("Foo"));
    }

    #[test]
    fn has_rust_attribute_before_finds_match() {
        let src = "#[napi]\nfn foo() {}";
        let tree = parse_rust(src);
        // The function_item is the second top-level child (attribute is first).
        let mut func = None;
        let mut cursor = tree.root_node().walk();
        for child in tree.root_node().children(&mut cursor) {
            if child.kind() == "function_item" {
                func = Some(child);
            }
        }
        let func = func.expect("function_item present");
        assert!(has_rust_attribute_before(&func, src.as_bytes(), &["napi"]));
        assert!(!has_rust_attribute_before(
            &func,
            src.as_bytes(),
            &["pyfunction"]
        ));
    }

    #[test]
    fn has_rust_attribute_before_walks_past_doc_comments() {
        let src = "#[pyfunction]\n/// doc\nfn foo() {}";
        let tree = parse_rust(src);
        let mut func = None;
        let mut cursor = tree.root_node().walk();
        for child in tree.root_node().children(&mut cursor) {
            if child.kind() == "function_item" {
                func = Some(child);
            }
        }
        let func = func.expect("function_item present");
        assert!(has_rust_attribute_before(
            &func,
            src.as_bytes(),
            &["pyfunction"]
        ));
    }
}
