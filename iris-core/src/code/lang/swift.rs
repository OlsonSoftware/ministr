//! Swift-specific AST walker refinement.
//!
//! Handles Swift-specific constructs: classes, structs, protocols, enums,
//! extensions, functions, initializers, subscripts, typealiases, and
//! associated types. Designed for Tauri mobile plugin codebases.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Swift language refinement.
pub struct SwiftRefinement;

impl LanguageRefinement for SwiftRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_declaration"
            | "init_declaration"
            | "deinit_declaration"
            | "subscript_declaration" => Some(ItemKind::Function),
            "class_declaration" | "struct_declaration" => Some(ItemKind::Struct),
            "protocol_declaration" => Some(ItemKind::Trait),
            "enum_declaration" => Some(ItemKind::Enum),
            "extension_declaration" => Some(ItemKind::Impl),
            "typealias_declaration" | "associatedtype_declaration" => Some(ItemKind::Type),
            // Skip these
            "import_declaration" | "property_declaration" | "comment" | "multiline_comment" => None,
            _ => return None, // Delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        // Standard `name` field — works for class, struct, protocol, enum,
        // function, typealias, and associatedtype declarations.
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(text) = name_node.utf8_text(source) {
                return Some(text.to_string());
            }
        }

        // For extension_declaration, the extended type is in a `type_identifier`
        // child rather than a `name` field.
        if node.kind() == "extension_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" || child.kind() == "user_type" {
                    if let Ok(text) = child.utf8_text(source) {
                        return Some(text.to_string());
                    }
                }
            }
        }

        None
    }

    fn language_name(&self) -> &'static str {
        "swift"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_swift_items() {
        let r = SwiftRefinement;
        assert_eq!(
            r.classify_node_kind("function_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("init_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("deinit_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("subscript_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("class_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("struct_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("protocol_declaration"),
            Some(Some(ItemKind::Trait))
        );
        assert_eq!(
            r.classify_node_kind("enum_declaration"),
            Some(Some(ItemKind::Enum))
        );
        assert_eq!(
            r.classify_node_kind("extension_declaration"),
            Some(Some(ItemKind::Impl))
        );
        assert_eq!(
            r.classify_node_kind("typealias_declaration"),
            Some(Some(ItemKind::Type))
        );
        assert_eq!(
            r.classify_node_kind("associatedtype_declaration"),
            Some(Some(ItemKind::Type))
        );
    }

    #[test]
    fn skips_swift_noise() {
        let r = SwiftRefinement;
        assert_eq!(r.classify_node_kind("import_declaration"), Some(None));
        assert_eq!(r.classify_node_kind("property_declaration"), Some(None));
        assert_eq!(r.classify_node_kind("comment"), Some(None));
        assert_eq!(r.classify_node_kind("multiline_comment"), Some(None));
    }

    #[test]
    fn delegates_unknown() {
        let r = SwiftRefinement;
        assert_eq!(r.classify_node_kind("if_statement"), None);
    }
}
