//! JNI bridge extractor — the Java/Kotlin ↔ C/C++ boundary.
//!
//! - **Java/Kotlin imports** — `native` (Java) / `external` (Kotlin)
//!   method declarations: the JVM side that calls into native code.
//!   Binding key = the method name.
//! - **C/C++ exports** — `JNIEXPORT` functions named
//!   `Java_pkg_Class_method`. Binding key = the trailing `method`
//!   segment (matches the Java declaration).
//!
//! Implements [`BridgeExtractor`]; register with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::util::{node_line, node_text};
use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

/// Extracts JNI bindings from Java/Kotlin and C/C++ source files.
pub struct JniExtractor;

impl BridgeExtractor for JniExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::Jni
    }

    fn applicable_languages(&self) -> &[&str] {
        &["java", "kotlin", "c", "cpp"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        match language {
            "java" | "kotlin" => extract_jvm_native(tree, source, file_path, language),
            "c" | "cpp" => extract_jni_exports(tree, source, file_path, language),
            _ => Vec::new(),
        }
    }
}

/// Java `native` / Kotlin `external` method declarations → Import.
fn extract_jvm_native(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk(&mut cursor, &mut |node| {
        let kind = node.kind();
        let is_decl = matches!(
            kind,
            "method_declaration" | "function_declaration" | "function_definition"
        );
        if !is_decl {
            return;
        }
        // The modifier set must mention native/external.
        let text = node_text(node, source);
        let head = text.split(['{', '(']).next().unwrap_or(&text);
        let is_native = head.split_whitespace().any(|w| w == "native")
            || head.split_whitespace().any(|w| w == "external");
        if !is_native {
            return;
        }
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = node_text(&name_node, source);
        if name.is_empty() {
            return;
        }
        endpoints.push(BridgeEndpoint {
            binding_key: name.clone(),
            kind: BridgeKind::Jni,
            role: EndpointRole::Import,
            language: language.into(),
            file_path: file_path.into(),
            line: node_line(node),
            symbol_name: name,
            confidence: ConfidenceLevel::CaseTransformed.score(),
        });
    });
    endpoints
}

/// C/C++ `Java_pkg_Class_method` functions → Export.
fn extract_jni_exports(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk(&mut cursor, &mut |node| {
        if node.kind() != "function_definition" {
            return;
        }
        let Some(full) = c_function_name(node, source) else {
            return;
        };
        let Some(rest) = full.strip_prefix("Java_") else {
            return;
        };
        // `Java_com_example_Foo_bar` → method `bar` (last segment).
        let method = rest.rsplit('_').next().unwrap_or(rest);
        if method.is_empty() {
            return;
        }
        endpoints.push(BridgeEndpoint {
            binding_key: method.to_string(),
            kind: BridgeKind::Jni,
            role: EndpointRole::Export,
            language: language.into(),
            file_path: file_path.into(),
            line: node_line(node),
            symbol_name: full,
            confidence: ConfidenceLevel::CaseTransformed.score(),
        });
    });
    endpoints
}

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
    fn java_native_is_import() {
        let src = "class Foo {\n  public native int doWork(int n);\n  public int regular() { return 0; }\n}\n";
        let t = parse("java", src);
        let eps = JniExtractor.extract_endpoints(&t, src.as_bytes(), "Foo.java", "java");
        assert_eq!(eps.len(), 1, "got {eps:?}");
        assert_eq!(eps[0].binding_key, "doWork");
        assert_eq!(eps[0].role, EndpointRole::Import);
    }

    #[test]
    fn c_jniexport_is_export() {
        let src = "int Java_com_example_Foo_doWork(void* env, void* obj, int n) { return n; }\n";
        let t = parse("c", src);
        let eps = JniExtractor.extract_endpoints(&t, src.as_bytes(), "native.c", "c");
        assert_eq!(eps.len(), 1, "got {eps:?}");
        assert_eq!(eps[0].binding_key, "doWork");
        assert_eq!(eps[0].role, EndpointRole::Export);
    }

    #[test]
    fn jni_full_link() {
        use super::super::linker::{BridgeLinker, SourceFile};
        let java = "class Foo { native int doWork(int n); }\n";
        let c = "int Java_Foo_doWork(void* e, void* o, int n){return n;}\n";
        let jt = parse("java", java);
        let ct = parse("c", c);
        let mut linker = BridgeLinker::new();
        linker.register(Box::new(JniExtractor));
        let files = [
            SourceFile {
                file_path: "Foo.java",
                language: "java",
                tree: &jt,
                source: java.as_bytes(),
            },
            SourceFile {
                file_path: "n.c",
                language: "c",
                tree: &ct,
                source: c.as_bytes(),
            },
        ];
        let links = linker.extract_and_link(&files);
        assert_eq!(links.len(), 1, "links: {links:?}");
        assert_eq!(links[0].kind, BridgeKind::Jni);
        assert_eq!(links[0].export.binding_key, "doWork");
    }
}
