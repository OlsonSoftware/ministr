//! JavaScript language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// JavaScript language refinement.
pub struct JavaScriptRefinement;

impl LanguageRefinement for JavaScriptRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "class_declaration" | "class" => Some(ItemKind::Struct),
            "function_declaration"
            | "generator_function_declaration"
            | "method_definition"
            | "function_expression"
            | "arrow_function" => Some(ItemKind::Function),
            "import_statement" | "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "javascript"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_javascript_items() {
        let r = JavaScriptRefinement;
        assert_eq!(
            r.classify_node_kind("class_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("function_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("method_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("import_statement"), Some(None));
        assert_eq!(r.classify_node_kind("call_expression"), None);
    }
}
