//! GraphQL SDL language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// GraphQL language refinement.
pub struct GraphQlRefinement;

impl LanguageRefinement for GraphQlRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "object_type_definition" | "input_object_type_definition" | "object_type_extension" => {
                Some(ItemKind::Struct)
            }
            "interface_type_definition" => Some(ItemKind::Trait),
            "enum_type_definition" => Some(ItemKind::Enum),
            "union_type_definition" | "scalar_type_definition" => Some(ItemKind::Type),
            "operation_definition" | "fragment_definition" | "directive_definition" => {
                Some(ItemKind::Function)
            }
            "schema_definition" => Some(ItemKind::Module),
            "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "graphql"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_graphql_items() {
        let r = GraphQlRefinement;
        assert_eq!(
            r.classify_node_kind("object_type_definition"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("interface_type_definition"),
            Some(Some(ItemKind::Trait))
        );
        assert_eq!(
            r.classify_node_kind("enum_type_definition"),
            Some(Some(ItemKind::Enum))
        );
        assert_eq!(r.classify_node_kind("comment"), Some(None));
        assert_eq!(r.classify_node_kind("field_definition"), None);
    }
}
