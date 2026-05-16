//! C# language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// C# language refinement.
pub struct CSharpRefinement;

impl LanguageRefinement for CSharpRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "class_declaration" | "struct_declaration" | "record_declaration"
            | "record_struct_declaration" => Some(ItemKind::Struct),
            "interface_declaration" => Some(ItemKind::Trait),
            "enum_declaration" => Some(ItemKind::Enum),
            "method_declaration" | "constructor_declaration" | "local_function_statement"
            | "delegate_declaration" => Some(ItemKind::Function),
            "property_declaration" | "field_declaration" => Some(ItemKind::Const),
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                Some(ItemKind::Module)
            }
            "using_directive" | "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "csharp"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_csharp_items() {
        let r = CSharpRefinement;
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
            r.classify_node_kind("method_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("namespace_declaration"),
            Some(Some(ItemKind::Module))
        );
        assert_eq!(r.classify_node_kind("using_directive"), Some(None));
        assert_eq!(r.classify_node_kind("invocation_expression"), None);
    }
}
