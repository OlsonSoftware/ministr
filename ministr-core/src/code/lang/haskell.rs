//! Haskell language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Haskell language refinement.
pub struct HaskellRefinement;

impl LanguageRefinement for HaskellRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "data_type" | "newtype" => Some(ItemKind::Struct),
            "class" => Some(ItemKind::Trait),
            "instance" => Some(ItemKind::Impl),
            "function" | "bind" | "signature" => Some(ItemKind::Function),
            "type_synonym" => Some(ItemKind::Type),
            "module" => Some(ItemKind::Module),
            "import" | "comment" | "pragma" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "haskell"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_haskell_items() {
        let r = HaskellRefinement;
        assert_eq!(
            r.classify_node_kind("data_type"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(r.classify_node_kind("class"), Some(Some(ItemKind::Trait)));
        assert_eq!(
            r.classify_node_kind("function"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("import"), Some(None));
        assert_eq!(r.classify_node_kind("expression"), None);
    }
}
