//! C++-specific AST walker refinement.
//!
//! Handles C++-specific constructs: classes, namespaces, templates,
//! struct/enum specifiers, function definitions, and preprocessor macros.
//! Reuses the C declarator chain walker for function name extraction.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// C++ language refinement.
pub struct CppRefinement;

impl LanguageRefinement for CppRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_definition" | "preproc_function_def" => Some(ItemKind::Function),
            "class_specifier" | "struct_specifier" => Some(ItemKind::Struct),
            "namespace_definition" => Some(ItemKind::Module),
            "enum_specifier" => Some(ItemKind::Enum),
            "type_definition" | "alias_declaration" => Some(ItemKind::Type),
            "template_declaration" => {
                // template_declaration wraps the actual declaration.
                // We classify as Struct as a safe default — the name
                // extractor will look inside to find the real declaration.
                Some(ItemKind::Struct)
            }
            "preproc_def" => Some(ItemKind::Const),
            // Skip these
            "using_declaration" | "preproc_include" | "preproc_ifdef" | "preproc_ifndef"
            | "preproc_endif" | "preproc_else" | "preproc_if" | "declaration" | "comment" => None,
            _ => return None, // Delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let kind = node.kind();

        // For template_declaration, recurse into the wrapped declaration
        if kind == "template_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let child_kind = child.kind();
                if child_kind != "template_parameter_list" && child.is_named() {
                    return self.extract_name(&child, source);
                }
            }
        }

        // For function_definition, walk the declarator chain
        if kind == "function_definition"
            && let Some(declarator) = node.child_by_field_name("declarator")
        {
            return super::c::extract_name_from_declarator(&declarator, source);
        }

        // Preprocessor macros use `name` field
        if (kind == "preproc_def" || kind == "preproc_function_def")
            && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            return Some(text.to_string());
        }

        // Standard `name` field for class_specifier, struct_specifier,
        // namespace_definition, enum_specifier, alias_declaration, etc.
        if let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            return Some(text.to_string());
        }

        None
    }

    fn language_name(&self) -> &'static str {
        "cpp"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_cpp_items() {
        let r = CppRefinement;
        assert_eq!(
            r.classify_node_kind("function_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("class_specifier"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("struct_specifier"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("namespace_definition"),
            Some(Some(ItemKind::Module))
        );
        assert_eq!(
            r.classify_node_kind("enum_specifier"),
            Some(Some(ItemKind::Enum))
        );
        assert_eq!(
            r.classify_node_kind("type_definition"),
            Some(Some(ItemKind::Type))
        );
        assert_eq!(
            r.classify_node_kind("alias_declaration"),
            Some(Some(ItemKind::Type))
        );
        assert_eq!(
            r.classify_node_kind("template_declaration"),
            Some(Some(ItemKind::Struct))
        );
        assert_eq!(
            r.classify_node_kind("preproc_function_def"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("preproc_def"),
            Some(Some(ItemKind::Const))
        );
    }

    #[test]
    fn skips_cpp_noise() {
        let r = CppRefinement;
        assert_eq!(r.classify_node_kind("using_declaration"), Some(None));
        assert_eq!(r.classify_node_kind("preproc_include"), Some(None));
        assert_eq!(r.classify_node_kind("declaration"), Some(None));
        assert_eq!(r.classify_node_kind("comment"), Some(None));
    }

    #[test]
    fn delegates_unknown() {
        let r = CppRefinement;
        assert_eq!(r.classify_node_kind("binary_expression"), None);
    }
}
