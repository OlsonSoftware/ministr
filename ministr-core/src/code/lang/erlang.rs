//! Erlang language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Erlang language refinement.
pub struct ErlangRefinement;

impl LanguageRefinement for ErlangRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "fun_decl" | "function_clause" => Some(ItemKind::Function),
            "type_alias" | "opaque" | "nominal_type" => Some(ItemKind::Type),
            "record_decl" => Some(ItemKind::Struct),
            "module_attribute" => Some(ItemKind::Module),
            "comment" | "export_attribute" | "import_attribute"
            | "pp_include" | "pp_define" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "erlang"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_erlang_items() {
        let r = ErlangRefinement;
        assert_eq!(
            r.classify_node_kind("fun_decl"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("record_decl"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(r.classify_node_kind("comment"), Some(None));
        assert_eq!(r.classify_node_kind("call"), None);
    }
}
