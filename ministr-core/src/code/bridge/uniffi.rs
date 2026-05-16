//! UniFFI bridge extractor — Rust ↔ Swift/Kotlin/Python (mobile).
//!
//! - **Rust exports** — items annotated `#[uniffi::export]` or deriving
//!   `uniffi::Object` / `uniffi::Record` / `uniffi::Enum`, plus
//!   `#[uniffi::constructor]` / `#[uniffi::method]`.
//! - **Foreign imports** — Swift/Kotlin/Python import statements that
//!   pull names out of the UniFFI-generated binding module (heuristic,
//!   mirroring the PyO3 import side). Binding key = symbol name.
//!
//! Implements [`BridgeExtractor`]; register with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::util::{has_rust_attribute_before, node_line, node_text, rust_item_name};
use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

const UNIFFI_ATTRS: &[&str] = &[
    "uniffi::export",
    "uniffi::Object",
    "uniffi::Record",
    "uniffi::Enum",
    "uniffi::Error",
    "uniffi::constructor",
    "uniffi::method",
];

/// Extracts UniFFI bindings.
pub struct UniffiExtractor;

impl BridgeExtractor for UniffiExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::UniFfi
    }

    fn applicable_languages(&self) -> &[&str] {
        &["rust", "swift", "kotlin", "python"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        match language {
            "rust" => extract_rust(tree, source, file_path),
            "swift" | "kotlin" | "python" => extract_foreign(tree, source, file_path, language),
            _ => Vec::new(),
        }
    }
}

fn extract_rust(tree: &tree_sitter::Tree, source: &[u8], file_path: &str) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk(&mut cursor, &mut |node| {
        let kind = node.kind();
        if !matches!(
            kind,
            "function_item" | "struct_item" | "enum_item" | "impl_item"
        ) {
            return;
        }
        if !has_rust_attribute_before(node, source, UNIFFI_ATTRS) {
            return;
        }
        if let Some(name) = rust_item_name(node, source) {
            endpoints.push(BridgeEndpoint {
                binding_key: name.clone(),
                kind: BridgeKind::UniFfi,
                role: EndpointRole::Export,
                language: "rust".into(),
                file_path: file_path.into(),
                line: node_line(node),
                symbol_name: name,
                confidence: ConfidenceLevel::CaseTransformed.score(),
            });
        }
    });
    endpoints
}

/// Foreign side: imported names from the generated UniFFI module.
/// Heuristic — only fires for import statements whose module path
/// mentions `uniffi` or ends in a generated-looking name. Unmatched
/// imports never link, so over-capture is harmless.
fn extract_foreign(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk(&mut cursor, &mut |node| {
        let kind = node.kind();
        let is_import = matches!(
            kind,
            "import_declaration"      // swift
                | "import"            // kotlin
                | "import_from_statement" // python
                | "import_statement"
        );
        if !is_import {
            return;
        }
        let text = node_text(node, source);
        if !text.to_lowercase().contains("uniffi") && !text.contains("generated") {
            return;
        }
        // Pull identifier-ish tail segments as candidate binding keys.
        for tok in text
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty())
        {
            if tok == "import" || tok == "from" || tok == "uniffi" {
                continue;
            }
            if tok.chars().next().is_some_and(char::is_uppercase) {
                endpoints.push(BridgeEndpoint {
                    binding_key: tok.to_string(),
                    kind: BridgeKind::UniFfi,
                    role: EndpointRole::Import,
                    language: language.into(),
                    file_path: file_path.into(),
                    line: node_line(node),
                    symbol_name: tok.to_string(),
                    confidence: ConfidenceLevel::Fuzzy.score(),
                });
            }
        }
    });
    endpoints
}

fn walk(cursor: &mut tree_sitter::TreeCursor<'_>, visit: &mut dyn FnMut(&tree_sitter::Node<'_>)) {
    loop {
        visit(&cursor.node());
        if cursor.goto_first_child() {
            walk(cursor, visit);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::GrammarRegistry;

    fn parse(lang: &str, src: &str) -> tree_sitter::Tree {
        let l = GrammarRegistry::global()
            .language_by_name(lang)
            .unwrap_or_else(|| {
                // Rust isn't in the registry by name; use the crate directly.
                panic!("use parse_rust for rust")
            });
        let mut p = tree_sitter::Parser::new();
        p.set_language(l).unwrap();
        p.parse(src, None).unwrap()
    }

    fn parse_rust(src: &str) -> tree_sitter::Tree {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        p.parse(src, None).unwrap()
    }

    #[test]
    fn rust_uniffi_export() {
        let src = "#[uniffi::export]\npub fn add(a: i32, b: i32) -> i32 { a + b }\n\n#[derive(uniffi::Object)]\npub struct Calc;\n";
        let t = parse_rust(src);
        let eps = UniffiExtractor.extract_endpoints(&t, src.as_bytes(), "lib.rs", "rust");
        let names: Vec<_> = eps.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"add"), "got {names:?}");
        assert!(names.contains(&"Calc"), "got {names:?}");
        assert!(eps.iter().all(|e| e.role == EndpointRole::Export));
    }

    #[test]
    fn kotlin_uniffi_import_links() {
        use super::super::linker::{BridgeLinker, SourceFile};
        let rust = "#[uniffi::export]\npub fn Greet() {}\n";
        let kt = "import com.example.uniffi.Greet\nfun main() { Greet() }\n";
        let rt = parse_rust(rust);
        let kt_t = parse("kotlin", kt);
        let mut linker = BridgeLinker::new();
        linker.register(Box::new(UniffiExtractor));
        let files = [
            SourceFile {
                file_path: "lib.rs",
                language: "rust",
                tree: &rt,
                source: rust.as_bytes(),
            },
            SourceFile {
                file_path: "Main.kt",
                language: "kotlin",
                tree: &kt_t,
                source: kt.as_bytes(),
            },
        ];
        let links = linker.extract_and_link(&files);
        assert!(
            links
                .iter()
                .any(|l| l.kind == BridgeKind::UniFfi && l.export.binding_key == "Greet"),
            "links: {links:?}"
        );
    }
}
