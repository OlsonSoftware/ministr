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
//! Per-language: a language only produces occurrences when it has an entry in
//! [`identifier_kinds`]. FL1 graduated this from Rust-only to all 15 supported
//! code languages (the position→symbol substrate LSP-equivalence needs).

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
/// Returns an empty vec for languages without an [`identifier_kinds`] entry —
/// the index is per-language. All 15 supported code languages produce output
/// (FL1); config/markup grammars (json, yaml, toml, …) intentionally do not.
#[must_use]
pub fn extract_occurrences(tree: &Tree, source: &[u8], language: &str) -> Vec<Occurrence> {
    let kinds = identifier_kinds(language);
    if kinds.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    collect(tree.root_node(), source, kinds, &mut out);
    out
}

/// The tree-sitter LEAF node kinds that represent an identifier *occurrence*
/// for each language, keyed by the canonical grammar name (see
/// [`crate::code::GrammarRegistry`]).
///
/// Only true terminal identifier kinds belong here — composite nodes like
/// `scoped_identifier` / `scoped_type_identifier` / `qualified_name` are
/// deliberately omitted so [`collect`] recurses into them and records each of
/// their identifier *parts* individually (a click on any segment resolves).
#[must_use]
fn identifier_kinds(language: &str) -> &'static [&'static str] {
    match language {
        "rust" => &[
            "identifier",
            "type_identifier",
            "field_identifier",
            "shorthand_field_identifier",
        ],
        // Python, C#, and tree-sitter-kotlin-ng all emit a single `identifier`
        // leaf for every identifier (value names + type names alike).
        "python" | "csharp" | "kotlin" => &["identifier"],
        "javascript" => &[
            "identifier",
            "property_identifier",
            "shorthand_property_identifier",
            "private_property_identifier",
        ],
        // tsx shares TypeScript's identifier node kinds.
        "typescript" | "tsx" => &[
            "identifier",
            "property_identifier",
            "shorthand_property_identifier",
            "private_property_identifier",
            "type_identifier",
        ],
        "go" => &[
            "identifier",
            "field_identifier",
            "type_identifier",
            "package_identifier",
        ],
        "java" => &["identifier", "type_identifier"],
        "c" => &[
            "identifier",
            "field_identifier",
            "type_identifier",
            "statement_identifier",
        ],
        "cpp" => &[
            "identifier",
            "field_identifier",
            "type_identifier",
            "namespace_identifier",
            "statement_identifier",
        ],
        "ruby" => &[
            "identifier",
            "constant",
            "instance_variable",
            "class_variable",
            "global_variable",
        ],
        // Swift uses `simple_identifier` for value names + `type_identifier`
        // for type names (Kotlin-ng instead uses plain `identifier`, above).
        "swift" => &["simple_identifier", "type_identifier"],
        "scala" => &["identifier", "type_identifier", "operator_identifier"],
        // PHP names are `name`; `$`-variables are `variable_name`.
        "php" => &["name", "variable_name"],
        _ => &[],
    }
}

