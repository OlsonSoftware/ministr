//! Dart language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Dart language refinement.
pub struct DartRefinement;

impl LanguageRefinement for DartRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "class_definition" | "mixin_declaration" | "extension_declaration" => {
                Some(ItemKind::Struct)
            }
            "enum_declaration" => Some(ItemKind::Enum),
            "function_signature" | "method_signature" | "function_declaration"
            | "method_declaration" | "constructor_signature" | "getter_signature"
            | "setter_signature" => Some(ItemKind::Function),
            "type_alias" => Some(ItemKind::Type),
            "import_or_export" | "library_name" | "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "dart"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_dart_items() {
        let r = DartRefinement;
        assert_eq!(
            r.classify_node_kind("class_definition"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("enum_declaration"),
            Some(Some(ItemKind::Enum))
        );
        assert_eq!(
            r.classify_node_kind("function_signature"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("import_or_export"), Some(None));
        assert_eq!(r.classify_node_kind("if_statement"), None);
    }
}
