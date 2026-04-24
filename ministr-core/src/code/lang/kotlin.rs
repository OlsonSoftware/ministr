//! Kotlin-specific AST walker refinement.
//!
//! Handles Kotlin-specific constructs: data classes, objects, companion objects,
//! functions, type aliases, and annotations. Designed for Tauri mobile plugin
//! codebases.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Kotlin language refinement.
pub struct KotlinRefinement;

impl LanguageRefinement for KotlinRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_declaration" | "secondary_constructor" => Some(ItemKind::Function),
            "class_declaration" | "object_declaration" | "companion_object" => {
                Some(ItemKind::Struct)
            }
            "type_alias" => Some(ItemKind::Type),
            // Skip these
            "import_header"
            | "import_list"
            | "package_header"
            | "property_declaration"
            | "comment"
            | "multiline_comment" => None,
            _ => return None, // Delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let kind = node.kind();

        // For companion_object, the name is optional. Look for type_identifier child.
        if kind == "companion_object" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier"
                    && let Ok(text) = child.utf8_text(source)
                {
                    return Some(text.to_string());
                }
            }
            // Unnamed companion object — use "Companion" as the canonical name.
            return Some("Companion".to_string());
        }

        // Standard `name` field — works for function_declaration, class_declaration,
        // object_declaration, and type_alias.
        if let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            return Some(text.to_string());
        }

        // Fallback: look for simple_identifier child (common in Kotlin grammars).
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "simple_identifier"
                && let Ok(text) = child.utf8_text(source)
            {
                return Some(text.to_string());
            }
        }

        None
    }

    fn language_name(&self) -> &'static str {
        "kotlin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_kotlin_items() {
        let r = KotlinRefinement;
        assert_eq!(
            r.classify_node_kind("function_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("secondary_constructor"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("class_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("object_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("companion_object"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("type_alias"),
            Some(Some(ItemKind::Type))
        );
    }

    #[test]
    fn skips_kotlin_noise() {
        let r = KotlinRefinement;
        assert_eq!(r.classify_node_kind("import_header"), Some(None));
        assert_eq!(r.classify_node_kind("import_list"), Some(None));
        assert_eq!(r.classify_node_kind("package_header"), Some(None));
        assert_eq!(r.classify_node_kind("property_declaration"), Some(None));
    }

    #[test]
    fn delegates_unknown() {
        let r = KotlinRefinement;
        assert_eq!(r.classify_node_kind("for_statement"), None);
    }
}
