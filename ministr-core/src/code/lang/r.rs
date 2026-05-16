//! R language refinement.
//!
//! R has no dedicated declaration nodes — functions are `<-` bindings whose
//! RHS is a `function_definition`. The generic extractor handles the binding
//! name; this refinement just classifies the function node itself.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// R language refinement.
pub struct RRefinement;

impl LanguageRefinement for RRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_definition" => Some(ItemKind::Function),
            "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "r"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_r_items() {
        let r = RRefinement;
        assert_eq!(
            r.classify_node_kind("function_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("comment"), Some(None));
        assert_eq!(r.classify_node_kind("call"), None);
    }
}
