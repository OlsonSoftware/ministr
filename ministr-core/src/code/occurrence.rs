//! Occurrence-level extraction: every identifier token in a source file with
//! its byte + line/column span.
//!
//! Where [`refs`](super::refs) extracts a *curated* set of cross-references
//! (imports, impls, calls, type-usages) keyed by line, the occurrence index is
//! exhaustive: it records **every** identifier occurrence so the code browser
//! can resolve a click on *any* token, not just known definitions
//! (F-CodeExplorer v2). Occurrences are name-only here; resolution to a
//! `symbol_id` happens during ingestion against the stored symbol table.
//!
//! This is opt-in and per-language — only languages with an arm in
//! [`extract_occurrences`] produce output. Rust is the v1 language.

use tree_sitter::Tree;

/// A single identifier occurrence in a source file.
///
/// Byte offsets are into the raw UTF-8 source; `line` is 1-based and `col` is
/// the 0-based byte column (matching tree-sitter's `start_position`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// The identifier text (e.g. `MinistrConfig`, `read_file`).
    pub name: String,
    /// Start byte offset (inclusive) into the source.
    pub byte_start: u32,
    /// End byte offset (exclusive) into the source.
    pub byte_end: u32,
    /// 1-based line of the occurrence's first byte.
    pub line: u32,
    /// 0-based byte column of the occurrence's first byte.
    pub col: u32,
}

/// Extract identifier occurrences for the given language.
///
/// Returns an empty vec for languages without an occurrence arm — the index
/// is opt-in and lands per-language (see the module docs). Rust is supported.
#[must_use]
pub fn extract_occurrences(tree: &Tree, source: &[u8], language: &str) -> Vec<Occurrence> {
    match language {
        "rust" => {
            let mut out = Vec::new();
            collect_rust(tree.root_node(), source, &mut out);
            out
        }
        _ => Vec::new(),
    }
}

/// Identifier node kinds in the Rust tree-sitter grammar. These are leaves, so
/// recursion stops at each one.
const RUST_IDENT_KINDS: &[&str] = &[
    "identifier",
    "type_identifier",
    "field_identifier",
    "shorthand_field_identifier",
];

#[allow(clippy::cast_possible_truncation)] // source files are far under 4 GiB
fn collect_rust(node: tree_sitter::Node<'_>, source: &[u8], out: &mut Vec<Occurrence>) {
    if RUST_IDENT_KINDS.contains(&node.kind()) {
        if let Ok(name) = node.utf8_text(source) {
            let pos = node.start_position();
            out.push(Occurrence {
                name: name.to_string(),
                byte_start: node.start_byte() as u32,
                byte_end: node.end_byte() as u32,
                line: pos.row as u32 + 1,
                col: pos.column as u32,
            });
        }
        return; // identifiers are leaves
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust(child, source, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_tree(src: &str) -> Tree {
        super::super::AstParser::new()
            .parse(src.as_bytes())
            .unwrap()
    }

    #[test]
    fn extracts_rust_identifier_occurrences_with_spans() {
        let src = "fn main() {\n    let cfg = MinistrConfig::new();\n}\n";
        let tree = rust_tree(src);
        let occ = extract_occurrences(&tree, src.as_bytes(), "rust");

        // Every identifier occurrence is captured (main, cfg, MinistrConfig, new).
        let names: Vec<&str> = occ.iter().map(|o| o.name.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(names.contains(&"cfg"));
        assert!(names.contains(&"MinistrConfig"));
        assert!(names.contains(&"new"));

        // Spans are exact: the source slice at [byte_start, byte_end) equals the name.
        for o in &occ {
            let slice = &src.as_bytes()[o.byte_start as usize..o.byte_end as usize];
            assert_eq!(std::str::from_utf8(slice).unwrap(), o.name);
        }

        // MinistrConfig is the usage site on line 2 — clickable in v2 (the v1
        // index never had its non-definition occurrence).
        let mc = occ.iter().find(|o| o.name == "MinistrConfig").unwrap();
        assert_eq!(mc.line, 2);
    }

    #[test]
    fn unsupported_language_yields_no_occurrences() {
        let src = "fn main() {}";
        let tree = rust_tree(src);
        assert!(extract_occurrences(&tree, src.as_bytes(), "python").is_empty());
    }
}
