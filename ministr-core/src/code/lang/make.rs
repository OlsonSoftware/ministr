//! Makefile language refinement.
//!
//! Makefiles have no functions/types; the meaningful symbols are targets
//! (rules) and variable definitions.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Make language refinement.
pub struct MakeRefinement;

impl LanguageRefinement for MakeRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "rule" | "define_directive" => Some(ItemKind::Function),
            "variable_assignment" => Some(ItemKind::Const),
            "comment" | "include_directive" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "make"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_make_items() {
        let r = MakeRefinement;
        assert_eq!(r.classify_node_kind("rule"), Some(Some(ItemKind::Function)));
        assert_eq!(
            r.classify_node_kind("variable_assignment"),
            Some(Some(ItemKind::Const))
        );
        assert_eq!(r.classify_node_kind("comment"), Some(None));
        assert_eq!(r.classify_node_kind("shell_assignment"), None);
    }
}
