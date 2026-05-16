//! PHP language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// PHP language refinement.
pub struct PhpRefinement;

impl LanguageRefinement for PhpRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "class_declaration" => Some(ItemKind::Struct),
            "interface_declaration" | "trait_declaration" => Some(ItemKind::Trait),
            "enum_declaration" => Some(ItemKind::Enum),
            "function_definition" | "method_declaration" => Some(ItemKind::Function),
            "namespace_definition" => Some(ItemKind::Module),
            "const_declaration" => Some(ItemKind::Const),
            "php_tag" | "comment" | "namespace_use_declaration" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "php"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_php_items() {
        let r = PhpRefinement;
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
            r.classify_node_kind("function_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("php_tag"), Some(None));
        assert_eq!(r.classify_node_kind("echo_statement"), None);
    }
}
