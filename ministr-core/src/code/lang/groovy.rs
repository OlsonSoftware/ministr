//! Groovy / Gradle language refinement.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Groovy language refinement.
pub struct GroovyRefinement;

impl LanguageRefinement for GroovyRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "class_declaration" => Some(ItemKind::Struct),
            "interface_declaration" | "trait_declaration" => Some(ItemKind::Trait),
            "enum_declaration" => Some(ItemKind::Enum),
            "method_declaration" | "function_declaration"
            | "function_definition" => Some(ItemKind::Function),
            "import_declaration" | "package_declaration" | "comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "groovy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_groovy_items() {
        let r = GroovyRefinement;
        assert_eq!(
            r.classify_node_kind("class_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("method_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("import_declaration"), Some(None));
        assert_eq!(r.classify_node_kind("juxt_function_call"), None);
    }
}
