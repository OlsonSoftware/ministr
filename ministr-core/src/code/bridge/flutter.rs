//! Flutter platform-channel bridge extractor.
//!
//! Flutter talks to native code over named channels. The *same channel
//! name string* appears on both sides:
//!
//! - **Dart side** (`dart`) — `MethodChannel('com.x/foo')` /
//!   `EventChannel(...)` / `BasicMessageChannel(...)`. Treated as the
//!   consumer → [`EndpointRole::Import`].
//! - **Native side** (`kotlin`/`java`/`swift`/`objc`) — the matching
//!   `MethodChannel(messenger, "com.x/foo")` /
//!   `FlutterMethodChannel(name: "com.x/foo", ...)` registration.
//!   Treated as the provider → [`EndpointRole::Export`].
//!
//! Binding key = the channel-name string (quotes stripped). Channel
//! names are distinctive (reverse-DNS-ish), so name-only matching has a
//! low false-positive rate; unmatched endpoints never link.
//!
//! The construction syntax differs per language, so rather than model
//! five grammars we use a bounded ancestor-text heuristic: a string
//! literal whose nearby enclosing expression mentions a channel
//! constructor is a channel name. This mirrors the deliberately
//! name-only approach of the gRPC extractor.
//!
//! Implements [`BridgeExtractor`]; register with a
//! [`BridgeLinker`](super::linker::BridgeLinker).

use super::util::{node_line, node_text};
use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, ConfidenceLevel, EndpointRole};

const CHANNEL_CTORS: &[&str] = &[
    "MethodChannel",
    "EventChannel",
    "BasicMessageChannel",
    "FlutterMethodChannel",
    "FlutterEventChannel",
    "FlutterBasicMessageChannel",
];

/// Max enclosing-expression source length to consider when deciding
/// whether a string literal is a channel name. Keeps the heuristic from
/// matching a string against a channel constructor elsewhere in a large
/// ancestor (e.g. a whole class body).
const MAX_CTX: usize = 240;

/// Extracts Flutter platform-channel bindings.
pub struct FlutterChannelExtractor;

impl BridgeExtractor for FlutterChannelExtractor {
    fn bridge_kind(&self) -> BridgeKind {
        BridgeKind::FlutterChannel
    }

    fn applicable_languages(&self) -> &[&str] {
        &["dart", "kotlin", "java", "swift", "objc"]
    }

    fn extract_endpoints(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        file_path: &str,
        language: &str,
    ) -> Vec<BridgeEndpoint> {
        let role = if language == "dart" {
            EndpointRole::Import
        } else {
            EndpointRole::Export
        };
        let mut endpoints = Vec::new();
        let mut seen: std::collections::BTreeSet<(String, u32)> = std::collections::BTreeSet::new();
        let mut cursor = tree.walk();
        walk(&mut cursor, &mut |node| {
            if !node.kind().contains("string") || node.child_count() > 3 {
                return;
            }
            let raw = node_text(node, source);
            let chan = raw.trim_matches(['"', '\'', '`', '@']).trim();
            if chan.is_empty() || chan.len() > 200 {
                return;
            }
            if !near_channel_ctor(node, source) {
                return;
            }
            let line = node_line(node);
            if seen.insert((chan.to_string(), line)) {
                endpoints.push(BridgeEndpoint {
                    binding_key: chan.to_string(),
                    kind: BridgeKind::FlutterChannel,
                    role,
                    language: language.into(),
                    file_path: file_path.into(),
                    line,
                    symbol_name: chan.to_string(),
                    confidence: ConfidenceLevel::CaseTransformed.score(),
                });
            }
        });
        endpoints
    }
}

/// Walk up a few ancestors; if a bounded-size enclosing expression
/// mentions a channel constructor, this string is a channel name.
fn near_channel_ctor(node: &tree_sitter::Node<'_>, source: &[u8]) -> bool {
    let mut cur = node.parent();
    for _ in 0..6 {
        let Some(n) = cur else { break };
        let text = node_text(&n, source);
        if text.len() <= MAX_CTX && CHANNEL_CTORS.iter().any(|c| text.contains(c)) {
            return true;
        }
        cur = n.parent();
    }
    false
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
    fn flutter_dart_to_kotlin_link() {
        use super::super::linker::{BridgeLinker, SourceFile};
        let dart = "final c = MethodChannel('com.example/battery');\n";
        let kotlin = "val ch = MethodChannel(messenger, \"com.example/battery\")\n";
        let dt = parse("dart", dart);
        let kt = parse("kotlin", kotlin);
        let mut linker = BridgeLinker::new();
        linker.register(Box::new(FlutterChannelExtractor));
        let files = [
            SourceFile {
                file_path: "main.dart",
                language: "dart",
                tree: &dt,
                source: dart.as_bytes(),
            },
            SourceFile {
                file_path: "MainActivity.kt",
                language: "kotlin",
                tree: &kt,
                source: kotlin.as_bytes(),
            },
        ];
        let links = linker.extract_and_link(&files);
        assert!(
            links.iter().any(|l| l.kind == BridgeKind::FlutterChannel
                && l.export.binding_key == "com.example/battery"),
            "links: {links:?}"
        );
    }
}
