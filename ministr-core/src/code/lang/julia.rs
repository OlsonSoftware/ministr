//! Julia language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Julia language refinement.
pub struct JuliaRefinement;

impl LanguageRefinement for JuliaRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "struct_definition" => Some(ItemKind::Struct),
            "abstract_definition" | "primitive_definition" => Some(ItemKind::Type),
            "function_definition" | "short_function_definition" | "macro_definition" => {
                Some(ItemKind::Function)
            }
            "module_definition" | "baremodule_definition" => Some(ItemKind::Module),
            "const_statement" => Some(ItemKind::Const),
            "import_statement" | "using_statement" | "line_comment" | "block_comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "julia"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_julia_items() {
        let r = JuliaRefinement;
        assert_eq!(
            r.classify_node_kind("struct_definition"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("function_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("module_definition"),
            Some(Some(ItemKind::Module))
        );
        assert_eq!(r.classify_node_kind("import_statement"), Some(None));
        assert_eq!(r.classify_node_kind("call_expression"), None);
    }
}
