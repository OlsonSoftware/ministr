//! Zig language refinement.
//!
//! Zig models types as `const X = struct {...}` / `enum` / `union`, so most
//! symbols surface as variable declarations. The generic extractor picks up
//! the binding name; this refinement classifies the function/test nodes.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// Zig language refinement.
pub struct ZigRefinement;

impl LanguageRefinement for ZigRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "FnProto" | "function_declaration" | "TestDecl" => Some(ItemKind::Function),
            "ContainerDecl" | "struct_declaration" => Some(ItemKind::Struct),
            "comment" | "line_comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "zig"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_zig_items() {
        let r = ZigRefinement;
        assert_eq!(
            r.classify_node_kind("function_declaration"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("comment"), Some(None));
        assert_eq!(r.classify_node_kind("AssignExpr"), None);
    }
}
