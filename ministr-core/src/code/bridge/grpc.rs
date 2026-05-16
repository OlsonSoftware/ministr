//! gRPC bridge extractor — `.proto` services ↔ generated client stubs.
//!
//! - **proto exports** — every `service Foo { ... }` declaration.
//!   Binding key = the service name.
//! - **stub imports** — references in Go/Python/TS/Java/Kotlin to a
//!   generated stub/client type for that service: `FooClient`,
//!   `FooStub`, `FooBlockingStub`, `FooServicer`, `NewFooClient`,
//!   `FooCoroutineStub`, … Binding key = the recovered service name.
//!
//! Service names are distinctive enough that name-only matching has a
//! low false-positive rate, and unmatched endpoints never link.
//!
//! Implements [`BridgeExtractor`]; register with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::util::{node_line, node_text};
use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

const STUB_SUFFIXES: &[&str] = &[
    "BlockingStub",
    "CoroutineStub",
    "FutureStub",
    "Servicer",
    "Client",
    "Stub",
];

/// Recover a gRPC service name from a generated stub/client identifier.
/// `GreeterClient` → `Greeter`; `NewGreeterClient` → `Greeter`;
/// `GreeterBlockingStub` → `Greeter`. Returns `None` if it doesn't look
/// like a stub identifier.
fn service_from_stub(ident: &str) -> Option<String> {
    let base = ident.strip_prefix("New").unwrap_or(ident);
    for suffix in STUB_SUFFIXES {
        if let Some(name) = base.strip_suffix(suffix)
            && !name.is_empty()
            && name.chars().next().is_some_and(char::is_uppercase)
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Extracts gRPC service/stub bindings.
pub struct GrpcExtractor;

impl BridgeExtractor for GrpcExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::Grpc
    }

    fn applicable_languages(&self) -> &[&str] {
        &["proto", "go", "python", "typescript", "tsx", "java", "kotlin"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        if language == "proto" {
            extract_proto_services(tree, source, file_path)
        } else {
            extract_stub_refs(tree, source, file_path, language)
        }
    }
}

fn extract_proto_services(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut cursor = tree.walk();
    walk(&mut cursor, &mut |node| {
        if node.kind() != "service" {
            return;
        }
        let mut c = node.walk();
        for ch in node.children(&mut c) {
            if ch.kind() == "service_name" {
                let name = node_text(&ch, source);
                if !name.is_empty() {
                    endpoints.push(BridgeEndpoint {
                        binding_key: name.clone(),
                        kind: BridgeKind::Grpc,
                        role: EndpointRole::Export,
                        language: "proto".into(),
                        file_path: file_path.into(),
                        line: node_line(node),
                        symbol_name: name,
                        confidence: ConfidenceLevel::CaseTransformed.score(),
                    });
                }
            }
        }
    });
    endpoints
}

fn extract_stub_refs(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
) -> Vec<BridgeEndpoint> {
    let mut endpoints = Vec::new();
    let mut seen: std::collections::BTreeSet<(String, u32)> = std::collections::BTreeSet::new();
    let mut cursor = tree.walk();
    walk(&mut cursor, &mut |node| {
        if node.child_count() != 0 {
            return; // leaf identifiers only
        }
        let kind = node.kind();
        if !(kind == "identifier" || kind == "type_identifier" || kind == "field_identifier") {
            return;
        }
        let text = node_text(node, source);
        if let Some(service) = service_from_stub(&text) {
            let line = node_line(node);
            if seen.insert((service.clone(), line)) {
                endpoints.push(BridgeEndpoint {
                    binding_key: service.clone(),
                    kind: BridgeKind::Grpc,
                    role: EndpointRole::Import,
                    language: language.into(),
                    file_path: file_path.into(),
                    line,
                    symbol_name: text,
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
            .expect("registered");
        let mut p = tree_sitter::Parser::new();
        p.set_language(l).unwrap();
        p.parse(src, None).unwrap()
    }

    #[test]
    fn service_from_stub_cases() {
        assert_eq!(service_from_stub("GreeterClient").as_deref(), Some("Greeter"));
        assert_eq!(
            service_from_stub("NewGreeterClient").as_deref(),
            Some("Greeter")
        );
        assert_eq!(
            service_from_stub("GreeterBlockingStub").as_deref(),
            Some("Greeter")
        );
        assert_eq!(service_from_stub("regularName"), None);
        assert_eq!(service_from_stub("Client"), None);
    }

    #[test]
    fn grpc_proto_to_go_link() {
        use super::super::linker::{BridgeLinker, SourceFile};
        let proto = "syntax=\"proto3\";\nservice Greeter { rpc SayHi(M) returns (M); }\nmessage M {}\n";
        let go = "package main\nfunc run(c GreeterClient) { _ = c }\n";
        let pt = parse("proto", proto);
        let gt = parse("go", go);
        let mut linker = BridgeLinker::new();
        linker.register(Box::new(GrpcExtractor));
        let files = [
            SourceFile {
                file_path: "svc.proto",
                language: "proto",
                tree: &pt,
                source: proto.as_bytes(),
            },
            SourceFile {
                file_path: "main.go",
                language: "go",
                tree: &gt,
                source: go.as_bytes(),
            },
        ];
        let links = linker.extract_and_link(&files);
        assert!(
            links
                .iter()
                .any(|l| l.kind == BridgeKind::Grpc && l.export.binding_key == "Greeter"),
            "links: {links:?}"
        );
    }
}
