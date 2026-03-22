//! TypeScript-specific AST walker refinement.
//!
//! Handles TypeScript-specific constructs: interfaces, type aliases, decorators,
//! export declarations, and ambient declarations.

use crate::code::ast_parser::ItemKind;
use crate::code::lang::LanguageRefinement;

/// TypeScript language refinement.
pub struct TypeScriptRefinement;

impl LanguageRefinement for TypeScriptRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_declaration"
            | "method_definition"
            | "arrow_function"
            | "generator_function_declaration" => Some(ItemKind::Function),
            "class_declaration" | "abstract_class_declaration" => Some(ItemKind::Struct),
            "interface_declaration" => Some(ItemKind::Trait),
            "type_alias_declaration" => Some(ItemKind::Type),
            "enum_declaration" => Some(ItemKind::Enum),
            "module" => Some(ItemKind::Module),
            "lexical_declaration" => Some(ItemKind::Const),
            // `export_statement` wraps other declarations — we skip it and
            // let the wrapped declaration be extracted directly.
            // Also skip imports, expression statements, etc.
            "export_statement"
            | "import_statement"
            | "import_declaration"
            | "expression_statement"
            | "comment" => None,
            _ => return None, // Delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let kind = node.kind();

        // For lexical declarations (const/let/var), extract the variable name
        if kind == "lexical_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(text) = name_node.utf8_text(source) {
                            return Some(text.to_string());
                        }
                    }
                }
            }
        }

        // Standard `name` field
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(text) = name_node.utf8_text(source) {
                return Some(text.to_string());
            }
        }

        None
    }

    fn language_name(&self) -> &'static str {
        "typescript"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_typescript_items() {
        let r = TypeScriptRefinement;
        assert_eq!(
            r.classify_node_kind("interface_declaration"),
            Some(Some(ItemKind::Trait))
        );
        assert_eq!(
            r.classify_node_kind("type_alias_declaration"),
            Some(Some(ItemKind::Type))
        );
        assert_eq!(
            r.classify_node_kind("class_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("enum_declaration"),
            Some(Some(ItemKind::Enum))
        );
    }

    #[test]
    fn export_statement_skipped() {
        let r = TypeScriptRefinement;
        assert_eq!(r.classify_node_kind("export_statement"), Some(None));
    }

    #[test]
    fn delegates_unknown() {
        let r = TypeScriptRefinement;
        assert_eq!(r.classify_node_kind("jsx_element"), None);
    }
}
