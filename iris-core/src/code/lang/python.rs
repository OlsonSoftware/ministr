//! Python-specific AST walker refinement.
//!
//! Handles Python-specific constructs: classes with decorators, type hints,
//! and the `decorated_definition` wrapper node.

use crate::code::ast_parser::ItemKind;
use crate::code::lang::LanguageRefinement;

/// Python language refinement.
pub struct PythonRefinement;

impl LanguageRefinement for PythonRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_definition" => Some(ItemKind::Function),
            "class_definition" => Some(ItemKind::Struct),
            "decorated_definition" => {
                // A decorated_definition wraps a function or class with decorators.
                // We classify it based on what it wraps, but we return Struct as
                // a safe default — the name extractor will look inside.
                Some(ItemKind::Struct)
            }
            // Skip these
            "import_statement"
            | "import_from_statement"
            | "expression_statement"
            | "comment"
            | "assignment"
            | "augmented_assignment"
            | "if_statement"
            | "for_statement"
            | "while_statement"
            | "try_statement"
            | "with_statement" => None,
            _ => return None, // Delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let kind = node.kind();

        // For decorated_definition, look inside the wrapped definition
        if kind == "decorated_definition" {
            if let Some(definition) = node.child_by_field_name("definition") {
                return self.extract_name(&definition, source);
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
        "python"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_python_items() {
        let r = PythonRefinement;
        assert_eq!(
            r.classify_node_kind("function_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("class_definition"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("decorated_definition"),
            Some(Some(ItemKind::Struct))
        );
    }

    #[test]
    fn skips_python_statements() {
        let r = PythonRefinement;
        assert_eq!(r.classify_node_kind("import_statement"), Some(None));
        assert_eq!(r.classify_node_kind("expression_statement"), Some(None));
        assert_eq!(r.classify_node_kind("assignment"), Some(None));
    }

    #[test]
    fn delegates_unknown() {
        let r = PythonRefinement;
        assert_eq!(r.classify_node_kind("list_comprehension"), None);
    }
}
