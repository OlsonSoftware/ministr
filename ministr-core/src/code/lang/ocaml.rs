//! OCaml language refinement (covers .ml and .mli).

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// OCaml language refinement.
pub struct OCamlRefinement;

impl LanguageRefinement for OCamlRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "type_definition" | "type_binding" => Some(ItemKind::Type),
            "module_definition" | "module_binding" => Some(ItemKind::Module),
            "module_type_definition" => Some(ItemKind::Trait),
            "value_definition" | "let_binding" | "method_definition" | "external" => {
                Some(ItemKind::Function)
            }
            "class_definition" | "class_binding" => Some(ItemKind::Struct),
            "open_module" | "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "ocaml"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_ocaml_items() {
        let r = OCamlRefinement;
        assert_eq!(
            r.classify_node_kind("type_definition"),
            Some(Some(ItemKind::Type))
        );
        assert_eq!(
            r.classify_node_kind("module_definition"),
            Some(Some(ItemKind::Module))
        );
        assert_eq!(
            r.classify_node_kind("value_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("open_module"), Some(None));
        assert_eq!(r.classify_node_kind("application_expression"), None);
    }
}
