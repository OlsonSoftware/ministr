//! Java-specific AST walker refinement.
//!
//! Handles Java-specific constructs: classes, interfaces, enums, annotations,
//! records, method and constructor declarations, and visibility modifiers.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Java language refinement.
pub struct JavaRefinement;

impl LanguageRefinement for JavaRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "method_declaration" | "constructor_declaration" => Some(ItemKind::Function),
            "class_declaration" | "record_declaration" => Some(ItemKind::Struct),
            "interface_declaration" => Some(ItemKind::Trait),
            "enum_declaration" => Some(ItemKind::Enum),
            "annotation_type_declaration" => Some(ItemKind::Type),
            // Skip these
            "import_declaration"
            | "package_declaration"
            | "field_declaration"
            | "static_initializer"
            | "comment"
            | "block_comment"
            | "line_comment" => None,
            _ => return None, // Delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        // Standard `name` field — works for class, interface, enum, annotation,
        // record, method, and constructor declarations.
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(text) = name_node.utf8_text(source) {
                return Some(text.to_string());
            }
        }

        None
    }

    fn language_name(&self) -> &'static str {
        "java"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_java_items() {
        let r = JavaRefinement;
        assert_eq!(
            r.classify_node_kind("class_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("interface_declaration"),
            Some(Some(ItemKind::Trait))
        );
        assert_eq!(
            r.classify_node_kind("enum_declaration"),
            Some(Some(ItemKind::Enum))
        );
        assert_eq!(
            r.classify_node_kind("annotation_type_declaration"),
            Some(Some(ItemKind::Type))
        );
        assert_eq!(
            r.classify_node_kind("record_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("method_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("constructor_declaration"),
            Some(Some(ItemKind::Function))
        );
    }

    #[test]
    fn skips_java_noise() {
        let r = JavaRefinement;
        assert_eq!(r.classify_node_kind("import_declaration"), Some(None));
        assert_eq!(r.classify_node_kind("package_declaration"), Some(None));
        assert_eq!(r.classify_node_kind("field_declaration"), Some(None));
        assert_eq!(r.classify_node_kind("static_initializer"), Some(None));
    }

    #[test]
    fn delegates_unknown() {
        let r = JavaRefinement;
        assert_eq!(r.classify_node_kind("binary_expression"), None);
    }
}
