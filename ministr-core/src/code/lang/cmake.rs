//! CMake language refinement.
//!
//! The useful symbols in CMake are user `function(...)` / `macro(...)`
//! definitions. Everything else is imperative command invocation.

use super::LanguageRefinement;
use crate::code::ast_parser::ItemKind;

/// CMake language refinement.
pub struct CMakeRefinement;

impl LanguageRefinement for CMakeRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        let result = match kind {
            "function_def" | "macro_def" => Some(ItemKind::Function),
            "line_comment" | "bracket_comment" => None,
            _ => return None,
        };
        Some(result)
    }

    fn language_name(&self) -> &'static str {
        "cmake"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_cmake_items() {
        let r = CMakeRefinement;
        assert_eq!(
            r.classify_node_kind("function_def"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(
            r.classify_node_kind("macro_def"),
            Some(Some(ItemKind::Function))
        );
        assert_eq!(r.classify_node_kind("line_comment"), Some(None));
        assert_eq!(r.classify_node_kind("normal_command"), None);
    }
}
