//! Ruby language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Ruby language refinement.
pub struct RubyRefinement;

impl LanguageRefinement for RubyRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "class" | "singleton_class" => Some(ItemKind::Struct),
            "module" => Some(ItemKind::Module),
            "method" | "singleton_method" => Some(ItemKind::Function),
            "comment" | "call" => None,
            _ => return None, // delegate to generic
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "ruby"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_ruby_items() {
        let r = RubyRefinement;
        assert_eq!(r.classify_node_kind("class"), Some(Some(ItemKind::Struct)));
        assert_eq!(r.classify_node_kind("module"), Some(Some(ItemKind::Module)));
        assert_eq!(
            r.classify_node_kind("method"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("singleton_method"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("comment"), Some(None));
        assert_eq!(r.classify_node_kind("if"), None);
    }
}
