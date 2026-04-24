//! Rust-specific AST walker refinement.
//!
//! Handles Rust-specific node kinds: `trait_item`, `impl_item`, derive macros,
//! and the `_item` suffix convention.

use crate::code::ast_parser::ItemKind;
use crate::code::lang::LanguageRefinement;

/// Rust language refinement.
pub struct RustRefinement;

impl LanguageRefinement for RustRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        // Rust uses `_item` suffix convention
        let result = match kind {
            "function_item" | "macro_definition" => Some(ItemKind::Function),
            "struct_item" => Some(ItemKind::Struct),
            "enum_item" => Some(ItemKind::Enum),
            "trait_item" => Some(ItemKind::Trait),
            "impl_item" => Some(ItemKind::Impl),
            "mod_item" => Some(ItemKind::Module),
            "type_item" => Some(ItemKind::Type),
            "const_item" => Some(ItemKind::Const),
            "static_item" => Some(ItemKind::Static),
            // Skip these explicitly
            "use_declaration"
            | "extern_crate_declaration"
            | "attribute_item"
            | "inner_attribute_item"
            | "line_comment"
            | "block_comment" => None,
            _ => return None, // Delegate to generic
        };
        Some(result)
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        let kind = node.kind();

        // impl blocks: extract the type being implemented
        if kind == "impl_item" {
            return Some(
                node.child_by_field_name("type")
                    .and_then(|n| n.utf8_text(source).ok())
                    .unwrap_or("<unknown>")
                    .to_string(),
            );
        }

        // Most Rust items have a `name` field
        if let Some(name_node) = node.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
        {
            return Some(text.to_string());
        }

        None
    }

    fn language_name(&self) -> &'static str {
        "rust"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_rust_items() {
        let r = RustRefinement;
        assert_eq!(
            r.classify_node_kind("function_item"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("trait_item"),
            Some(Some(ItemKind::Trait))
        );
        assert_eq!(
            r.classify_node_kind("impl_item"),
            Some(Some(ItemKind::Impl))
        );
        assert_eq!(r.classify_node_kind("use_declaration"), Some(None));
    }

    #[test]
    fn delegates_unknown() {
        let r = RustRefinement;
        assert_eq!(r.classify_node_kind("some_unknown_thing"), None);
    }
}
