//! Protobuf-specific AST walker refinement.
//!
//! tree-sitter-proto exposes top-level `message`, `enum`, and `service`
//! nodes whose names live in dedicated `message_name` / `enum_name` /
//! `service_name` children (not a generic `name` field), and the bare
//! `message`/`service` kinds aren't caught by the generic heuristic
//! classifier. This refinement makes proto schemas first-class in the
//! symbol index (high value for gRPC / API discovery).

use crate::code::ast_parser::ItemKind;
use crate::code::lang::LanguageRefinement;

/// Protobuf language refinement.
pub struct ProtoRefinement;

impl LanguageRefinement for ProtoRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            // A message is a record type.
            "message" => Some(ItemKind::Struct),
            "enum" => Some(ItemKind::Enum),
            // A service is the closest analogue to a trait/interface — a
            // named set of callable rpc endpoints.
            "service" => Some(ItemKind::Trait),
            // Noise / non-symbol top-level nodes.
            "syntax" | "package" | "import" | "option" | "comment" => None,
            _ => return None, // delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        // message/enum/service names are in a `*_name` child node.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind().ends_with("_name")
                && let Ok(text) = child.utf8_text(source)
            {
                let t = text.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
        None
    }

    fn language_name(&self) -> &'static str {
        "proto"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_proto_items() {
        let r = ProtoRefinement;
        assert_eq!(r.classify_node_kind("message"), Some(Some(ItemKind::Struct)));
        assert_eq!(r.classify_node_kind("enum"), Some(Some(ItemKind::Enum)));
        assert_eq!(r.classify_node_kind("service"), Some(Some(ItemKind::Trait)));
        assert_eq!(r.classify_node_kind("syntax"), Some(None));
        assert_eq!(r.classify_node_kind("field"), None);
    }
}
