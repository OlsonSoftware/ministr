//! Language-specific AST walker refinements.
//!
//! Each submodule provides a [`LanguageRefinement`] implementation that adds
//! language-specific node kind classification and name extraction on top of
//! the generic extractor.

mod c;
mod cpp;
mod go;
mod java;
mod kotlin;
mod python;
mod rust;
mod swift;
mod typescript;

use crate::code::ast_parser::ItemKind;

/// Language-specific refinement for symbol extraction.
///
/// Implementations override the generic node kind classification and name
/// extraction for languages that have non-standard tree-sitter node structures.
pub trait LanguageRefinement: Send + Sync {
    /// Classify a tree-sitter node kind into an [`ItemKind`].
    ///
    /// Returns `None` to delegate to the generic classifier.
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>>;

    /// Extract the name of an item from its AST node.
    ///
    /// Returns `None` to delegate to the generic name extractor.
    fn extract_name(&self, _node: &tree_sitter::Node, _source: &[u8]) -> Option<String> {
        None
    }

    /// The canonical language name.
    fn language_name(&self) -> &'static str;
}

/// Get the language refinement for a language name, if one exists.
#[must_use]
pub fn refinement_for(language: &str) -> Option<Box<dyn LanguageRefinement>> {
    match language {
        "rust" => Some(Box::new(rust::RustRefinement)),
        "python" => Some(Box::new(python::PythonRefinement)),
        "typescript" | "tsx" => Some(Box::new(typescript::TypeScriptRefinement)),
        "go" => Some(Box::new(go::GoRefinement)),
        "java" => Some(Box::new(java::JavaRefinement)),
        "c" => Some(Box::new(c::CRefinement)),
        "cpp" => Some(Box::new(cpp::CppRefinement)),
        "swift" => Some(Box::new(swift::SwiftRefinement)),
        "kotlin" => Some(Box::new(kotlin::KotlinRefinement)),
        _ => None,
    }
}
