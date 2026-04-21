//! C-specific AST walker refinement.
//!
//! Handles C-specific constructs: function definitions, struct/enum specifiers,
//! typedefs, and preprocessor macros. Walks the `declarator` chain for
//! function definitions to extract the function name.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// C language refinement.
pub struct CRefinement;

impl LanguageRefinement for CRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_definition" | "preproc_function_def" => Some(ItemKind::Function),
            "struct_specifier" => Some(ItemKind::Struct),
            "enum_specifier" => Some(ItemKind::Enum),
            "type_definition" => Some(ItemKind::Type),
            "preproc_def" => Some(ItemKind::Const),
            // Skip these
            "preproc_include" | "preproc_ifdef" | "preproc_ifndef" | "preproc_endif"
            | "preproc_else" | "preproc_if" | "declaration" | "comment" => None,
            _ => return None, // Delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let kind = node.kind();

        // For function_definition, walk the declarator chain:
        // function_definition → declarator (function_declarator) → declarator (identifier)
        if kind == "function_definition"
            && let Some(declarator) = node.child_by_field_name("declarator")
        {
            return extract_name_from_declarator(&declarator, source);
        }

        // Preprocessor macros use `name` field
        if (kind == "preproc_def" || kind == "preproc_function_def")
            && let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            return Some(text.to_string());
        }

        // Standard `name` field for struct_specifier, enum_specifier, etc.
        if let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            return Some(text.to_string());
        }

        None
    }

    fn language_name(&self) -> &'static str {
        "c"
    }
}

/// Walk a C/C++ declarator chain to find the innermost identifier name.
///
/// In tree-sitter-c, a function definition's `declarator` field points to a
/// `function_declarator`, whose own `declarator` field points to an `identifier`
/// (or `pointer_declarator` → `function_declarator` → `identifier`, etc.).
pub(super) fn extract_name_from_declarator(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    // If this node is a plain identifier, return its text
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        return node.utf8_text(source).ok().map(String::from);
    }

    // If this node has a `declarator` child, recurse into it
    if let Some(inner) = node.child_by_field_name("declarator") {
        return extract_name_from_declarator(&inner, source);
    }

    // Fallback: try `name` field
    if let Some(name_node) = node.child_by_field_name("name") {
        return name_node.utf8_text(source).ok().map(String::from);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_c_items() {
        let r = CRefinement;
        assert_eq!(
            r.classify_node_kind("function_definition"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("struct_specifier"),
            Some(Some(ItemKind::Struct))
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
            r.classify_node_kind("preproc_function_def"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("preproc_def"),
            Some(Some(ItemKind::Const))
        );
    }

    #[test]
    fn skips_c_noise() {
        let r = CRefinement;
        assert_eq!(r.classify_node_kind("preproc_include"), Some(None));
        assert_eq!(r.classify_node_kind("preproc_ifdef"), Some(None));
        assert_eq!(r.classify_node_kind("declaration"), Some(None));
        assert_eq!(r.classify_node_kind("comment"), Some(None));
    }

    #[test]
    fn delegates_unknown() {
        let r = CRefinement;
        assert_eq!(r.classify_node_kind("binary_expression"), None);
    }
}