#[allow(clippy::cast_possible_truncation)] // source files are far under 4 GiB
fn collect(node: tree_sitter::Node<'_>, source: &[u8], kinds: &[&str], out: &mut Vec<Occurrence>) {
    // An identifier occurrence is a LEAF whose kind is one of `kinds`. The
    // `child_count() == 0` guard keeps a composite node (e.g. a hypothetical
    // grammar where a listed kind isn't terminal) from swallowing its parts —
    // we fall through and recurse instead.
    if node.child_count() == 0 && kinds.contains(&node.kind()) {
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
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, source, kinds, out);
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

    /// Parse `src` with a specific grammar (by canonical name) — used by the
    /// per-language occurrence tests. All grammars are compiled in under the
    /// default `lang-all` feature.
    fn parse_lang(name: &str, src: &str) -> Tree {
        let reg = crate::code::GrammarRegistry::global();
        let language = reg
            .language_by_name(name)
            .unwrap_or_else(|| panic!("grammar '{name}' not available"));
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(language)
            .unwrap_or_else(|e| panic!("set language '{name}': {e}"));
        parser.parse(src, None).expect("parse succeeded")
    }

    /// Assert every occurrence's byte span slices back to its own name, and
    /// that each expected identifier was captured at least once.
    fn assert_occurrences(language: &str, src: &str, expected: &[&str]) {
        let tree = parse_lang(language, src);
        let occ = extract_occurrences(&tree, src.as_bytes(), language);
        let names: Vec<&str> = occ.iter().map(|o| o.name.as_str()).collect();
        for want in expected {
            assert!(
                names.contains(want),
                "[{language}] expected identifier {want:?} not found; got {names:?}",
            );
        }
        for o in &occ {
            let slice = &src.as_bytes()[o.byte_start as usize..o.byte_end as usize];
            assert_eq!(
                std::str::from_utf8(slice).unwrap(),
                o.name,
                "[{language}] span/name mismatch for {o:?}",
            );
        }
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
        // index never had its non-definition occurrence). The scoped call
        // `MinistrConfig::new` records *both* segments (scoped_identifier is
        // not a leaf, so collect recurses into its parts).
        let mc = occ.iter().find(|o| o.name == "MinistrConfig").unwrap();
        assert_eq!(mc.line, 2);
    }

    #[test]
    fn unsupported_language_yields_no_occurrences() {
        // A registered-but-non-code grammar (json) has no identifier_kinds entry.
        let src = "{\n  \"key\": \"value\"\n}\n";
        let tree = parse_lang("json", src);
        assert!(extract_occurrences(&tree, src.as_bytes(), "json").is_empty());
    }

    #[test]
    fn python_occurrences() {
        assert_occurrences(
            "python",
            "def foo():\n    bar = Baz()\n    return bar.attr\n",
            &["foo", "bar", "Baz", "attr"],
        );
    }

    #[test]
    fn javascript_occurrences() {
        assert_occurrences(
            "javascript",
            "function foo() {\n  const x = new Bar();\n  return obj.prop;\n}\n",
            &["foo", "x", "Bar", "obj", "prop"],
        );
    }

    #[test]
    fn typescript_occurrences() {
        assert_occurrences(
            "typescript",
            "interface Foo { x: number }\nconst a: Bar = baz();\n",
            &["Foo", "Bar", "baz", "a"],
        );
    }

    #[test]
    fn tsx_occurrences() {
        assert_occurrences(
            "tsx",
            "const App = () => { const v: Foo = bar(); return v; };\n",
            &["App", "Foo", "bar", "v"],
        );
    }

    #[test]
    fn go_occurrences() {
        assert_occurrences(
            "go",
            "package main\nfunc Foo() {\n\tx := Bar{}\n\t_ = x.Field\n}\n",
            &["main", "Foo", "x", "Bar", "Field"],
        );
    }

    #[test]
    fn java_occurrences() {
        assert_occurrences(
            "java",
            "class Foo {\n  void bar() {\n    Baz x = null;\n  }\n}\n",
            &["Foo", "bar", "Baz", "x"],
        );
    }

    #[test]
    fn c_occurrences() {
        assert_occurrences(
            "c",
            "int main() {\n  struct Foo f;\n  f.field = 1;\n}\n",
            &["main", "Foo", "f", "field"],
        );
    }

    #[test]
    fn cpp_occurrences() {
        assert_occurrences(
            "cpp",
            "namespace ns {\nclass Foo {\n  void bar();\n};\n}\n",
            &["ns", "Foo", "bar"],
        );
    }

    #[test]
    fn ruby_occurrences() {
        assert_occurrences(
            "ruby",
            "class Foo\n  def bar\n    @x = BAZ\n  end\nend\n",
            &["Foo", "bar", "BAZ"],
        );
    }

    #[test]
    fn csharp_occurrences() {
        assert_occurrences(
            "csharp",
            "class Foo {\n  void Bar() {\n    var x = new Baz();\n  }\n}\n",
            &["Foo", "Bar", "x", "Baz"],
        );
    }

    #[test]
    fn swift_occurrences() {
        assert_occurrences(
            "swift",
            "class Foo {\n  func bar() {\n    let x = Baz()\n  }\n}\n",
            &["Foo", "bar", "x", "Baz"],
        );
    }

    #[test]
    fn kotlin_occurrences() {
        assert_occurrences(
            "kotlin",
            "class Foo {\n  fun bar() {\n    val x = Baz()\n  }\n}\n",
            &["Foo", "bar", "x", "Baz"],
        );
    }

    #[test]
    fn scala_occurrences() {
        assert_occurrences(
            "scala",
            "class Foo {\n  def bar = {\n    val x = Baz\n    x\n  }\n}\n",
            &["Foo", "bar", "x", "Baz"],
        );
    }

    #[test]
    fn php_occurrences() {
        assert_occurrences(
            "php",
            "<?php\nclass Foo {\n  function bar() {\n    $x = new Baz();\n  }\n}\n",
            &["Foo", "bar", "Baz"],
        );
    }
}
