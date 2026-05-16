//! Scala language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Scala language refinement.
pub struct ScalaRefinement;

impl LanguageRefinement for ScalaRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "class_definition" | "object_definition" | "case_class" => Some(ItemKind::Struct),
            "trait_definition" => Some(ItemKind::Trait),
            "enum_definition" => Some(ItemKind::Enum),
            "function_definition" | "function_declaration" => Some(ItemKind::Function),
            "val_definition" | "var_definition" => Some(ItemKind::Const),
            "type_definition" => Some(ItemKind::Type),
            "package_clause" | "package_object" => Some(ItemKind::Module),
            "import_declaration" | "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "scala"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_scala_items() {
        let r = ScalaRefinement;
        assert_eq!(
            r.classify_node_kind("class_definition"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("trait_definition"),
            Some(Some(ItemKind::Trait))
        );
        assert_eq!(
            r.classify_node_kind("function_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("package_clause"),
            Some(Some(ItemKind::Module))
        );
        assert_eq!(r.classify_node_kind("import_declaration"), Some(None));
        assert_eq!(r.classify_node_kind("call_expression"), None);
    }
}
