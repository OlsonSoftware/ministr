//! Go-specific AST walker refinement.
//!
//! Handles Go-specific constructs: interfaces, method declarations with
//! receivers, exported names (uppercase), and type specifications.

use crate::code::ast_parser::ItemKind;
use crate::code::lang::LanguageRefinement;

/// Go language refinement.
pub struct GoRefinement;

impl LanguageRefinement for GoRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_declaration" | "method_declaration" => Some(ItemKind::Function),
            "type_declaration" => {
                // Go `type_declaration` wraps one or more `type_spec` nodes.
                // We classify at the declaration level; the name extractor
                // will look inside type_spec children.
                Some(ItemKind::Type)
            }
            "const_declaration" => Some(ItemKind::Const),
            // Skip these
            "import_declaration" | "package_clause" | "comment" | "var_declaration" => None,
            _ => return None, // Delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let kind = node.kind();

        // For type_declaration, extract from the first type_spec child
        if kind == "type_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_spec"
                    && let Some(name_node) = child.child_by_field_name("name")
                    && let Ok(text) = name_node.utf8_text(source)
                {
                    return Some(text.to_string());
                }
            }
        }

        // For method_declaration, extract the method name (not the receiver)
        if kind == "method_declaration"
            && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            return Some(text.to_string());
        }

        // Standard `name` field
        if let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            return Some(text.to_string());
        }

        None
    }

    fn language_name(&self) -> &'static str {
        "go"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_go_items() {
        let r = GoRefinement;
        assert_eq!(
            r.classify_node_kind("function_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("method_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("type_declaration"),
            Some(Some(ItemKind::Type))
        );
        assert_eq!(
            r.classify_node_kind("const_declaration"),
            Some(Some(ItemKind::Const))
        );
    }

    #[test]
    fn skips_go_imports() {
        let r = GoRefinement;
        assert_eq!(r.classify_node_kind("import_declaration"), Some(None));
        assert_eq!(r.classify_node_kind("package_clause"), Some(None));
    }

    #[test]
    fn delegates_unknown() {
        let r = GoRefinement;
        assert_eq!(r.classify_node_kind("short_var_declaration"), None);
    }
}
