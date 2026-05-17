//! Bash / shell language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Bash language refinement.
pub struct BashRefinement;

impl LanguageRefinement for BashRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_definition" => Some(ItemKind::Function),
            "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "bash"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_bash_items() {
        let r = BashRefinement;
        assert_eq!(
            r.classify_node_kind("function_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("comment"), Some(None));
        assert_eq!(r.classify_node_kind("command"), None);
    }
}
