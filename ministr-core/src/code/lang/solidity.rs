//! Solidity language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Solidity language refinement.
pub struct SolidityRefinement;

impl LanguageRefinement for SolidityRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "contract_declaration" | "library_declaration" | "struct_declaration" => {
                Some(ItemKind::Struct)
            }
            "interface_declaration" => Some(ItemKind::Trait),
            "enum_declaration" => Some(ItemKind::Enum),
            "function_definition"
            | "modifier_definition"
            | "constructor_definition"
            | "fallback_receive_definition"
            | "event_definition" => Some(ItemKind::Function),
            "user_defined_type_definition" => Some(ItemKind::Type),
            "import_directive" | "pragma_directive" | "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "solidity"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_solidity_items() {
        let r = SolidityRefinement;
        assert_eq!(
            r.classify_node_kind("contract_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("interface_declaration"),
            Some(Some(ItemKind::Trait))
        );
        assert_eq!(
            r.classify_node_kind("function_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("pragma_directive"), Some(None));
        assert_eq!(r.classify_node_kind("expression_statement"), None);
    }
}
