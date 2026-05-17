//! CSS / SCSS language refinement.
//!
//! CSS has no functions or types; the useful "symbols" are rule selectors,
//! at-rules (`@media`, `@keyframes`), and SCSS mixins/functions. We map the
//! structural blocks to the closest [`ItemKind`] so they show up in
//! `ministr_symbols`.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// CSS language refinement.
pub struct CssRefinement;

impl LanguageRefinement for CssRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "rule_set" => Some(ItemKind::Struct),
            "keyframes_statement" | "media_statement" | "supports_statement" => {
                Some(ItemKind::Module)
            }
            // SCSS extensions
            "mixin_statement" | "function_statement" => Some(ItemKind::Function),
            "comment" | "import_statement" | "charset_statement" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "css"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_css_items() {
        let r = CssRefinement;
        assert_eq!(
            r.classify_node_kind("rule_set"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("keyframes_statement"),
            Some(Some(ItemKind::Module))
        );
        assert_eq!(r.classify_node_kind("comment"), Some(None));
        assert_eq!(r.classify_node_kind("declaration"), None);
    }
}
