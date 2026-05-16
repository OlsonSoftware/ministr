//! Language-specific AST walker refinements.
//!
//! Each submodule provides a [`LanguageRefinement`] implementation that adds
//! language-specific node kind classification and name extraction on top of
//! the generic extractor.

mod bash;
mod c;
mod cmake;
mod cpp;
mod csharp;
mod css;
mod dart;
mod erlang;
mod go;
mod graphql;
mod groovy;
mod haskell;
mod hcl;
mod java;
mod javascript;
mod julia;
mod kotlin;
mod lua;
mod make;
mod ocaml;
mod php;
mod proto;
mod python;
mod r;
mod ruby;
mod rust;
mod scala;
mod solidity;
mod sql;
mod swift;
mod typescript;
mod zig;

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
        "proto" => Some(Box::new(proto::ProtoRefinement)),
        "hcl" => Some(Box::new(hcl::HclRefinement)),
        "sql" => Some(Box::new(sql::SqlRefinement)),
        "javascript" => Some(Box::new(javascript::JavaScriptRefinement)),
        "ruby" => Some(Box::new(ruby::RubyRefinement)),
        "php" => Some(Box::new(php::PhpRefinement)),
        "scala" => Some(Box::new(scala::ScalaRefinement)),
        "csharp" => Some(Box::new(csharp::CSharpRefinement)),
        "bash" => Some(Box::new(bash::BashRefinement)),
        "lua" => Some(Box::new(lua::LuaRefinement)),
        "haskell" => Some(Box::new(haskell::HaskellRefinement)),
        "ocaml" | "ocaml_interface" => Some(Box::new(ocaml::OCamlRefinement)),
        "dart" => Some(Box::new(dart::DartRefinement)),
        "r" => Some(Box::new(r::RRefinement)),
        "zig" => Some(Box::new(zig::ZigRefinement)),
        "css" => Some(Box::new(css::CssRefinement)),
        "graphql" => Some(Box::new(graphql::GraphQlRefinement)),
        "groovy" => Some(Box::new(groovy::GroovyRefinement)),
        "solidity" => Some(Box::new(solidity::SolidityRefinement)),
        "erlang" => Some(Box::new(erlang::ErlangRefinement)),
        "julia" => Some(Box::new(julia::JuliaRefinement)),
        "cmake" => Some(Box::new(cmake::CMakeRefinement)),
        "make" => Some(Box::new(make::MakeRefinement)),
        _ => None,
    }
}
