//! cgo bridge extractor — the Go ↔ C boundary.
//!
//! Detects the canonical "Go calls C" cgo pattern:
//!
//! - **C exports** — top-level C `function_definition`s. cgo lets Go call
//!   any C function in the preamble / linked translation units.
//! - **Go imports** — `C.func(...)` selector expressions (the `import "C"`
//!   pseudo-package). The binding key is the C function name.
//!
//! Only links that actually pair (a C function that Go calls via `C.`)
//! survive the linker, so unmatched C definitions add no noise.
//!
//! Implements [`BridgeExtractor`]; register with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::util::{node_line, node_text};
use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

/// Extracts cgo bindings from Go and C source files.
pub struct CgoExtractor;

impl BridgeExtractor for CgoExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::Cgo
    }

    fn applicable_languages(&self) -> &[&str] {
        &["go", "c"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        match language {
            "go" => extract_go_c_calls(tree, source, file_path),
            "c" => extract_c_functions(tree, source, file_path),
            _ => Vec::new(),
        }
    }
}

/// Go side: every `C.name` selector expression → Import endpoint `name`.
fn extract_go_c_calls(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk(&mut cursor, &mut |node| {
        if node.kind() != "selector_expression" {
            return;
        }
        let Some(operand) = node.child_by_field_name("operand") else {
            return;
        };
        if operand.kind() != "identifier" || node_text(&operand, source) != "C" {
            return;
        }
        let Some(field) = node.child_by_field_name("field") else {
            return;
        };
        let name = node_text(&field, source);
        if name.is_empty() {
            return;
        }
        endpoints.push(BridgeEndpoint {
            binding_key: name.clone(),
            kind: BridgeKind::Cgo,
            role: EndpointRole::Import,
            language: "go".into(),
            file_path: file_path.into(),
            line: node_line(node),
            symbol_name: name,
            confidence: ConfidenceLevel::Exact.score(),
        });
    });
    endpoints
}

/// C side: every top-level `function_definition` → Export endpoint.
fn extract_c_functions(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for node in root.children(&mut cursor) {
        if node.kind() != "function_definition" {
            continue;
        }
        if let Some(name) = c_function_name(&node, source) {
            endpoints.push(BridgeEndpoint {
                binding_key: name.clone(),
                kind: BridgeKind::Cgo,
                role: EndpointRole::Export,
                language: "c".into(),
                file_path: file_path.into(),
                line: node_line(&node),
                symbol_name: name,
                confidence: ConfidenceLevel::Exact.score(),
            });
        }
    }
    endpoints
}

/// Peel a C `function_definition`'s declarator chain down to the
/// `function_declarator`'s identifier.
fn c_function_name(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut decl = node.child_by_field_name("declarator")?;
    while decl.kind() != "function_declarator" {
        decl = decl.child_by_field_name("declarator")?;
    }
    let ident = decl.child_by_field_name("declarator")?;
    let text = node_text(&ident, source);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Pre-order walk applying `visit` to every node.
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
            .expect("registered");
        let mut p = tree_sitter::Parser::new();
        p.set_language(l).unwrap();
        p.parse(src, None).unwrap()
    }

    #[test]
    fn go_c_call_is_import() {
        let src = "package main\n\n// #include \"m.h\"\nimport \"C\"\n\nfunc run() {\n\tC.do_work(1)\n\tx := C.compute()\n\t_ = x\n}\n";
        let t = parse("go", src);
        let eps = CgoExtractor.extract_endpoints(&t, src.as_bytes(), "main.go", "go");
        let names: Vec<_> = eps.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"do_work"), "got {names:?}");
        assert!(names.contains(&"compute"), "got {names:?}");
        assert!(eps.iter().all(|e| e.role == EndpointRole::Import));
    }

    #[test]
    fn c_function_is_export() {
        let src = "int do_work(int n) { return n + 1; }\nstatic int compute(void) { return 42; }\n";
        let t = parse("c", src);
        let eps = CgoExtractor.extract_endpoints(&t, src.as_bytes(), "m.c", "c");
        let names: Vec<_> = eps.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(names.contains(&"do_work"), "got {names:?}");
        assert!(names.contains(&"compute"), "got {names:?}");
        assert!(eps.iter().all(|e| e.role == EndpointRole::Export));
    }

    #[test]
    fn cgo_full_link() {
        use super::super::linker::{BridgeLinker, SourceFile};
        let go_src = "package main\nimport \"C\"\nfunc main() { C.do_work(1) }\n";
        let c_src = "int do_work(int n) { return n; }\n";
        let gt = parse("go", go_src);
        let ct = parse("c", c_src);
        let mut linker = BridgeLinker::new();
        linker.register(Box::new(CgoExtractor));
        let files = [
            SourceFile {
                file_path: "main.go",
                language: "go",
                tree: &gt,
                source: go_src.as_bytes(),
            },
            SourceFile {
                file_path: "m.c",
                language: "c",
                tree: &ct,
                source: c_src.as_bytes(),
            },
        ];
        let links = linker.extract_and_link(&files);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, BridgeKind::Cgo);
        assert_eq!(links[0].export.binding_key, "do_work");
        assert_eq!(links[0].export.language, "c");
        assert_eq!(links[0].import.language, "go");
    }
}
